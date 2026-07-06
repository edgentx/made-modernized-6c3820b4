/**
 * Request/response DTOs for the `/v1` REST surface.
 *
 * These mirror the backend domain aggregates in `crates/domain` (CardDefinition,
 * PlayerCollection, ExpansionSet, Order, RankedStanding/Season) expressed in
 * idiomatic camelCase TypeScript. Because the REST layer is the contract between
 * the PWA and the edge, these interfaces are the client-side source of truth for
 * those payloads; keep them in step with the domain structs as endpoints land.
 *
 * Enums are modelled as string unions rather than TS `enum`s so they erase at
 * runtime (no emitted objects) and compare directly against JSON string values.
 */

// ── Catalog: card definitions & expansion sets ────────────────────────────────

/** Mirrors `card_definition::CardType`. */
export type CardType = 'unit' | 'spell' | 'trap' | 'leader'
/** Mirrors `card_definition::CardClass`. */
export type CardClass = 'neutral' | 'aggression' | 'control' | 'tempo' | 'combo'
/** Mirrors `card_definition::Rarity`. */
export type Rarity = 'common' | 'uncommon' | 'rare' | 'epic' | 'legendary'

/** A published card definition (mirrors `CardDefined`). */
export interface Card {
  readonly cardId: string
  readonly name: string
  readonly cost: number
  readonly cardClass: CardClass
  readonly cardType: CardType
  readonly rarity: Rarity
  readonly keywords: readonly string[]
  readonly effectScriptRef: string
  /** Max copies of this card allowed in a single deck. */
  readonly copyCap: number
}

/** Release channel of an expansion set (mirrors `ExpansionReleased.release_channel`). */
export type ReleaseChannel = 'alpha' | 'beta' | 'live'

/** A card expansion / set (mirrors `expansion_set` aggregate). */
export interface ExpansionSet {
  readonly setCode: string
  readonly name: string
  readonly releaseChannel: ReleaseChannel
  readonly cardIds: readonly string[]
}

// ── Collection & deck ─────────────────────────────────────────────────────────

/** A single owned card row: the definition plus per-player state. */
export interface OwnedCard {
  readonly cardId: string
  /** How many copies the player owns. */
  readonly quantity: number
  /** Equipped cosmetic skin ref, if any (mirrors `CosmeticEquipped`). */
  readonly cosmeticSkinRef: string | null
}

/** A saved deck of card references. */
export interface Deck {
  readonly deckId: string
  readonly name: string
  /** Ordered card ids composing the deck (duplicates allowed up to copyCap). */
  readonly cardIds: readonly string[]
  /** Whether this is the player's active deck. */
  readonly active: boolean
}

/** Response of `GET /v1/collection/{playerId}`. */
export interface CollectionResponse {
  readonly playerId: string
  readonly ownedCards: readonly OwnedCard[]
  readonly decks: readonly Deck[]
}

/** Body of `PUT /v1/collection/{playerId}/decks/{deckId}`. */
export interface SaveDeckRequest {
  readonly name: string
  readonly cardIds: readonly string[]
  readonly active?: boolean
}

// ── Leaderboard ───────────────────────────────────────────────────────────────

/** One ranked row (mirrors a `RankedStanding` projection / `Season` leaderboard). */
export interface LeaderboardEntry {
  readonly rank: number
  readonly playerId: string
  readonly displayName: string
  readonly rating: number
  readonly stars: number
}

/** Response of `GET /v1/leaderboard` — a page of ranked standings. */
export interface LeaderboardPage {
  readonly seasonId: string
  readonly entries: readonly LeaderboardEntry[]
  /** Total ranked players in the season (for pagination UIs). */
  readonly total: number
  readonly page: number
  readonly pageSize: number
}

/** Query params for `GET /v1/leaderboard`. */
export interface LeaderboardQuery {
  readonly seasonId?: string
  readonly page?: number
  readonly pageSize?: number
}

// ── Shop & orders ─────────────────────────────────────────────────────────────

/** A purchasable shop item (pack, cosmetic, expansion). */
export interface ShopItem {
  readonly sku: string
  readonly name: string
  readonly description: string
  /** Price in minor currency units (e.g. cents), to avoid float money. */
  readonly priceMinor: number
  readonly currency: string
}

/** Lifecycle state of an order (mirrors the `order` aggregate transitions). */
export type OrderStatus = 'created' | 'paid' | 'fulfilled' | 'refunded'

/** An order and its current state (mirrors `Order`). */
export interface Order {
  readonly orderId: string
  readonly playerId: string
  readonly lineItems: readonly string[]
  readonly currency: string
  readonly status: OrderStatus
}

/** Body of `POST /v1/shop/orders` (mirrors `CreateOrderCmd`). */
export interface CreateOrderRequest {
  readonly playerId: string
  readonly lineItems: readonly string[]
  readonly currency: string
}
