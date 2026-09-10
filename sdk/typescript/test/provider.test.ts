/**
 * The stdio and HTTP runtimes' failure contract: `SPEC.md` §11 R1 — a provider
 * must not crash on a bad request, and must reply `error` with a code.
 *
 * Every assertion here is about a reply that *exists*. Before the fix these
 * paths produced no reply at all: `handleLine` rethrew an unexpected error and
 * `runStdioProvider`'s `.catch(() => {})` discarded it, so a correlating host
 * held the slot for an id that never came back. Silence is the regression;
 * a written envelope is the fix.
 *
 * The seam under test is {@link handleLine}'s `sinks` argument — the runtime
 * owns stdin, stdout and `process.exit`, so redirecting its two outputs is what
 * lets a unit test read the reply without spawning a child process.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { handleEnvelope, respondToEnvelopeBody } from "../src/http.js";
import { ProviderError, handleLine, type Provider } from "../src/provider.js";
import type { Capabilities, Envelope, ProviderInfo } from "../src/types.js";

const INFO: ProviderInfo = {
  name: "test-provider",
  version: "0.0.0",
  data_flow: { reads: true, writes: false, egress: false, egress_scopes: ["local-only"] },
};

const CAPS: Capabilities = { query: { kinds: ["doc"] }, correlation: true, verify: true };

/** A provider whose every method behaves as the test asks, and nothing more. */
function providerOf(overrides: Partial<Provider> = {}): Provider {
  return {
    info: () => INFO,
    capabilities: () => CAPS,
    query: () => ({ frames: [], truncated: false }),
    ...overrides,
  };
}

interface Captured {
  replies: Envelope[];
  logs: unknown[][];
}

/** Drive one line through the runtime, capturing both of its outputs. */
async function drive(provider: Provider, line: string): Promise<Captured> {
  const captured: Captured = { replies: [], logs: [] };
  await handleLine(provider, line, {
    write: (envelope) => captured.replies.push(envelope),
    logError: (message, error) => captured.logs.push([message, error]),
  });
  return captured;
}

function only(captured: Captured): Envelope {
  assert.equal(
    captured.replies.length,
    1,
    `expected exactly one reply, got ${captured.replies.length}`,
  );
  return captured.replies[0] as Envelope;
}

const QUERY_LINE = JSON.stringify({
  type: "query",
  id: "q1",
  query: { goal: "anything", max_frames: 8, max_tokens: 2000 },
});

const VERIFY_LINE = JSON.stringify({
  type: "verify",
  id: "v1",
  request: { frames: [{ provider_id: "p", frame_id: "f", content_digest: null }] },
});

test("an unexpected throw from query is answered `internal`, with the id echoed", async () => {
  const captured = await drive(
    providerOf({
      query: () => {
        throw new TypeError("cannot read properties of undefined (reading 'slice')");
      },
    }),
    QUERY_LINE,
  );

  const reply = only(captured);
  assert.equal(reply.type, "error");
  assert.equal(reply.type === "error" && reply.code, "internal");
  assert.equal(reply.type === "error" && reply.id, "q1");
  assert.match(
    reply.type === "error" ? reply.message : "",
    /cannot read properties of undefined/,
    "the real crash message reaches the host, not a generic placeholder",
  );
  // The operator still needs the actual error; the host only gets a code.
  assert.equal(captured.logs.length, 1, "the crash is reported to stderr");
  assert.ok(captured.logs[0]?.[1] instanceof TypeError);
});

test("an unexpected async rejection from query is answered `internal` too", async () => {
  const captured = await drive(
    providerOf({ query: () => Promise.reject(new Error("index unavailable")) }),
    QUERY_LINE,
  );

  const reply = only(captured);
  assert.equal(reply.type === "error" && reply.code, "internal");
  assert.equal(reply.type === "error" && reply.message, "index unavailable");
});

test("a ProviderError from query still answers with its own code, unlogged", async () => {
  const captured = await drive(
    providerOf({
      query: () => {
        throw new ProviderError("embedding has 3 dimensions, not 384 (§E1)", "bad_request");
      },
    }),
    QUERY_LINE,
  );

  const reply = only(captured);
  assert.equal(reply.type === "error" && reply.code, "bad_request");
  assert.equal(reply.type === "error" && reply.id, "q1");
  assert.equal(
    captured.logs.length,
    0,
    "a deliberate refusal is not an operator-facing crash",
  );
});

test("a codeless ProviderError answers without inventing a code", async () => {
  const captured = await drive(
    providerOf({
      query: () => {
        throw new ProviderError("no index loaded");
      },
    }),
    QUERY_LINE,
  );

  const reply = only(captured);
  assert.equal(reply.type, "error");
  assert.equal(reply.type === "error" && reply.code, undefined);
  assert.equal(reply.type === "error" && reply.message, "no index loaded");
});

test("an unexpected throw from verify is answered, not swallowed", async () => {
  const captured = await drive(
    providerOf({
      verify: () => {
        throw new Error("digest store offline");
      },
    }),
    VERIFY_LINE,
  );

  const reply = only(captured);
  assert.equal(reply.type, "error");
  assert.equal(reply.type === "error" && reply.code, "internal");
  assert.equal(reply.type === "error" && reply.id, "v1");
  assert.equal(captured.logs.length, 1);
});

