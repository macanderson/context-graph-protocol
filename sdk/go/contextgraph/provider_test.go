package contextgraph

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

// queryFunc is a Provider whose Query is whatever the test supplies. Info and
// Capabilities are the honest minimum, since no test here inspects them.
type queryFunc func(ContextQuery) (ContextQueryResult, error)

func (queryFunc) Info() ProviderInfo {
	return ProviderInfo{Name: "test-provider", Version: "0.0.0", DataFlow: DataFlow{Reads: true}}
}

func (queryFunc) Capabilities() Capabilities {
	return Capabilities{Query: QueryCapability{Kinds: []string{"doc"}}}
}

func (f queryFunc) Query(query ContextQuery) (ContextQueryResult, error) { return f(query) }

// verifyingProvider is a queryFunc that also answers context/verify with
// whatever the test supplies — including a nil Verdicts slice, which is the
// hazard withVerdicts exists to cover.
type verifyingProvider struct {
	queryFunc
	verify func(VerifyRequest) VerifyResponse
}

func (p verifyingProvider) Verify(request VerifyRequest) VerifyResponse { return p.verify(request) }

// reply drives one line through the stdio state machine and returns the exact
// bytes written back — the wire, not a re-marshalled approximation of it.
func reply(t *testing.T, provider Provider, line string) string {
	t.Helper()
	var out bytes.Buffer
	writer := bufio.NewWriter(&out)
	handleLine(provider, line, writer)
	if err := writer.Flush(); err != nil {
		t.Fatalf("flush: %v", err)
	}
	return strings.TrimSpace(out.String())
}

const queryLine = `{"type":"query","id":"q1","query":{"goal":"anything","max_frames":8,"max_tokens":2000}}`

// A provider that finds nothing must be able to say so. Go marshals a nil slice
// to `null`, and the reference host rejects `"frames": null` with
// "invalid type: null, expected a sequence" — so the one explicitly permitted
// answer would otherwise be the one answer a Go provider could not deliver.
func TestEmptyResultSerializesFramesAsArray(t *testing.T) {
	provider := queryFunc(func(ContextQuery) (ContextQueryResult, error) {
		return ContextQueryResult{}, nil // the zero value: Frames is nil
	})

	got := reply(t, provider, queryLine)

	if !strings.Contains(got, `"frames":[]`) {
		t.Fatalf("reply does not carry an empty frames array: %s", got)
	}
	if strings.Contains(got, `"frames":null`) {
		t.Fatalf("reply carries frames:null, which a conforming host rejects: %s", got)
	}
}

// The same hazard reached through a provider that builds its frames with `var`
// and never appends — the shape the issue names as the ordinary case.
func TestNeverAppendedFramesSerializeAsArray(t *testing.T) {
	provider := queryFunc(func(ContextQuery) (ContextQueryResult, error) {
		var frames []ContextFrame
		return ContextQueryResult{Frames: frames, Truncated: false}, nil
	})

	if got := reply(t, provider, queryLine); !strings.Contains(got, `"frames":[]`) {
		t.Fatalf("reply does not carry an empty frames array: %s", got)
	}
}

// A user-supplied Verifier returning a nil Verdicts slice is the verify-side
// twin. (The SDK's own no-verify fallback was always safe: it uses make().)
func TestNilVerdictsSerializeAsArray(t *testing.T) {
	provider := verifyingProvider{
		queryFunc: func(ContextQuery) (ContextQueryResult, error) { return ContextQueryResult{}, nil },
		verify:    func(VerifyRequest) VerifyResponse { return VerifyResponse{} },
	}

	got := reply(t, provider, `{"type":"verify","request":{"frames":[]}}`)

	if !strings.Contains(got, `"verdicts":[]`) {
		t.Fatalf("reply does not carry an empty verdicts array: %s", got)
	}
	if strings.Contains(got, `"verdicts":null`) {
		t.Fatalf("reply carries verdicts:null, which a conforming host rejects: %s", got)
	}
}

// errorSpellings is shared by the stdio and HTTP error-code tests: every way a
// provider can hand back a ProviderError must preserve its code. The pointer
// spelling is the one Go authors reach for, and the one errors.As silently
// missed when the SDK looked only for the value form.
var errorSpellings = []struct {
	name string
	err  error
	code string
}{
	{"value", ProviderError{Code: "bad_request", Message: "wrong dimensions"}, "bad_request"},
	{"pointer", &ProviderError{Code: "bad_request", Message: "wrong dimensions"}, "bad_request"},
	{"wrapped value", fmt.Errorf("scoring: %w", ProviderError{Code: "internal", Message: "boom"}), "internal"},
	{"wrapped pointer", fmt.Errorf("scoring: %w", &ProviderError{Code: "internal", Message: "boom"}), "internal"},
	{"plain error", fmt.Errorf("something went wrong"), ""},
}

