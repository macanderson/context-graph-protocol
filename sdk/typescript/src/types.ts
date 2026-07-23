/**
 * Context Graph Protocol wire types — the language-neutral contract, mirrored
 * from `schema/contextgraph-envelope.schema.json` and `SPEC.md`.
 *
 * These are plain interfaces so a provider author works with ordinary objects;
 * the {@link Envelope} union is the one shape that crosses the wire (one JSON
 * object per line over stdio).
 */

/** The protocol version this SDK speaks. */
export const PROTOCOL_VERSION = "contextgraph/1.0-draft" as const;

/** What kind of thing a frame represents. */
export type FrameKind =
  | "snippet"
  | "symbol"
  | "fact"
  | "doc"
  | "memory"
  | "episode"
  | "graph";

/** How a frame carries its content. Absent on the wire ⇒ `full`. */
export type Representation = "full" | "compact" | "reference";

/** Fidelity of carried content relative to the source. */
export type ContentFidelity = "exact" | "normalized" | "summarized" | "omitted";

/** Whether a frame's point of use requires inline content. */
export type InlineContentRequirement = "required" | "resolvable_reference_allowed";

/** One link in a frame's provenance chain. `type` is the wire name for kind. */
export interface Provenance {
  type: string;
  uri?: string;
  range?: string;
  digest?: string;
  method?: string;
  by?: string;
}

/** A graph edge a frame participates in, surfaced by a human `display_name`. */
export interface Relation {
  rel: string;
  target_uri: string;
  display_name?: string;
}

export interface FrameEmbedding {
  fingerprint: string;
  vector?: number[];
}

/** Opaque resolver handle for a `compact`/`reference` frame's content. */
export interface ContentRef {
  provider_id: string;
  uri: string;
  expires_at?: string;
}

/** The transformation a `compact` frame applied to its source. */
export interface Transform {
  method: string;
  implementation: string;
  version: string;
}

/** One context frame returned from `context/query`. */
export interface ContextFrame {
  id: string;
  kind: FrameKind;
  title: string;
  /** Present for `full`/`compact`; absent for `reference` frames. */
  content?: string;
  /** Hash of the inline content bytes (the spec's `content_hash`); feeds FrameId. */
  content_digest?: string;
  uri?: string;
  representation?: Representation;
  content_fidelity?: ContentFidelity;
  /** Hash of the complete source content (distinct from `content_digest`). */
  canonical_content_hash?: string;
  content_ref?: ContentRef;
  transform?: Transform;
  minimum_content_fidelity?: ContentFidelity;
  inline_content_requirement?: InlineContentRequirement;
  /** Provider-normalized relevance in `[0, 1]`. */
  score: number;
  /** Honest inline token cost — MUST equal `ceil(utf8_byte_length(content)/4)` (B3). */
  token_cost: number;
  canonical_token_cost?: number;
  tokenizer_ref?: string;
  valid_from?: string;
  valid_to?: string;
  recorded_at?: string;
  provenance?: Provenance[];
  citation_label?: string;
  embedding?: FrameEmbedding;
  relations?: Relation[];
}

/** A request to a provider for context frames relevant to a goal. */
export interface ContextQuery {
  goal: string;
  query_text?: string;
  embedding?: number[];
  kinds?: FrameKind[];
  anchors?: string[];
  max_frames: number;
  max_tokens: number;
  as_of?: string;
  representation_preferences?: Representation[];
}

export interface ContextQueryResult {
  frames: ContextFrame[];
  truncated: boolean;
  dropped_estimate?: number;
}

export interface QueryCapability {
  kinds?: string[];
}

/** What a provider can do, negotiated at handshake time. */
export interface Capabilities {
  query: QueryCapability;
  /** Provider echoes a request's `id` on its reply, enabling pipelining (H4). */
  correlation?: boolean;
  graph?: boolean;
  embeddings_fingerprint?: string | null;
  /** Whether the provider answers `context/verify`. */
  verify?: boolean;
  representations?: Representation[];
  resolve?: boolean;
}

/** An egress scope (open vocabulary), e.g. `local-only`, `third-party-model`. */
export type EgressScope = string;

/** Declares what a provider does with data, so a host can gate consent. */
export interface DataFlow {
  reads: boolean;
  writes: boolean;
  egress: boolean;
  egress_scopes?: EgressScope[];
}

export interface ProviderInfo {
  name: string;
  version: string;
  data_flow: DataFlow;
}

/** The stable identity of one frame's exact content bytes. */
export interface FrameId {
  provider_id: string;
  frame_id: string;
  content_digest?: string | null;
}

export type VerdictStatus = "valid" | "stale" | "gone" | "unknown";

export interface FrameVerdict {
  frame: FrameId;
  status: VerdictStatus;
  /** Only meaningful with `stale`: the provider's current digest for the frame. */
  replacement_digest?: string | null;
}

export interface VerifyRequest {
  frames: FrameId[];
}

export interface VerifyResponse {
  verdicts: FrameVerdict[];
}

/** Machine-readable error classification (open vocabulary). */
export type ErrorCode = string;

/**
 * The one shape that crosses the wire: an internally-tagged union selected by
 * `type`. One JSON object per line over stdio.
 */
export type Envelope =
  | { type: "handshake"; protocol_version: string }
  | {
      type: "handshake_ack";
      protocol_version: string;
      provider: ProviderInfo;
      capabilities: Capabilities;
    }
  | { type: "query"; query: ContextQuery; id?: string }
  | { type: "frames"; result: ContextQueryResult; id?: string }
  | { type: "verify"; request: VerifyRequest }
  | { type: "verified"; response: VerifyResponse }
  | { type: "shutdown" }
  | { type: "error"; message: string; id?: string; code?: ErrorCode };
