/**
 * A tiny reference Context Graph Protocol provider, in TypeScript — the mirror
 * of the Rust `contextgraph-example-docs`. It serves two canned frames honestly
 * — one `doc` and one `snippet`, with disjoint validity windows, so the `kinds`
 * filter (§Q1) and the `as_of` pin (§F4) both have observable work to do — and
 * is the fixture the language-neutral conformance suite drives to prove a
 * second, independent implementation passes:
 *
 * ```sh
 * contextgraph-inspect stdio --json -- node dist/examples/example-docs.js
 * ```
 */
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { budgetTokens } from "../src/budget.js";
import { ProviderError, runStdioProvider, type Provider } from "../src/provider.js";
import type {
  Capabilities,
  ContextFrame,
  FrameKind,
  ProviderInfo,
  VerdictStatus,
  VerifyRequest,
  VerifyResponse,
} from "../src/types.js";

// The directory holding this provider's on-disk backing files. Resolved from
// this module's own location so a digest is computed over the same bytes no
// matter where the provider is spawned from (SPEC.md §6.2). The compiled
// module runs from `dist/examples/`, where `fixtures/` is not copied — the
// second candidate reaches the source tree's copy.
const MODULE_DIR = dirname(fileURLToPath(import.meta.url));
const FIXTURE_DIR = [
  join(MODULE_DIR, "fixtures"),
  join(MODULE_DIR, "..", "..", "examples", "fixtures"),
].find((dir) => existsSync(dir)) ?? join(MODULE_DIR, "fixtures");

/**
 * The absolute `file://` URI a host re-reads to verify a frame's provenance
 * digest (`provenance-fixture-consistency`). Absolute and cwd-independent, so
 * verification never depends on the host's working directory.
 */
function fixtureUri(file: string): string {
  return pathToFileURL(join(FIXTURE_DIR, file)).href;
}

/**
 * The real `sha256:<64 lowercase hex>` digest over a backing file's exact
 * on-disk bytes — byte-for-byte what a host recomputes when it re-reads the
 * file, so an unmutated frame verifies end to end (SPEC.md §6.2, §F5).
 */
