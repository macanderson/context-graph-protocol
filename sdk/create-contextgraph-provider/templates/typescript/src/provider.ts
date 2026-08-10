/**
 * {{PROJECT_NAME}} — a Context Graph Protocol provider.
 *
 * This is the one place your provider's behavior lives. Both transports import
 * it: `stdio.ts` (a child process the host spawns) and `server.ts` (an HTTP
 * endpoint the host POSTs to). Edit `query` to serve your real frames — the
 * scaffold ships an honest two-frame docs example that passes the conformance
 * suite out of the box, so you always start from green.
 */
import { budgetTokens, ProviderError, type Provider } from "@contextgraphprotocol/typescript-sdk";
import type {
  Capabilities,
  ContextFrame,
  ProviderInfo,
  VerifyRequest,
  VerifyResponse,
  VerdictStatus,
} from "@contextgraphprotocol/typescript-sdk";

// Stable, syntactically valid `sha256:<64 hex>` digests (SPEC.md §F5). Replace
// these with real content hashes once you serve real bytes — verify compares
// the digest a host presents against the one you currently serve.
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
    // Honest cost: ceil(utf8_len(content)/4) (B3). Always compute it — never
    // guess — or the budget-honesty conformance check fails.
    token_cost: budgetTokens(content),
    valid_from: "2026-01-01T00:00:00Z",
    recorded_at: "2026-07-20T18:00:00Z",
    provenance: [
      {
        type: "file",
        uri: `file:///docs/${file}`,
        range,
        digest,
        by: "{{PROJECT_NAME}}",
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

export const provider: Provider = {
  info(): ProviderInfo {
    // Declare your data-flow honestly. `egress: false` here because this example
    // only serves local canned frames; set it true if your provider reaches any
    // remote service, or the host cannot gate consent correctly.
    return {
      name: "{{PROJECT_NAME}}",
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
      query: { kinds: ["doc"] },
      // Echo request ids so a host can pipeline — the SDK does the echo for you.
      correlation: true,
      // This provider surfaces graph relations, so anchored queries can boost.
      graph: true,
      // It can revalidate frame identities it served (see `verify` below).
      verify: true,
    };
  },

  query(query) {
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
    // §G4: rank anchored frames first when the query carries anchors.
    const anchors = query.anchors ?? [];
    if (anchors.length > 0) {
      frames.sort(
        (a, b) => Number(isAnchored(b, anchors)) - Number(isAnchored(a, anchors)),
      );
    }
    // If you have more relevant material than fits `query.max_tokens`, return
    // your best frames within budget and set `truncated: true` instead.
    void ProviderError; // exported for you to throw a coded refusal (e.g. §E1).
    return { frames, truncated: false };
  },

  verify(request: VerifyRequest): VerifyResponse {
    // Compare each presented digest against what you currently serve. Never send
    // frame bodies back — a `verified` reply carries identities only.
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
