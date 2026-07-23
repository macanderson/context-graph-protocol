/**
 * A tiny reference Context Graph Protocol provider, in TypeScript — the mirror
 * of the Rust `contextgraph-example-docs`. It serves two canned documentation
 * frames honestly, and is the fixture the language-neutral conformance suite
 * drives to prove a second, independent implementation passes:
 *
 * ```sh
 * contextgraph-inspect stdio --json -- node dist/examples/example-docs.js
 * ```
 */
import { budgetTokens } from "../src/budget.js";
import { runStdioProvider, type Provider } from "../src/provider.js";
import type {
  Capabilities,
  ContextFrame,
  ProviderInfo,
  VerdictStatus,
  VerifyRequest,
  VerifyResponse,
} from "../src/types.js";

// Stable, syntactically valid `sha256:<64 hex>` digests (SPEC.md §F5). Not real
// hashes of anything — this fixture serves string literals, not on-disk bytes —
// but well-formed, and the same value verify answers with, so the frames it
// serves and its verify verdicts can never drift apart.
const GETTING_STARTED_DIGEST = `sha256:${"11".repeat(32)}`;
const CONFIGURATION_DIGEST = `sha256:${"22".repeat(32)}`;

function currentDigest(frameId: string): string | undefined {
  switch (frameId) {
    case "frm_getting_started":
      return GETTING_STARTED_DIGEST;
    case "frm_configuration":
      return CONFIGURATION_DIGEST;
    default:
      return undefined;
  }
}

function docFrame(
  id: string,
  title: string,
  content: string,
  file: string,
  range: string,
  score: number,
  digest: string,
): ContextFrame {
  return {
    id,
    kind: "doc",
    title,
    content,
    content_digest: digest,
    uri: `file:///docs/${file}`,
    score,
    // Honest cost: ceil(utf8_len(content)/4) (B3).
    token_cost: budgetTokens(content),
    valid_from: "2026-01-01T00:00:00Z",
    recorded_at: "2026-07-20T18:00:00Z",
    provenance: [
      {
        type: "file",
        uri: `file:///docs/${file}`,
        range,
        digest,
        by: "contextgraph-ts-example-docs",
      },
    ],
    citation_label: `${file} ${range}`,
    relations: [],
  };
}

const provider: Provider = {
  info(): ProviderInfo {
    // A docs index reads the query and serves local frames; nothing leaves the
    // machine, so it honestly declares the `local-only` egress scope.
    return {
      name: "contextgraph-ts-example-docs",
      version: "0.1.0",
      data_flow: {
        reads: true,
        writes: false,
        egress: false,
        egress_scopes: ["local-only"],
      },
    };
  },

  capabilities(): Capabilities {
    return {
      query: { kinds: ["doc", "snippet"] },
      correlation: true,
      graph: false,
      // It can compare a presented digest against what it currently serves.
      verify: true,
    };
  },

  query() {
    return {
      frames: [
        docFrame(
          "frm_getting_started",
          "Getting Started",
          "Install the reference binding, then implement the required provider methods.",
          "getting-started.md",
          "L1-40",
          0.82,
          GETTING_STARTED_DIGEST,
        ),
        docFrame(
          "frm_configuration",
          "Configuration",
          "Providers declare their data-flow direction at the handshake so hosts can gate consent before sending any query.",
          "configuration.md",
          "L1-25",
          0.61,
          CONFIGURATION_DIGEST,
        ),
      ],
      truncated: false,
    };
  },

  verify(request: VerifyRequest): VerifyResponse {
    // Honest verify: compare each presented digest against the one currently
    // served. A differing digest is exactly what a mutated source looks like.
    return {
      verdicts: request.frames.map((frame) => {
        const current = currentDigest(frame.frame_id);
        let status: VerdictStatus;
        let replacement: string | undefined;
        if (current === undefined) {
          status = "gone";
        } else if (!frame.content_digest) {
          status = "unknown";
        } else if (frame.content_digest === current) {
          status = "valid";
        } else {
          status = "stale";
          replacement = current;
        }
        return replacement !== undefined
          ? { frame, status, replacement_digest: replacement }
          : { frame, status };
      }),
    };
  },
};

runStdioProvider(provider);
