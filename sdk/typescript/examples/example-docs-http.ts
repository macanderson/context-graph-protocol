/**
 * The HTTP twin of `example-docs.ts`: the same honest two-frame documentation
 * provider, served over the "streamable HTTP" transport (`SPEC.md` §3) instead
 * of stdio. It answers the whole CGP protocol on one POST endpoint, so the
 * conformance suite can drive it remotely:
 *
 * ```sh
 * PORT=8787 node dist/examples/example-docs-http.js &
 * contextgraph-inspect http http://127.0.0.1:8787
 * ```
 *
 * The provider logic is identical to the stdio example — only the transport
 * differs, which is the whole point of a framework-agnostic {@link handleEnvelope}:
 * write the provider once, host it however you like.
 */
import { createServer } from "node:http";

import { budgetTokens } from "../src/budget.js";
import { createHttpHandler } from "../src/http.js";
import { ProviderError, type Provider } from "../src/provider.js";
import type {
  Capabilities,
  ContextFrame,
  ProviderInfo,
  VerifyRequest,
  VerifyResponse,
  VerdictStatus,
} from "../src/types.js";

// Stable, syntactically valid `sha256:<64 hex>` digests (SPEC.md §F5) — the same
// values verify answers with, so served frames and verify verdicts never drift.
const GETTING_STARTED_DIGEST = `sha256:${"11".repeat(32)}`;
const CONFIGURATION_DIGEST = `sha256:${"22".repeat(32)}`;

const EMBEDDING_FINGERPRINT = "bge-small-en-v1.5/384/l2";
const EMBEDDING_DIMENSIONS = Number(EMBEDDING_FINGERPRINT.split("/")[1]);

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

function isAnchored(frame: ContextFrame, anchors: string[]): boolean {
  if (frame.uri !== undefined && anchors.includes(frame.uri)) return true;
  return (frame.relations ?? []).some((rel) => anchors.includes(rel.target_uri));
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
        by: "contextgraph-ts-example-docs-http",
      },
    ],
    citation_label: `${file} ${range}`,
    relations: [
      {
        rel: "doc.documents",
        target_uri: `symbol:///docs/${file}#overview`,
        display_name: `${title} overview`,
      },
    ],
  };
}

const provider: Provider = {
  info(): ProviderInfo {
    // The provider itself serves local canned frames; nothing leaves the
    // machine of its own accord, so it declares the honest local-only scope.
    // The HTTP transport is treated as egress by the host regardless (a remote
    // can't lie its way out of the consent gate — see SPEC.md §4).
    return {
      name: "contextgraph-ts-example-docs-http",
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
      graph: true,
      embeddings_fingerprint: EMBEDDING_FINGERPRINT,
      verify: true,
    };
  },

  query(query) {
    const embedding = query.embedding;
    if (embedding !== undefined && embedding.length !== EMBEDDING_DIMENSIONS) {
      throw new ProviderError(
        `query embedding has ${embedding.length} dimensions; this provider indexes ${EMBEDDING_DIMENSIONS} (${EMBEDDING_FINGERPRINT}) (§E1)`,
        "bad_request",
      );
    }
    const frames = [
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
    ];
    const anchors = query.anchors ?? [];
    if (anchors.length > 0) {
      frames.sort(
        (a, b) => Number(isAnchored(b, anchors)) - Number(isAnchored(a, anchors)),
      );
    }
    return { frames, truncated: false };
  },

  verify(request: VerifyRequest): VerifyResponse {
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

const port = Number(process.env.PORT ?? "8787");
const host = process.env.HOST ?? "127.0.0.1";
const server = createServer(createHttpHandler(provider));
server.listen(port, host, () => {
  // One line to stdout so a supervising script (or a human) knows the URL to
  // point `contextgraph-inspect http` at.
  process.stdout.write(`contextgraph provider listening on http://${host}:${port}\n`);
});

// Exit cleanly on a supervisor's signal so a CI harness can reap the server.
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
