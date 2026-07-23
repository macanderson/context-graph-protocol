// Package contextgraph is a zero-dependency Go SDK for building conformant
// Context Graph Protocol providers. The wire types below mirror
// schema/contextgraph-envelope.schema.json; `omitempty` matches the reference
// serializer, which omits an absent optional rather than emitting null.
package contextgraph

// ProtocolVersion is the protocol version this SDK speaks.
const ProtocolVersion = "contextgraph/1.0-draft"

// Provenance is one link in a frame's provenance chain. Type is the wire name
// for the entry's kind ("file", "derivation", ...).
type Provenance struct {
	Type   string `json:"type"`
	URI    string `json:"uri,omitempty"`
	Range  string `json:"range,omitempty"`
	Digest string `json:"digest,omitempty"`
	Method string `json:"method,omitempty"`
	By     string `json:"by,omitempty"`
}

// Relation is a graph edge a frame participates in, surfaced by DisplayName.
type Relation struct {
	Rel         string `json:"rel"`
	TargetURI   string `json:"target_uri"`
	DisplayName string `json:"display_name,omitempty"`
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
	ID                       string       `json:"id"`
	Kind                     string       `json:"kind"`
	Title                    string       `json:"title"`
	Content                  string       `json:"content,omitempty"` // absent for reference frames
	ContentDigest            string       `json:"content_digest,omitempty"`
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
	Goal                      string   `json:"goal"`
	QueryText                 string   `json:"query_text,omitempty"`
	Kinds                     []string `json:"kinds,omitempty"`
	Anchors                   []string `json:"anchors,omitempty"`
	MaxFrames                 uint32   `json:"max_frames"`
	MaxTokens                 uint32   `json:"max_tokens"`
	AsOf                      string   `json:"as_of,omitempty"`
	RepresentationPreferences []string `json:"representation_preferences,omitempty"`
}

// ContextQueryResult is the response to a query.
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

// VerifyResponse is a provider's answer to a VerifyRequest.
type VerifyResponse struct {
	Verdicts []FrameVerdict `json:"verdicts"`
}