func TestProviderErrorCodeSurvivesEverySpelling(t *testing.T) {
	for _, spelling := range errorSpellings {
		t.Run(spelling.name, func(t *testing.T) {
			provider := queryFunc(func(ContextQuery) (ContextQueryResult, error) {
				return ContextQueryResult{}, spelling.err
			})

			var got errorReply
			if err := json.Unmarshal([]byte(reply(t, provider, queryLine)), &got); err != nil {
				t.Fatalf("reply was not a JSON envelope: %v", err)
			}
			if got.Type != "error" {
				t.Fatalf("reply type = %q, want error", got.Type)
			}
			if got.Code != spelling.code {
				t.Fatalf("reply code = %q, want %q", got.Code, spelling.code)
			}
			if got.ID == nil || *got.ID != "q1" {
				t.Fatalf("reply did not echo the correlation id: %+v", got.ID)
			}
		})
	}
}

// A nil *ProviderError satisfies error but has no message to report; the code
// lookup must not dereference it. (Query itself would panic on Error(), so this
// pins providerErrorCode rather than the whole reply path.)
func TestProviderErrorCodeIgnoresNilPointer(t *testing.T) {
	var nilPointer *ProviderError
	if code, ok := providerErrorCode(error(nilPointer)); ok {
		t.Fatalf("nil *ProviderError reported code %q", code)
	}
}

// §E1 needs the frame-level vector the other three SDKs already carry: a Go
// provider could declare an embeddings fingerprint and never attach one.
func TestFrameEmbeddingRoundTrips(t *testing.T) {
	frame := ContextFrame{
		ID:    "frm_1",
		Kind:  "doc",
		Title: "Getting Started",
		// A pointer since #124: the attestation encoding needs an absent field
		// to be distinguishable from an empty one.
		Content:       ptrTo("hello"),
		Score:         0.5,
		TokenCost:     BudgetTokens("hello"),
		CitationLabel: "getting-started.md L1-40",
		Embedding: &FrameEmbedding{
			Fingerprint: "bge-small-en-v1.5/384/l2",
			Vector:      []float64{0.25, -0.5, 0.75},
		},
	}

	encoded, err := json.Marshal(frame)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if !strings.Contains(string(encoded), `"embedding":{"fingerprint":"bge-small-en-v1.5/384/l2","vector":[0.25,-0.5,0.75]}`) {
		t.Fatalf("embedding did not serialize in the schema's shape: %s", encoded)
	}

	var decoded ContextFrame
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if decoded.Embedding == nil {
		t.Fatal("embedding was lost in the round trip")
	}
	if decoded.Embedding.Fingerprint != frame.Embedding.Fingerprint {
		t.Fatalf("fingerprint = %q, want %q", decoded.Embedding.Fingerprint, frame.Embedding.Fingerprint)
	}
	if len(decoded.Embedding.Vector) != 3 || decoded.Embedding.Vector[1] != -0.5 {
		t.Fatalf("vector = %v, want [0.25 -0.5 0.75]", decoded.Embedding.Vector)
	}
}

// The vector is elidable: a provider may name the space without shipping the
// numbers, and a frame with no embedding must not emit the key at all.
func TestFrameEmbeddingVectorIsElidable(t *testing.T) {
	withoutVector, err := json.Marshal(ContextFrame{
		ID: "frm_1", Kind: "doc", Title: "t",
		Embedding: &FrameEmbedding{Fingerprint: "bge-small-en-v1.5/384/l2"},
	})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if strings.Contains(string(withoutVector), `"vector"`) {
		t.Fatalf("an absent vector was serialized: %s", withoutVector)
	}

	withoutEmbedding, err := json.Marshal(ContextFrame{ID: "frm_1", Kind: "doc", Title: "t"})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if strings.Contains(string(withoutEmbedding), `"embedding"`) {
		t.Fatalf("a frame with no embedding emitted the key: %s", withoutEmbedding)
	}
}

// §E1's predicate is presence, not length. Rust asks `embedding.is_some()`,
// TypeScript `embedding !== undefined`, Python `embedding is not None`; without
// HasEmbedding, Go could only ask `len(...) > 0`, which waves through the
// `"embedding": []` the other three reject.
func TestHasEmbeddingDistinguishesAbsentFromEmpty(t *testing.T) {
	cases := []struct {
		body    string
		present bool
		length  int
	}{
		{`{"goal":"g","max_frames":8,"max_tokens":2000}`, false, 0},
		{`{"goal":"g","embedding":null,"max_frames":8,"max_tokens":2000}`, false, 0},
		{`{"goal":"g","embedding":[],"max_frames":8,"max_tokens":2000}`, true, 0},
		{`{"goal":"g","embedding":[0.1,0.2],"max_frames":8,"max_tokens":2000}`, true, 2},
	}
	for _, testCase := range cases {
		var query ContextQuery
		if err := json.Unmarshal([]byte(testCase.body), &query); err != nil {
			t.Fatalf("unmarshal %s: %v", testCase.body, err)
		}
		if got := query.HasEmbedding(); got != testCase.present {
			t.Fatalf("%s: HasEmbedding = %v, want %v", testCase.body, got, testCase.present)
		}
		if got := len(query.Embedding); got != testCase.length {
			t.Fatalf("%s: len(Embedding) = %d, want %d", testCase.body, got, testCase.length)
		}
	}
}

// ptrTo is the one-liner the pointer-valued optional fields need in tests.
// Those fields are pointers so an absent value and an empty one stay
// distinguishable, which the §6.5.2 frame commitment's enc_opt requires (#124).
func ptrTo[T any](v T) *T { return &v }