test("a ProviderError from verify keeps its own code", async () => {
  const captured = await drive(
    providerOf({
      verify: () => {
        throw new ProviderError("too many frames presented", "bad_request");
      },
    }),
    VERIFY_LINE,
  );

  const reply = only(captured);
  assert.equal(reply.type === "error" && reply.code, "bad_request");
  assert.equal(captured.logs.length, 0);
});

test("a healthy verify still answers `verified`", async () => {
  const captured = await drive(
    providerOf({
      verify: (request) => ({
        verdicts: request.frames.map((frame) => ({ frame, status: "valid" as const })),
      }),
    }),
    VERIFY_LINE,
  );

  const reply = only(captured);
  assert.equal(reply.type, "verified");
  assert.equal(reply.type === "verified" && reply.response.verdicts.length, 1);
});

test("a throwing info()/capabilities() answers instead of stranding the handshake", async () => {
  const captured = await drive(
    providerOf({
      capabilities: () => {
        throw new Error("config not loaded");
      },
    }),
    JSON.stringify({ type: "handshake", protocol_version: "contextgraph/1.0" }),
  );

  const reply = only(captured);
  assert.equal(reply.type, "error");
  assert.equal(reply.type === "error" && reply.code, "internal");
  assert.equal(captured.logs.length, 1);
});

test("a healthy handshake still acks", async () => {
  const captured = await drive(
    providerOf(),
    JSON.stringify({ type: "handshake", protocol_version: "contextgraph/1.0" }),
  );

  const reply = only(captured);
  assert.equal(reply.type, "handshake_ack");
  assert.equal(reply.type === "handshake_ack" && reply.provider.name, "test-provider");
});

test("a line that parses to a non-object is `bad_request`, not silence", async () => {
  for (const line of ["42", '"x"', "[]", "null", "true"]) {
    const captured = await drive(providerOf(), line);
    const reply = only(captured);
    assert.equal(reply.type, "error", `line ${line} should be answered`);
    assert.equal(reply.type === "error" && reply.code, "bad_request");
  }
});

test("a line that is not JSON at all is still `bad_request`", async () => {
  const reply = only(await drive(providerOf(), "{not json"));
  assert.equal(reply.type === "error" && reply.code, "bad_request");
});

test("a query envelope missing its payload is `bad_request`, with the id echoed", async () => {
  const captured = await drive(providerOf(), JSON.stringify({ type: "query", id: "q9" }));
  const reply = only(captured);
  assert.equal(reply.type === "error" && reply.code, "bad_request");
  assert.equal(reply.type === "error" && reply.id, "q9");
  assert.equal(captured.logs.length, 0, "a malformed request is not a provider crash");
});

test("a verify envelope missing its payload is `bad_request`", async () => {
  const reply = only(await drive(providerOf(), JSON.stringify({ type: "verify" })));
  assert.equal(reply.type === "error" && reply.code, "bad_request");
});

test("a blank line is ignored entirely", async () => {
  const captured = await drive(providerOf(), "   ");
  assert.equal(captured.replies.length, 0);
});

// --- the HTTP sibling: same contract, same answers -------------------------

const HTTP_SINKS = { logError: () => {} };

test("HTTP: an unexpected throw from query becomes an `internal` envelope", async () => {
  const reply = await handleEnvelope(
    providerOf({
      query: () => {
        throw new Error("index unavailable");
      },
    }),
    { type: "query", id: "q1", query: { goal: "g", max_frames: 8, max_tokens: 2000 } },
    HTTP_SINKS,
  );

  assert.ok(reply);
  assert.equal(reply.type === "error" && reply.code, "internal");
  assert.equal(reply.type === "error" && reply.id, "q1");
});

test("HTTP: an unexpected throw from verify becomes an `internal` envelope", async () => {
  // The regression this pins: `respondToEnvelopeBody` is called directly from
  // framework routes that have no outer catch, so a throwing verify used to
  // reject a promise nobody handled rather than answering the host.
  const { status, body } = await respondToEnvelopeBody(
    providerOf({
      verify: () => {
        throw new Error("digest store offline");
      },
    }),
    JSON.stringify({
      type: "verify",
      request: { frames: [{ provider_id: "p", frame_id: "f" }] },
    }),
    HTTP_SINKS,
  );

  assert.equal(status, 200);
  const reply = JSON.parse(body) as Envelope;
  assert.equal(reply.type, "error");
  assert.equal(reply.type === "error" && reply.code, "internal");
});

test("HTTP: a ProviderError still keeps its own code", async () => {
  const reply = await handleEnvelope(
    providerOf({
      query: () => {
        throw new ProviderError("bad embedding (§E1)", "bad_request");
      },
    }),
    { type: "query", id: "q1", query: { goal: "g", max_frames: 8, max_tokens: 2000 } },
    HTTP_SINKS,
  );

  assert.ok(reply);
  assert.equal(reply.type === "error" && reply.code, "bad_request");
});

test("HTTP: a body that parses to a non-object is a 400 `bad_request`", async () => {
  const { status, body } = await respondToEnvelopeBody(providerOf(), "42", HTTP_SINKS);
  assert.equal(status, 400);
  assert.equal((JSON.parse(body) as { code?: string }).code, "bad_request");
});

test("HTTP: shutdown still ends the exchange without a body", async () => {
  const { status, body } = await respondToEnvelopeBody(
    providerOf(),
    JSON.stringify({ type: "shutdown" }),
    HTTP_SINKS,
  );
  assert.equal(status, 204);
  assert.equal(body, "");
});
