package main

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"

	cg "github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"
)

// The HTTP example carries the same provider logic as the stdio one, so it
// carries the same regressions and needs the same guards. Queries are decoded
// from wire bytes so an absent `embedding` stays distinguishable from an empty
// one, exactly as it is for a real host.
func decodeQuery(t *testing.T, body string) cg.ContextQuery {
	t.Helper()
	var query cg.ContextQuery
	if err := json.Unmarshal([]byte(body), &query); err != nil {
		t.Fatalf("unmarshal %s: %v", body, err)
	}
	return query
}

// §Q1 for every kind the provider declares, not just the first.
func TestQueryHonoursEveryDeclaredKind(t *testing.T) {
	for _, kind := range (exampleDocsProvider{}).Capabilities().Query.Kinds {
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

func TestQueryReturnsNoFramesForAnUnservedKind(t *testing.T) {
	result, err := exampleDocsProvider{}.Query(decodeQuery(t,
		`{"goal":"anything","kinds":["snippet"],"max_frames":8,"max_tokens":2000}`))
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(result.Frames) != 0 {
		t.Fatalf("kinds=[snippet] returned %d frame(s); this provider serves no snippets", len(result.Frames))
	}
	if result.Truncated || result.DroppedEstimate != nil {
		t.Fatalf("a kind filter reported truncation: truncated=%v dropped=%v", result.Truncated, result.DroppedEstimate)
	}
}

// §F4: the suite pins at 2026-07-01, between the two canned frames.
func TestQueryHonoursAsOfPin(t *testing.T) {
	const pin = "2026-07-01T00:00:00Z"
	result, err := exampleDocsProvider{}.Query(decodeQuery(t,
		`{"goal":"anything","as_of":"`+pin+`","max_frames":8,"max_tokens":2000}`))
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	if len(result.Frames) != 1 {
		t.Fatalf("as_of=%s returned %d frame(s), want 1 — the fixture must straddle the pin", pin, len(result.Frames))
	}
	if result.Frames[0].ValidFrom > pin {
		t.Fatalf("frame %s (valid_from=%s) postdates as_of=%s (§F4)", result.Frames[0].ID, result.Frames[0].ValidFrom, pin)
	}
}

// §E1: an empty vector contradicts a declared 384 exactly as 385 does.
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

// The whole point of the coded refusal is that it reaches the host with its
// code intact over this transport too.
func TestEmptyEmbeddingRefusalReachesTheWireWithItsCode(t *testing.T) {
	status, payload := cg.RespondToBody(exampleDocsProvider{},
		[]byte(`{"type":"query","id":"q1","query":{"goal":"anything","embedding":[],"max_frames":8,"max_tokens":2000}}`))
	if status != 200 {
		t.Fatalf("status = %d, want 200", status)
	}
	var reply struct {
		Type string `json:"type"`
		Code string `json:"code"`
	}
	if err := json.Unmarshal(payload, &reply); err != nil {
		t.Fatalf("body was not a JSON envelope: %v", err)
	}
	if reply.Type != "error" || reply.Code != "bad_request" {
		t.Fatalf("reply = %s, want an error envelope coded bad_request", payload)
	}
}

// An unserved kind must reach the wire as `"frames":[]`, never `null`.
func TestUnservedKindReachesTheWireAsAnEmptyArray(t *testing.T) {
	_, payload := cg.RespondToBody(exampleDocsProvider{},
		[]byte(`{"type":"query","id":"q1","query":{"goal":"anything","kinds":["snippet"],"max_frames":8,"max_tokens":2000}}`))
	if !strings.Contains(string(payload), `"frames":[]`) {
		t.Fatalf("body does not carry an empty frames array: %s", payload)
	}
}
