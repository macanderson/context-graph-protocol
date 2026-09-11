// Package contextgraph is a zero-dependency Go SDK for building conformant
// Context Graph Protocol providers. The wire types below mirror
// schema/contextgraph-envelope.schema.json; `omitempty` matches the reference
// serializer, which omits an absent optional rather than emitting null.
//
// # Why optional strings are pointers
//
// An optional field whose empty value is *distinguishable from absence* is
// declared *string, not string. Go's encoding/json decodes an absent member
// and an explicit "" into the same string, and `omitempty` drops an empty one
// on the way out, so a plain string silently conflates two states the protocol
// keeps apart:
//
//   - The five optional fields of [Provenance] and [ContextFrame.ContentDigest]
//     are enc_opt inputs to the attestation preimage (SPEC.md §6.5.1, §6.5.2).
//     The presence byte there is normative — enc_opt(None) is 0x00 and
//     enc_opt(Some(s)) is 0x01 ‖ enc_str(s) — precisely so a link's URI cannot
//     be deleted from a signed chain without disturbing the hash. Conflating
//     them left a Go verifier computing a chain head the signer never did, and
//     reporting commitment_mismatch on an honest frame.
//   - [ContextFrame.Content] is absent for a reference frame and present for a
//     full frame, which the schema enforces per representation. A full frame
//     carrying an empty document must still emit `"content": ""`; omitting the
//     member reshapes it into a frame the schema rejects.
//
// Every other optional string here is either an enum member or carries a
// schema `minLength`/`pattern` that makes "" invalid on the wire, so absent and
// empty are not two states a peer can distinguish. Those stay plain strings.
//
// A nil pointer is omitted by `omitempty`; a pointer to "" serializes as "".
// Use [Ptr] to build one.
package contextgraph

// ProtocolVersion is the protocol version this SDK speaks.
const ProtocolVersion = "contextgraph/1.0"

// Ptr returns a pointer to v.
//
// Go has no literal for the address of a constant, and the optional fields of
// these wire types are pointers so absence stays distinct from emptiness:
//
//	cg.Provenance{Type: "file", URI: cg.Ptr("file:///docs/start.md")}
//
// Generic, so the same call also builds the *uint32 that
// [ContextFrame.CanonicalTokenCost] takes.
func Ptr[T any](v T) *T { return &v }

// Provenance is one link in a frame's provenance chain. Type is the wire name
// for the entry's kind ("file", "derivation", ...).
//
// The five optional fields are pointers because they are enc_opt inputs to the
// normative §6.5.1 link encoding: `"uri": null` and `"uri": ""` MUST hash
// differently, so this type has to be able to hold both. Convert to the
// encoding's link type with attest.LinkFromProvenance, which now copies the
// presence through rather than collapsing it.
type Provenance struct {
	Type   string  `json:"type"`
	URI    *string `json:"uri,omitempty"`
	Range  *string `json:"range,omitempty"`
	Digest *string `json:"digest,omitempty"`
	Method *string `json:"method,omitempty"`
	By     *string `json:"by,omitempty"`
}

// Relation is a graph edge a frame participates in, surfaced by DisplayName.
type Relation struct {
	Rel         string `json:"rel"`
	TargetURI   string `json:"target_uri"`
	DisplayName string `json:"display_name,omitempty"`
}

// FrameEmbedding is a frame's vector, in the embedding space named by
// Fingerprint. The vector payload itself is elidable — a provider may name the
// space it embedded in without shipping the numbers — so Fingerprint is the
// required half and Vector the optional one, exactly as
// schema/contextgraph-envelope.schema.json declares and as the Rust
// (Option<Vec<f32>>), TypeScript (vector?: number[]) and Python SDKs expose it.
type FrameEmbedding struct {
	Fingerprint string    `json:"fingerprint"`
	Vector      []float64 `json:"vector,omitempty"`
}

// ContentRef is an opaque resolver handle for a compact/reference frame.
type ContentRef struct {
	ProviderID string `json:"provider_id"`
	URI        string `json:"uri"`
	ExpiresAt  string `json:"expires_at,omitempty"`
}

// Transform is the transformation a compact frame applied to its source.
type Transform struct {
	Method         string `json:"method"`
	Implementation string `json:"implementation"`
	Version        string `json:"version"`
}

// ContextFrame is one context frame returned from context/query.
type ContextFrame struct {
	ID    string `json:"id"`
	Kind  string `json:"kind"`
	Title string `json:"title"`
	// Content is absent for a reference frame and present for every other
	// representation — including a full frame whose document is empty, which
	// must still emit `"content": ""` to satisfy the schema's full branch.
	Content *string `json:"content,omitempty"`
	// ContentDigest is an enc_opt input to the §6.5.2 frame commitment, so
	// absent and present-but-empty must stay distinct here (see the package
	// doc). Absent ⇒ the frame is not verifiable and a host re-queries it.
	ContentDigest            *string      `json:"content_digest,omitempty"`
	URI                      string       `json:"uri,omitempty"`
	Representation           string       `json:"representation,omitempty"`
	ContentFidelity          string       `json:"content_fidelity,omitempty"`
	CanonicalContentHash     string       `json:"canonical_content_hash,omitempty"`
	ContentRef               *ContentRef  `json:"content_ref,omitempty"`
	Transform                *Transform   `json:"transform,omitempty"`
	MinimumContentFidelity   string       `json:"minimum_content_fidelity,omitempty"`
	InlineContentRequirement string       `json:"inline_content_requirement,omitempty"`
	Score                    float64      `json:"score"`
	TokenCost                uint32       `json:"token_cost"` // ceil(utf8_len(content)/4); required, never omitted
	CanonicalTokenCost       *uint32      `json:"canonical_token_cost,omitempty"`
	TokenizerRef             string       `json:"tokenizer_ref,omitempty"`
	ValidFrom                string       `json:"valid_from,omitempty"`
	ValidTo                  string       `json:"valid_to,omitempty"`
	RecordedAt               string       `json:"recorded_at,omitempty"`
	Provenance               []Provenance `json:"provenance,omitempty"`
	CitationLabel            string       `json:"citation_label,omitempty"`
	Relations                []Relation   `json:"relations,omitempty"`
}

