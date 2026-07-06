/**
 * Deck (Outfit) legality — a faithful TypeScript mirror of the authoritative
 * `crates/domain/src/outfit.rs` aggregate, used to validate a deck client-side
 * *before* it is saved through the collection-deck-service.
 *
 * The Rust `Outfit` aggregate re-checks four invariants on every mutating
 * command and again on `ValidateOutfitCmd` / `SaveOutfitCmd`:
 *
 *  1. **Exactly 30 cards** — an Outfit holds exactly {@link LEGAL_OUTFIT_SIZE}
 *     cards to be legal for saving/play.
 *  2. **Own class or Neutral only** — an Outfit may include only cards of its
 *     own class plus Neutral cards.
 *  3. **Copy caps** — at most 2 copies of any card, and at most 1 for a
 *     Legendary. The per-card cap is carried on {@link Card.copyCap}; a
 *     Legendary is additionally clamped to {@link LEGENDARY_COPY_CAP}.
 *  4. **Owned at validation time** — every card in the Outfit must be owned in
 *     the player's collection, and no more copies than are owned.
 *
 * Like {@link module:match/rules}, this is a *pure* module (no DOM, no network):
 * every function takes plain data and returns a decision, so it is unit-testable
 * and can back both the live builder gate and any future practice tooling. It is
 * a client-side *pre-check* — the server's `SaveOutfitCmd` remains the
 * authority and re-runs the identical invariants — so a deck that passes here
 * can still be rejected by the edge, and a save is always defended in depth.
 */
import type { Card, CardClass } from '../api/types'

/** The number of cards an Outfit must hold, exactly, to be legal. */
export const LEGAL_OUTFIT_SIZE = 30

/** Default copy cap for a non-Legendary card (mirrors the domain's "at most 2"). */
export const DEFAULT_COPY_CAP = 2

/** Copy cap for a Legendary card (mirrors the domain's "1 copy for a Legendary"). */
export const LEGENDARY_COPY_CAP = 1

/**
 * The class a deck is built around. Mirrors the non-Neutral {@link CardClass}
 * values: an Outfit's own class is always one of these, and `neutral` cards are
 * legal in any deck (invariant #2).
 */
export type DeckClass = Exclude<CardClass, 'neutral'>

/** The four non-Neutral classes a deck may be built around. */
export const DECK_CLASSES: readonly DeckClass[] = ['aggression', 'control', 'tempo', 'combo']

/** A machine-readable classification of a single legality violation. */
export type LegalityCode =
  /** The deck does not hold exactly {@link LEGAL_OUTFIT_SIZE} cards. */
  | 'size'
  /** The deck includes a card outside its own class (and not Neutral). */
  | 'class'
  /** The deck exceeds a card's copy cap. */
  | 'copies'
  /** The deck includes more copies of a card than the player owns. */
  | 'ownership'
  /** A card id in the deck is not present in the catalog (cannot be judged). */
  | 'unknown-card'

/** One legality violation, ready to surface as inline messaging in the UI. */
export interface LegalityIssue {
  readonly code: LegalityCode
  readonly message: string
  /** The offending card, when the issue is card-specific (not the size rule). */
  readonly cardId?: string
}

/** The outcome of checking a deck: legal, or the specific reasons it is not. */
export interface LegalityReport {
  readonly legal: boolean
  readonly issues: readonly LegalityIssue[]
  /** Current card count (so the UI can render "27 / 30" without recomputing). */
  readonly size: number
}

/** The catalog + collection facts {@link checkLegality} needs to judge a deck. */
export interface LegalityContext {
  /** The class the deck is built around. */
  readonly deckClass: DeckClass
  /** Card definitions, keyed by `cardId` (the catalog). */
  readonly cards: ReadonlyMap<string, Card>
  /** How many copies of each card the player owns, keyed by `cardId`. */
  readonly owned: ReadonlyMap<string, number>
}

/**
 * The copy cap for `card`: its per-card {@link Card.copyCap}, additionally
 * clamped to {@link LEGENDARY_COPY_CAP} for a Legendary. Mirrors the domain's
 * "at most 2 copies of any card (1 copy for a Legendary)".
 */
