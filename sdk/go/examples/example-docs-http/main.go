// Command example-docs-http is the HTTP twin of example-docs: the same honest
// two-frame documentation provider, served over the "streamable HTTP" transport
// (SPEC.md §3) instead of stdio. It answers the whole CGP protocol on one POST
// endpoint, so the conformance suite can drive it remotely:
//
//	PORT=8789 go run ./sdk/go/examples/example-docs-http &
//	contextgraph-inspect http http://127.0.0.1:8789
//
// The provider logic is identical to the stdio example — only the transport
// differs, which is the whole point of a framework-agnostic Handler: write the
// provider once, host it however you like.
package main

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"os/signal"
	"sort"
	"strings"
	"syscall"
	"time"

	cg "github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"
)

const (
	embeddingFingerprint = "bge-small-en-v1.5/384/l2"
	embeddingDimensions  = 384
)

var (
	gettingStartedDigest = "sha256:" + strings.Repeat("11", 32)
	configurationDigest  = "sha256:" + strings.Repeat("22", 32)
)

func currentDigest(frameID string) (string, bool) {
	switch frameID {
	case "frm_getting_started":
		return gettingStartedDigest, true
	case "frm_configuration":
		return configurationDigest, true
	default:
		return "", false
	}
}

func docFrame(id, title, content, file, rng, validFrom string, score float64, digest string) cg.ContextFrame {
	return cg.ContextFrame{
		ID:            id,
		Kind:          "doc",
		Title:         title,
		Content:       cg.Ptr(content),
		ContentDigest: cg.Ptr(digest),
		URI:           "file:///docs/" + file,
		Score:         score,
		// Honest cost: ceil(utf8_len(content)/4) (B3).
		TokenCost:  cg.BudgetTokens(content),
		ValidFrom:  validFrom,
		RecordedAt: "2026-07-20T18:00:00Z",
		Provenance: []cg.Provenance{{
			Type:   "file",
			URI:    cg.Ptr("file:///docs/" + file),
			Range:  cg.Ptr(rng),
			Digest: cg.Ptr(digest),
			By:     cg.Ptr("contextgraph-go-example-docs-http"),
		}},
		CitationLabel: file + " " + rng,
		Relations: []cg.Relation{{
			Rel:         "doc.documents",
			TargetURI:   "symbol:///docs/" + file + "#overview",
			DisplayName: title + " overview",
		}},
	}
}

// retain keeps only the frames satisfying keep, preserving order — the Go
// spelling of the reference provider's Vec::retain. It filters in place, so the
// result is never nil even when nothing survives.
func retain(frames []cg.ContextFrame, keep func(cg.ContextFrame) bool) []cg.ContextFrame {
	kept := frames[:0]
	for _, frame := range frames {
		if keep(frame) {
			kept = append(kept, frame)
		}
	}
	return kept
}

// containsKind reports whether kinds names frame's kind.
func containsKind(kinds []string, kind string) bool {
	for _, candidate := range kinds {
		if candidate == kind {
			return true
		}
	}
	return false
}

func isAnchored(frame cg.ContextFrame, anchors []string) bool {
	for _, anchor := range anchors {
		if frame.URI == anchor {
			return true
		}
		for _, rel := range frame.Relations {
			if rel.TargetURI == anchor {
				return true
			}
		}
	}
	return false
}

type exampleDocsProvider struct{}

func (exampleDocsProvider) Info() cg.ProviderInfo {
	// The provider serves local canned frames; nothing leaves the machine of its
	// own accord, so it declares the honest local-only scope. The HTTP transport
	// is treated as egress by the host regardless (SPEC.md §4).
	return cg.ProviderInfo{
		Name:    "contextgraph-go-example-docs-http",
		Version: "0.1.0",
		DataFlow: cg.DataFlow{
			Reads:        true,
			Writes:       false,
			Egress:       false,
			EgressScopes: []string{"local-only"},
		},
	}
}

func (exampleDocsProvider) Capabilities() cg.Capabilities {
	fingerprint := embeddingFingerprint
	return cg.Capabilities{
		Query:                 cg.QueryCapability{Kinds: []string{"doc", "snippet"}},
		Correlation:           true,
		Graph:                 true,
		EmbeddingsFingerprint: &fingerprint,
		Verify:                true,
	}
}