// ContextQuery is a request to a provider for frames relevant to a goal.
type ContextQuery struct {
	Goal      string `json:"goal"`
	QueryText string `json:"query_text,omitempty"`
	// Embedding is the query vector, and its *presence* is load-bearing:
	// SPEC.md §E1 rejects a vector whose length contradicts the provider's
	// declared embeddings_fingerprint dimension, and an empty vector has length
	// 0, which contradicts every non-zero dimension. Use HasEmbedding to ask
	// whether the host sent one — encoding/json leaves this nil for an absent
	// or null `embedding` and gives it a non-nil zero-length slice for `[]`, so
	// nil corresponds exactly to Rust's None and a present-but-empty vector is
	// distinguishable from no vector at all. Testing len() alone silently
	// accepts `"embedding": []`, which the Rust, TypeScript and Python
	// implementations all reject.
	Embedding                 []float64 `json:"embedding,omitempty"`
	Kinds                     []string  `json:"kinds,omitempty"`
	Anchors                   []string  `json:"anchors,omitempty"`
	MaxFrames                 uint32    `json:"max_frames"`
	MaxTokens                 uint32    `json:"max_tokens"`
	AsOf                      string    `json:"as_of,omitempty"`
	RepresentationPreferences []string  `json:"representation_preferences,omitempty"`
}

// HasEmbedding reports whether the host sent an `embedding` field at all, as
// distinct from sending an empty one. It is the Go spelling of Rust's
// `query.embedding.is_some()`, TypeScript's `embedding !== undefined` and
// Python's `embedding is not None` — the guard a §E1 dimension check belongs
// behind, so an empty vector is rejected rather than waved through.
//
// (The `omitempty` on Embedding means a Go-built query re-encodes a
// present-but-empty vector as absent. That direction never runs in a provider,
// which only ever decodes queries, and dropping an empty vector loses no
// information a provider could have used.)
func (q ContextQuery) HasEmbedding() bool { return q.Embedding != nil }

// ContextQueryResult is the response to a query.
//
// Frames is required on the wire and must be an array even when empty:
// "I have nothing relevant" is a permitted answer, but `"frames": null` is a
// deserialization error at a conforming host (the reference host declares
// `Vec<ContextFrame>` with no serde default, and rejects null with
// "invalid type: null, expected a sequence"). Go marshals a nil slice to null,
// so the SDK normalizes a nil Frames to an empty slice on the way out — a
// provider may return the zero value and be understood.
type ContextQueryResult struct {
	Frames          []ContextFrame `json:"frames"`
	Truncated       bool           `json:"truncated"`
	DroppedEstimate *uint32        `json:"dropped_estimate,omitempty"`
}

// QueryCapability is the retrieval surface a provider offers.
type QueryCapability struct {
	Kinds []string `json:"kinds,omitempty"`
}

// Capabilities is what a provider can do, negotiated at handshake.
type Capabilities struct {
	Query                 QueryCapability `json:"query"`
	Correlation           bool            `json:"correlation,omitempty"`
	Graph                 bool            `json:"graph,omitempty"`
	EmbeddingsFingerprint *string         `json:"embeddings_fingerprint,omitempty"`
	Verify                bool            `json:"verify,omitempty"`
	Representations       []string        `json:"representations,omitempty"`
	Resolve               bool            `json:"resolve,omitempty"`
}

// DataFlow declares what a provider does with data, so a host can gate consent.
type DataFlow struct {
	Reads        bool     `json:"reads"`
	Writes       bool     `json:"writes"`
	Egress       bool     `json:"egress"`
	EgressScopes []string `json:"egress_scopes,omitempty"`
}

// ProviderInfo is provider identity reported at handshake.
type ProviderInfo struct {
	Name     string   `json:"name"`
	Version  string   `json:"version"`
	DataFlow DataFlow `json:"data_flow"`
}

// FrameID is the stable identity of one frame's exact content bytes.
type FrameID struct {
	ProviderID    string `json:"provider_id"`
	FrameID       string `json:"frame_id"`
	ContentDigest string `json:"content_digest,omitempty"`
}

// FrameVerdict is one frame's verify verdict, flattened onto its identity.
type FrameVerdict struct {
	Frame             FrameID `json:"frame"`
	Status            string  `json:"status"` // valid | stale | gone | unknown
	ReplacementDigest string  `json:"replacement_digest,omitempty"`
}

// VerifyRequest carries the frame identities a host asks a provider to revalidate.
type VerifyRequest struct {
	Frames []FrameID `json:"frames"`
}

// VerifyResponse is a provider's answer to a VerifyRequest. Like
// ContextQueryResult.Frames, Verdicts must be an array on the wire even when
// empty; the SDK normalizes a nil Verdicts to an empty slice before writing.
type VerifyResponse struct {
	Verdicts []FrameVerdict `json:"verdicts"`
}
