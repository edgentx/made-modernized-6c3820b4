/**
 * Deck-legality tests.
 *
 * These pin the four invariants the browser mirrors from
 * `crates/domain/src/outfit.rs`: exactly 30 cards, own-class-or-Neutral only,
 * copy caps (Legendary = 1), and owned-at-validation time. They are plain TS
 * against the pure {@link checkLegality} (no DOM), matching the match/rules and
 * API-client test style.
 */
import { describe, expect, it } from 'vitest'
import {
  checkLegality,
  copyCapFor,
  inferDeckClass,
  LEGAL_OUTFIT_SIZE,
  type DeckClass,
  type LegalityContext,
} from './legality'
import type { Card, CardClass, Rarity } from '../api/types'

function card(id: string, over: Partial<Card> = {}): Card {
  return {
    cardId: id,
    name: over.name ?? id,
    cost: over.cost ?? 3,
    cardClass: over.cardClass ?? 'aggression',
    cardType: over.cardType ?? 'unit',
    rarity: over.rarity ?? 'common',
    keywords: over.keywords ?? [],
    effectScriptRef: over.effectScriptRef ?? '',
    copyCap: over.copyCap ?? 2,
  }
}

/**
 * A legal 30-card `aggression` deck: 15 distinct commons, 2 copies each, all
 * owned in quantity 2. Tests mutate one aspect at a time to drive a rejection.
 */
interface Fixture {
  cardIds: string[]
  cards: Map<string, Card>
  owned: Map<string, number>
  ctx: LegalityContext
}

function legalDeck(): Fixture {
  const cards = new Map<string, Card>()
  const owned = new Map<string, number>()
  const cardIds: string[] = []
  for (let i = 0; i < 15; i++) {
    const id = `c${i}`
    cards.set(id, card(id))
    owned.set(id, 2)
    cardIds.push(id, id)
  }
  // `ctx` holds the same Map references, so tests can mutate `cards`/`owned`.
  return { cardIds, cards, owned, ctx: { deckClass: 'aggression', cards, owned } }
}

describe('checkLegality', () => {
  it('accepts a legal 30-card deck', () => {
    const { cardIds, ctx } = legalDeck()
    const report = checkLegality(cardIds, ctx)
    expect(report.legal).toBe(true)
    expect(report.issues).toEqual([])
    expect(report.size).toBe(LEGAL_OUTFIT_SIZE)
  })

  it('rejects a deck that is not exactly 30 cards', () => {
    const { cardIds, ctx } = legalDeck()
    const report = checkLegality(cardIds.slice(0, 29), ctx)
    expect(report.legal).toBe(false)
    expect(report.issues.some((i) => i.code === 'size')).toBe(true)
  })

  it('rejects a foreign-class card (own class or Neutral only)', () => {
    const { cardIds, cards, ctx } = legalDeck()
    cards.set('c0', card('c0', { cardClass: 'control' }))
    const report = checkLegality(cardIds, ctx)
    expect(report.legal).toBe(false)
    expect(report.issues.some((i) => i.code === 'class' && i.cardId === 'c0')).toBe(true)
  })

  it('allows Neutral cards in any deck', () => {
    const { cardIds, cards, ctx } = legalDeck()
    cards.set('c0', card('c0', { cardClass: 'neutral' }))
    const report = checkLegality(cardIds, ctx)
    expect(report.legal).toBe(true)
  })

  it('rejects exceeding a non-Legendary copy cap (at most 2)', () => {
    const { owned, ctx } = legalDeck()
    // 26 cards from 13 distinct commons + 4 copies of one card = 30; the copy
    // violation (4 > cap of 2) is the point.
    const cardIds: string[] = []
    for (let i = 1; i < 14; i++) cardIds.push(`c${i}`, `c${i}`) // 26
    cardIds.push('c0', 'c0', 'c0', 'c0') // 30 total, 4 copies of c0
    owned.set('c0', 4)
    const report = checkLegality(cardIds, ctx)
    expect(report.legal).toBe(false)
    expect(report.issues.some((i) => i.code === 'copies' && i.cardId === 'c0')).toBe(true)
  })

  it('rejects a second copy of a Legendary (copy cap 1)', () => {
    const { cards, owned, ctx } = legalDeck()
    cards.set('c0', card('c0', { rarity: 'legendary', copyCap: 2 }))
    owned.set('c0', 2)
    const cardIds: string[] = []
    for (let i = 1; i < 15; i++) cardIds.push(`c${i}`, `c${i}`) // 28
    cardIds.push('c0', 'c0') // 30 total, 2 copies of a Legendary
    const report = checkLegality(cardIds, ctx)
    expect(report.legal).toBe(false)
    expect(report.issues.some((i) => i.code === 'copies' && i.cardId === 'c0')).toBe(true)
  })

  it('rejects more copies than the player owns', () => {
    const { cardIds, owned, ctx } = legalDeck()
    owned.set('c0', 1) // deck uses 2, owns 1
    const report = checkLegality(cardIds, ctx)
    expect(report.legal).toBe(false)
    expect(report.issues.some((i) => i.code === 'ownership' && i.cardId === 'c0')).toBe(true)
  })

  it('rejects a card missing from the catalog', () => {
    const { cardIds, cards, ctx } = legalDeck()
    cards.delete('c0')
    const report = checkLegality(cardIds, ctx)
    expect(report.legal).toBe(false)
    expect(report.issues.some((i) => i.code === 'unknown-card' && i.cardId === 'c0')).toBe(true)
  })

  it('reports every violation at once, not just the first', () => {
    const { cardIds, cards, owned, ctx } = legalDeck()
    cards.set('c0', card('c0', { cardClass: 'control' })) // class issue
    owned.set('c1', 0) // ownership issue
    const report = checkLegality(cardIds.slice(0, 28), ctx) // + size issue
    const codes = new Set(report.issues.map((i) => i.code))
    expect(codes.has('size')).toBe(true)
    expect(codes.has('class')).toBe(true)
    expect(codes.has('ownership')).toBe(true)
  })
})

describe('copyCapFor', () => {
  it('clamps a Legendary to a single copy', () => {
    expect(copyCapFor(card('x', { rarity: 'legendary', copyCap: 2 }))).toBe(1)
  })

  it('honours a per-card cap below the default', () => {
    expect(copyCapFor(card('x', { rarity: 'common', copyCap: 1 }))).toBe(1)
  })

  it('caps a common at 2 by default', () => {
    expect(copyCapFor(card('x', { rarity: 'common', copyCap: 2 }))).toBe(2)
  })
})

describe('inferDeckClass', () => {
  const cards = new Map<string, Card>([
    ['n', card('n', { cardClass: 'neutral' })],
    ['a', card('a', { cardClass: 'aggression' })],
    ['c', card('c', { cardClass: 'control' })],
  ])

  it('infers the single non-Neutral class present', () => {
    expect(inferDeckClass(['n', 'a', 'a'], cards)).toBe<DeckClass>('aggression')
  })

  it('returns null for an all-Neutral or empty deck', () => {
    expect(inferDeckClass(['n', 'n'], cards)).toBeNull()
    expect(inferDeckClass([], cards)).toBeNull()
  })

  it('returns null when classes are mixed (ambiguous)', () => {
    expect(inferDeckClass(['a', 'c'], cards)).toBeNull()
  })
})

// A compile-time nudge that the exported unions stay in step with the DTO.
const _classes: readonly CardClass[] = ['neutral', 'aggression', 'control', 'tempo', 'combo']
const _rarities: readonly Rarity[] = ['common', 'uncommon', 'rare', 'epic', 'legendary']
void _classes
void _rarities