function fixtureDigest(file: string): string {
  let bytes: Buffer;
  try {
    bytes = readFileSync(join(FIXTURE_DIR, file));
  } catch {
    bytes = Buffer.alloc(0);
  }
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

const GETTING_STARTED_DIGEST = fixtureDigest("getting-started.md");
const CONFIGURATION_DIGEST = fixtureDigest("configuration.md");

// The embedding space this fixture declares it indexes (SPEC.md §E1). Its
// dimension — the 2nd `/`-separated segment (384) — is the length a query
// embedding must match; a contradicting length is a vector from a different
// space, rejected `bad_request` rather than scored into meaningless similarity.
/**
 * Whether `frame` is anchored by any of `anchors` (SPEC.md §G4): zero hops via
 * the frame's own `uri`, one hop via a relation's `target_uri`. String
 * equality, so two implementations cannot disagree about what "anchored" means.
 */
function isAnchored(frame: ContextFrame, anchors: string[]): boolean {
  if (frame.uri !== undefined && anchors.includes(frame.uri)) return true;
  return (frame.relations ?? []).some((rel) => anchors.includes(rel.target_uri));
}

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

/**
 * One canned frame.
 *
 * `kind` and `validFrom` are parameters rather than constants because the two
 * frames deliberately differ in both, mirroring the Rust reference fixture: a
 * provider that declares `["doc", "snippet"]` and serves only `doc` frames
 * makes §Q1 unobservable (every kind-narrowed query it can be asked returns
 * frames it would have returned anyway), and two frames sharing one validity
 * window makes an `as_of` pin unobservable the same way.
 */
function docFrame(
  id: string,
  kind: FrameKind,
  title: string,
  content: string,
  file: string,
  range: string,
  score: number,
  digest: string,
  validFrom: string,
): ContextFrame {
  return {
    id,
    kind,
    title,
    content,
    content_digest: digest,
    uri: fixtureUri(file),
    score,
    // Honest cost: ceil(utf8_len(content)/4) (B3).
    token_cost: budgetTokens(content),
    valid_from: validFrom,
    recorded_at: "2026-07-20T18:00:00Z",
    provenance: [
      {
        type: "file",
        uri: fixtureUri(file),
        range,
        digest,
        by: "contextgraph-ts-example-docs",
      },
    ],
    citation_label: `${file} ${range}`,
    // A labelled edge to the symbol this page documents. §G4 makes a frame
    // "anchored" when its own `uri` or any relation's `target_uri` equals a
    // query anchor, so this edge is what an anchored query reaches at one hop.
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
      graph: true,
      // Declaring the embedding space it indexes lets the provider reject a
      // vector from a different one (§E1). A provider that declares no
      // fingerprint has nothing to contradict and is not E1-probed.
      embeddings_fingerprint: EMBEDDING_FINGERPRINT,
      // It can compare a presented digest against what it currently serves.
      verify: true,
    };
  },

  query(query) {
    // §E1: a query embedding whose length contradicts this provider's declared
    // fingerprint dimension names a different vector space; scoring it would
    // yield plausible-looking, meaningless similarity. An honest provider
    // rejects it `bad_request` rather than pretending.
    const embedding = query.embedding;
    if (embedding !== undefined && embedding.length !== EMBEDDING_DIMENSIONS) {
      throw new ProviderError(
        `query embedding has ${embedding.length} dimensions; this provider indexes ${EMBEDDING_DIMENSIONS} (${EMBEDDING_FINGERPRINT}) (§E1)`,
        "bad_request",
      );
    }
    let frames = [
      // Valid since the start of the year — before the conformance suite's
      // `as_of` pin, so a pinned query still reaches it.
      docFrame(
        "frm_getting_started",
        "doc",
        "Getting Started",
        "Install the reference binding, then implement the required provider methods.",
        "getting-started.md",
        "L1-40",
        0.82,
        GETTING_STARTED_DIGEST,
        "2026-01-01T00:00:00Z",
      ),
      // A `snippet`, and one that only became true in the autumn: the second
      // frame is the one that gives §Q1 and §F4 something to observe. It is the
      // frame a `kinds: ["doc"]` query must drop and a `kinds: ["snippet"]`
      // query must keep, and the one a mid-year `as_of` pin must exclude.
      docFrame(
        "frm_configuration",
        "snippet",
        "Configuration example",
        "const host = createHost().withProvider(\"docs\", provider);",
        "configuration.md",
        "L1-25",
        0.61,
        CONFIGURATION_DIGEST,
        "2026-09-01T00:00:00Z",
      ),
    ];
    // §Q1: a non-empty `kinds` is a filter, not a hint. Returning a frame
    // outside it spends the host's budget on content it explicitly excluded.
    const kinds = query.kinds ?? [];
    if (kinds.length > 0) {
      frames = frames.filter((frame) => kinds.includes(frame.kind));
    }
    // §G4: a frame is anchored when its own `uri`, or any relation's
    // `target_uri`, equals one of the query's anchors. A graph-declaring
    // provider ranks anchored frames first — the "boost" §G3 asks for.
    const anchors = query.anchors ?? [];
    if (anchors.length > 0) {
      frames.sort(
        (a, b) => Number(isAnchored(b, anchors)) - Number(isAnchored(a, anchors)),
      );
    }
    // §F4/§6.1: honour an `as_of` pin — content that was not yet true at the
    // pinned instant is not returned. The timestamp profile admits one spelling
    // per instant, so a lexicographic compare on the UTC strings is a
    // chronological one.
    const asOf = query.as_of;
    if (asOf !== undefined) {
      frames = frames.filter(
        (frame) => frame.valid_from === undefined || frame.valid_from <= asOf,
      );
    }
    // `truncated` stays false: these filters are the host's own narrowing being
    // honoured, not this provider running out of budget. Reporting them as
    // truncation would tell the host it had missed relevant content when in
    // fact it had excluded it (§B2).
    return { frames, truncated: false };
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