func (exampleDocsProvider) Query(query cg.ContextQuery) (cg.ContextQueryResult, error) {
	// §E1: the test is presence, not length — an empty vector is length 0,
	// which contradicts 384 exactly as 385 does, and the other three
	// implementations reject it on that basis.
	if query.HasEmbedding() && len(query.Embedding) != embeddingDimensions {
		return cg.ContextQueryResult{}, &cg.ProviderError{
			Code: "bad_request",
			Message: fmt.Sprintf(
				"query embedding has %d dimensions; this provider indexes %d (%s) (§E1)",
				len(query.Embedding), embeddingDimensions, embeddingFingerprint,
			),
		}
	}
	frames := []cg.ContextFrame{
		docFrame(
			"frm_getting_started",
			"Getting Started",
			"Install the reference binding, then implement the required provider methods.",
			"getting-started.md",
			"L1-40",
			// Valid since the start of the year — before the as_of probe's pin.
			"2026-01-01T00:00:00Z",
			0.82,
			gettingStartedDigest,
		),
		docFrame(
			"frm_configuration",
			"Configuration",
			"Providers declare their data-flow direction at the handshake so hosts can gate consent before sending any query.",
			"configuration.md",
			"L1-25",
			// Became true only after the as_of probe's pin, so a mid-year pinned
			// query must not see it — the same disjoint window the reference
			// fixture gives its second frame.
			"2026-09-01T00:00:00Z",
			0.61,
			configurationDigest,
		),
	}
	// §Q1: a non-empty Kinds is a filter, not a hint. Both canned frames are
	// `doc`, so a query narrowed to `snippet` answers honestly with zero frames
	// rather than with documents the host excluded.
	if len(query.Kinds) > 0 {
		frames = retain(frames, func(frame cg.ContextFrame) bool {
			return containsKind(query.Kinds, frame.Kind)
		})
	}
	if len(query.Anchors) > 0 {
		sort.SliceStable(frames, func(i, j int) bool {
			return isAnchored(frames[i], query.Anchors) && !isAnchored(frames[j], query.Anchors)
		})
	}
	// §F4/§6.1: honour an as_of pin. One spelling per instant, so a
	// lexicographic compare on the UTC strings is a chronological one.
	if query.AsOf != "" {
		frames = retain(frames, func(frame cg.ContextFrame) bool {
			return frame.ValidFrom == "" || frame.ValidFrom <= query.AsOf
		})
	}
	// Truncated stays false however many frames the filters dropped: truncation
	// means the budget did not fit the candidates, not that the host's own
	// filter excluded some.
	return cg.ContextQueryResult{Frames: frames, Truncated: false}, nil
}

func (exampleDocsProvider) Verify(request cg.VerifyRequest) cg.VerifyResponse {
	verdicts := make([]cg.FrameVerdict, 0, len(request.Frames))
	for _, frame := range request.Frames {
		current, served := currentDigest(frame.FrameID)
		switch {
		case !served:
			verdicts = append(verdicts, cg.FrameVerdict{Frame: frame, Status: "gone"})
		case frame.ContentDigest == "":
			verdicts = append(verdicts, cg.FrameVerdict{Frame: frame, Status: "unknown"})
		case frame.ContentDigest == current:
			verdicts = append(verdicts, cg.FrameVerdict{Frame: frame, Status: "valid"})
		default:
			verdicts = append(verdicts, cg.FrameVerdict{Frame: frame, Status: "stale", ReplacementDigest: current})
		}
	}
	return cg.VerifyResponse{Verdicts: verdicts}
}

func main() {
	port := os.Getenv("PORT")
	if port == "" {
		port = "8789"
	}
	host := os.Getenv("HOST")
	if host == "" {
		host = "127.0.0.1"
	}
	addr := host + ":" + port
	server := &http.Server{Addr: addr, Handler: cg.Handler(exampleDocsProvider{})}

	go func() {
		fmt.Printf("contextgraph provider listening on http://%s\n", addr)
		if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			fmt.Fprintln(os.Stderr, "server error:", err)
			os.Exit(1)
		}
	}()

	// Exit cleanly on a supervisor's signal so a CI harness can reap the server.
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	<-sig
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	_ = server.Shutdown(ctx)
}
