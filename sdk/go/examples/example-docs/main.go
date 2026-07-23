// Command example-docs is a tiny reference Context Graph Protocol provider, in
// Go — the mirror of the Rust contextgraph-example-docs and the TypeScript and
// Python examples. It serves two canned documentation frames honestly, and is
// the fixture the language-neutral conformance suite drives to prove a fourth
// independent implementation passes:
//
//	contextgraph-inspect stdio --json -- go run ./sdk/go/examples/example-docs
package main

import (
	"strings"

	cg "github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"
)

// Stable, syntactically valid sha256:<64 hex> digests (SPEC.md F5). Not real
// hashes of anything — this fixture serves string literals, not on-disk bytes —
// but well-formed, and the same value verify answers with, so served frames and
// verify verdicts can never drift apart.
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
			By:     "contextgraph-go-example-docs",
		}},
		CitationLabel: file + " " + rng,
	}
}

type exampleDocsProvider struct{}

func (exampleDocsProvider) Info() cg.ProviderInfo {
	// A docs index reads the query and serves local frames; nothing leaves the
	// machine, so it honestly declares the local-only egress scope.
	return cg.ProviderInfo{
		Name:    "contextgraph-go-example-docs",
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
	return cg.Capabilities{
		Query:       cg.QueryCapability{Kinds: []string{"doc", "snippet"}},
		Correlation: true,
		Graph:       false,
		Verify:      true,
	}
}

func (exampleDocsProvider) Query(_ cg.ContextQuery) cg.ContextQueryResult {
	return cg.ContextQueryResult{
		Frames: []cg.ContextFrame{
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
		},
		Truncated: false,
	}
}

// Verify implements cg.Verifier: compare each presented digest against what is
// currently served. A differing digest is exactly what a mutated source looks
// like from here.
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
	cg.RunStdioProvider(exampleDocsProvider{})
}
