package main

import (
	"encoding/json"
	"errors"
	"testing"

	cg "github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"
)

// decodeQuery builds a ContextQuery the way a host does — from wire bytes — so
// an absent `embedding` and a present-but-empty one are as distinguishable in
// the test as they are in production.
func decodeQuery(t *testing.T, body string) cg.ContextQuery {
	t.Helper()
	var query cg.ContextQuery
	if err := json.Unmarshal([]byte(body), &query); err != nil {
		t.Fatalf("unmarshal %s: %v", body, err)
	}
	return query
}

// §Q1: a non-empty kinds is a filter, not a hint. This provider declares `doc`
// and `snippet` and currently serves only `doc`, so a snippet-narrowed query
// must come back with zero frames rather than with documents the host excluded.
func TestQueryHonoursEveryDeclaredKind(t *testing.T) {
	declared := exampleDocsProvider{}.Capabilities().Query.Kinds
	if len(declared) < 2 {
		t.Fatalf("provider declares %v; the §Q1 regression needs more than one kind", declared)
	}

	for _, kind := range declared {
		result, err := exampleDocsProvider{}.Query(decodeQuery(t,
			`{"goal":"anything","kinds":["`+kind+`"],"max_frames":8,"max_tokens":2000}`))
		if err != nil {
			t.Fatalf("kinds=[%s]: %v", kind, err)
		}
		for _, frame := range result.Frames {
			if frame.Kind != kind {
				t.Fatalf("kinds=[%s] returned frame %s of kind %s (§Q1)", kind, frame.ID, frame.Kind)
			}
		}
	}
}

// The empty answer, specifically: `snippet` is declared but unserved, and
// answering it honestly is what exercises the nil-Frames normalization in the
// conformance run.
func TestQueryReturnsNoFramesForAnUnservedKind(t *testing.T) {
	result, err := exampleDocsProvider{}.Query(decodeQuery(t,
		`{"goal":"anything","kinds":["snippet"],"max_frames":8,"max_tokens":2000}`))
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(result.Frames) != 0 {
		t.Fatalf("kinds=[snippet] returned %d frame(s); this provider serves no snippets", len(result.Frames))
	}
	// Filtering is not truncation: the host excluded these itself.
	if result.Truncated || result.DroppedEstimate != nil {
		t.Fatalf("a kind filter reported truncation: truncated=%v dropped=%v", result.Truncated, result.DroppedEstimate)
	}
}

// An unfiltered query still returns everything, so the filter is a filter and
// not a mute button.
func TestQueryWithoutKindsReturnsEveryFrame(t *testing.T) {
	result, err := exampleDocsProvider{}.Query(decodeQuery(t,
		`{"goal":"anything","max_frames":8,"max_tokens":2000}`))
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(result.Frames) != 2 {
		t.Fatalf("unfiltered query returned %d frame(s), want 2", len(result.Frames))
	}
}

// §F4/§6.1: content that was not yet true at the pinned instant is not
// returned. The conformance suite pins at 2026-07-01, between the two canned
// frames' valid_from stamps, so the retain has observable work to do.
func TestQueryHonoursAsOfPin(t *testing.T) {
	const pin = "2026-07-01T00:00:00Z"
	result, err := exampleDocsProvider{}.Query(decodeQuery(t,
		`{"goal":"anything","as_of":"`+pin+`","max_frames":8,"max_tokens":2000}`))
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(result.Frames) == 0 {
		t.Fatal("as_of pin dropped every frame; the fixture no longer straddles the pin")
	}
	for _, frame := range result.Frames {
		if frame.ValidFrom > pin {
			t.Fatalf("frame %s (valid_from=%s) postdates as_of=%s (§F4)", frame.ID, frame.ValidFrom, pin)
		}
	}
	if len(result.Frames) == 2 {
		t.Fatal("as_of pin dropped nothing; the fixture no longer straddles the pin, so the check is decorative")
	}
}

// §E1: an empty vector has length 0, which contradicts a declared 384 exactly
// as 385 does. Rust, TypeScript and Python all reject it; Go used to let it
// through because it could only test length.
func TestQueryRejectsEmptyEmbedding(t *testing.T) {
	provider := exampleDocsProvider{}
	_, err := provider.Query(decodeQuery(t,
		`{"goal":"anything","embedding":[],"max_frames":8,"max_tokens":2000}`))
	if err == nil {
		t.Fatal("a present-but-empty embedding was accepted; the other three SDKs reject it (§E1)")
	}
	var providerError *cg.ProviderError
	if !errors.As(err, &providerError) {
		t.Fatalf("error was not a *ProviderError: %#v", err)
	}
	if providerError.Code != "bad_request" {
		t.Fatalf("code = %q, want bad_request", providerError.Code)
	}
}

// A query with no embedding at all is not an §E1 violation — there is nothing
// to contradict the declared fingerprint.
func TestQueryAcceptsAbsentAndCorrectlySizedEmbeddings(t *testing.T) {
	provider := exampleDocsProvider{}

	if _, err := provider.Query(decodeQuery(t,
		`{"goal":"anything","max_frames":8,"max_tokens":2000}`)); err != nil {
		t.Fatalf("absent embedding rejected: %v", err)
	}

	vector := make([]float64, embeddingDimensions)
	body, err := json.Marshal(map[string]any{
		"goal": "anything", "embedding": vector, "max_frames": 8, "max_tokens": 2000,
	})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if _, err := provider.Query(decodeQuery(t, string(body))); err != nil {
		t.Fatalf("correctly sized embedding rejected: %v", err)
	}
}

// A wrong-length embedding is still rejected, which is the case CI already
// probed — kept so the presence fix cannot regress it.
func TestQueryRejectsWrongLengthEmbedding(t *testing.T) {
	provider := exampleDocsProvider{}
	if _, err := provider.Query(decodeQuery(t,
		`{"goal":"anything","embedding":[0.1,0.2,0.3],"max_frames":8,"max_tokens":2000}`)); err == nil {
		t.Fatal("a 3-dimension embedding was accepted by a 384-dimension provider (§E1)")
	}
}