export function copyCapFor(card: Card): number {
  const rarityCap = card.rarity === 'legendary' ? LEGENDARY_COPY_CAP : DEFAULT_COPY_CAP
  return Math.min(card.copyCap, rarityCap)
}

/** Tally a deck's card ids into a `cardId → count` map, preserving first-seen order. */
export function countCards(cardIds: readonly string[]): Map<string, number> {
  const counts = new Map<string, number>()
  for (const id of cardIds) {
    counts.set(id, (counts.get(id) ?? 0) + 1)
  }
  return counts
}

/**
 * Whether `card` is legal in a deck of `deckClass`: it must be of the deck's own
 * class or Neutral (invariant #2).
 */
export function isClassLegal(card: Card, deckClass: DeckClass): boolean {
  return card.cardClass === 'neutral' || card.cardClass === deckClass
}

/**
 * Check a deck's `cardIds` against the four Outfit invariants, returning every
 * violation (not just the first) so the builder can surface *all* the reasons a
 * deck is illegal at once. An empty {@link LegalityReport.issues} means the deck
 * is legal and safe to save.
 *
 * The size rule is reported once; the class/copy/ownership rules are reported
 * once per offending card so each row in the deck can carry its own message.
 */
export function checkLegality(cardIds: readonly string[], ctx: LegalityContext): LegalityReport {
  const issues: LegalityIssue[] = []
  const counts = countCards(cardIds)

  // Invariant #1 — exactly 30 cards.
  if (cardIds.length !== LEGAL_OUTFIT_SIZE) {
    const over = cardIds.length > LEGAL_OUTFIT_SIZE
    issues.push({
      code: 'size',
      message: `A deck must hold exactly ${LEGAL_OUTFIT_SIZE} cards — you have ${cardIds.length} (${
        over ? 'remove' : 'add'
      } ${Math.abs(cardIds.length - LEGAL_OUTFIT_SIZE)}).`,
    })
  }

  // Per-card invariants #2–#4, judged once per distinct card.
  for (const [cardId, count] of counts) {
    const card = ctx.cards.get(cardId)
    if (!card) {
      issues.push({
        code: 'unknown-card',
        cardId,
        message: `Card "${cardId}" is not in the catalog and cannot be validated.`,
      })
      continue
    }

    // #2 — own class or Neutral only.
    if (!isClassLegal(card, ctx.deckClass)) {
      issues.push({
        code: 'class',
        cardId,
        message: `"${card.name}" is a ${card.cardClass} card and cannot go in a ${ctx.deckClass} deck.`,
      })
    }

    // #3 — copy caps (Legendary = 1, otherwise the card's cap).
    const cap = copyCapFor(card)
    if (count > cap) {
      const legendary = card.rarity === 'legendary'
      issues.push({
        code: 'copies',
        cardId,
        message: `"${card.name}" is capped at ${cap} cop${cap === 1 ? 'y' : 'ies'}${
          legendary ? ' (Legendary)' : ''
        } — you have ${count}.`,
      })
    }

    // #4 — owned at validation time (and no more copies than owned).
    const ownedQty = ctx.owned.get(cardId) ?? 0
    if (count > ownedQty) {
      issues.push({
        code: 'ownership',
        cardId,
        message:
          ownedQty === 0
            ? `You do not own "${card.name}".`
            : `You own ${ownedQty} cop${ownedQty === 1 ? 'y' : 'ies'} of "${card.name}" but the deck uses ${count}.`,
      })
    }
  }

  return { legal: issues.length === 0, issues, size: cardIds.length }
}

/**
 * Infer the class a saved deck was built around — the single non-Neutral class
 * its cards share — so an existing deck (whose REST DTO carries no class) can be
 * reopened in the builder with its class preselected. Returns `null` when the
 * deck is empty/all-Neutral or mixes classes (nothing unambiguous to infer).
 */
export function inferDeckClass(
  cardIds: readonly string[],
  cards: ReadonlyMap<string, Card>,
): DeckClass | null {
  let found: DeckClass | null = null
  for (const id of cardIds) {
    const cls = cards.get(id)?.cardClass
    if (!cls || cls === 'neutral') continue
    if (found && found !== cls) return null // mixed classes — ambiguous
    found = cls
  }
  return found
}
