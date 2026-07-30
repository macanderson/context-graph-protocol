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

func docFrame(id, title, content, file, rng string, score float64, digest string) cg.ContextFrame {
	return cg.ContextFrame{
		ID:            id,
		Kind:          "doc",
		Title:         title,
		Content:       content,
		ContentDigest: digest,
		URI:           "file:///docs/" + file,
		Score:         score,
		// Honest cost: ceil(utf8_len(content)/4) (B3).
		TokenCost:  cg.BudgetTokens(content),
		ValidFrom:  "2026-01-01T00:00:00Z",
		RecordedAt: "2026-07-20T18:00:00Z",
		Provenance: []cg.Provenance{{
			Type:   "file",
			URI:    "file:///docs/" + file,
			Range:  rng,
			Digest: digest,
			By:     "contextgraph-go-example-docs-http",
		}},
		CitationLabel: file + " " + rng,
		Relations: []cg.Relation{{
			Rel:         "doc.documents",
			TargetURI:   "symbol:///docs/" + file + "#overview",
			DisplayName: title + " overview",
		}},
	}
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
	if n := len(query.Embedding); n > 0 && n != embeddingDimensions {
		return cg.ContextQueryResult{}, cg.ProviderError{
			Code: "bad_request",
			Message: fmt.Sprintf(
				"query embedding has %d dimensions; this provider indexes %d (%s) (§E1)",
				n, embeddingDimensions, embeddingFingerprint,
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
			0.82,
			gettingStartedDigest,
		),
		docFrame(
			"frm_configuration",
			"Configuration",
			"Providers declare their data-flow direction at the handshake so hosts can gate consent before sending any query.",
			"configuration.md",
			"L1-25",
			0.61,
			configurationDigest,
		),
	}
	if len(query.Anchors) > 0 {
		sort.SliceStable(frames, func(i, j int) bool {
			return isAnchored(frames[i], query.Anchors) && !isAnchored(frames[j], query.Anchors)
		})
	}
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
