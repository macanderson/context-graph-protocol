// Command example-docs is a tiny reference Context Graph Protocol provider, in
// Go — the mirror of the Rust contextgraph-example-docs and the TypeScript and
// Python examples. It serves two canned documentation frames honestly, and is
// the fixture the language-neutral conformance suite drives to prove a fourth
// independent implementation passes:
//
//	contextgraph-inspect stdio --json -- go run ./sdk/go/examples/example-docs
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"

	cg "github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"
)

// The embedding space this fixture declares it indexes (SPEC.md §E1). Its
// dimension -- the 2nd `/`-separated segment (384) -- is the length a query
// embedding must match; a contradicting length is a vector from a different
// space, rejected `bad_request` rather than scored into meaningless similarity.
const (
	embeddingFingerprint = "bge-small-en-v1.5/384/l2"
	embeddingDimensions  = 384
)

// fixtureDir is the directory holding this provider's on-disk backing files,
// resolved from this source file's own recorded path so a digest is computed
// over the same bytes no matter where the provider is spawned from (SPEC.md
// §6.2). runtime.Caller records the path at build time; the conformance run
// builds and probes on the same machine, so the path resolves.
func fixtureDir() string {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		return "fixtures"
	}
	return filepath.Join(filepath.Dir(file), "fixtures")
}

// fixtureURI is the absolute file:// URI a host re-reads to verify a frame's
// provenance digest (provenance-fixture-consistency). Absolute and
// cwd-independent, so verification never depends on the host's working
// directory.
func fixtureURI(file string) string {
	return "file://" + filepath.ToSlash(filepath.Join(fixtureDir(), file))
}

// fixtureDigest is the real sha256:<64 lowercase hex> digest over a backing
// file's exact on-disk bytes — byte-for-byte what a host recomputes when it
// re-reads the file, so an unmutated frame verifies end to end (SPEC.md §6.2,
// §F5).
func fixtureDigest(file string) string {
	bytes, err := os.ReadFile(filepath.Join(fixtureDir(), file))
	if err != nil {
		bytes = nil
	}
	sum := sha256.Sum256(bytes)
	return "sha256:" + hex.EncodeToString(sum[:])
}

var (
	gettingStartedDigest = fixtureDigest("getting-started.md")
	configurationDigest  = fixtureDigest("configuration.md")
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
		URI:           fixtureURI(file),
		Score:         score,
		// Honest cost: ceil(utf8_len(content)/4) (B3).
		TokenCost:  cg.BudgetTokens(content),
		ValidFrom:  "2026-01-01T00:00:00Z",
		RecordedAt: "2026-07-20T18:00:00Z",
		Provenance: []cg.Provenance{{
			Type:   "file",
			URI:    fixtureURI(file),
			Range:  rng,
			Digest: digest,
			By:     "contextgraph-go-example-docs",
		}},
		CitationLabel: file + " " + rng,
		// A labelled edge to the symbol this page documents. §G4 makes a frame
		// "anchored" when its own URI or any relation's TargetURI equals a query
		// anchor, so this edge is what an anchored query reaches at one hop.
		Relations: []cg.Relation{{
			Rel:         "doc.documents",
			TargetURI:   "symbol:///docs/" + file + "#overview",
			DisplayName: title + " overview",
		}},
	}
}

// isAnchored reports whether frame is anchored by any of anchors (SPEC.md §G4):
// zero hops via the frame's own URI, one hop via a relation's TargetURI. String
// equality, so two implementations cannot disagree about what "anchored" means.
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
	// Declaring the embedding space it indexes lets the provider reject a vector
	// from a different one (§E1). A provider that declares no fingerprint has
	// nothing to contradict and is not E1-probed.
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
	// §E1: a query embedding whose length contradicts this provider's declared
	// fingerprint dimension names a different vector space; scoring it would
	// yield plausible-looking, meaningless similarity. An honest provider
	// rejects it `bad_request` rather than pretending.
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
	// §G4: a frame is anchored when its own URI, or any relation's TargetURI,
	// equals one of the query's anchors. A graph-declaring provider ranks
	// anchored frames first — the "boost" §G3 asks for. SliceStable keeps the
	// relative order of equally-anchored frames, so composition stays
	// deterministic.
	if len(query.Anchors) > 0 {
		sort.SliceStable(frames, func(i, j int) bool {
			return isAnchored(frames[i], query.Anchors) && !isAnchored(frames[j], query.Anchors)
		})
	}
	return cg.ContextQueryResult{Frames: frames, Truncated: false}, nil
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
