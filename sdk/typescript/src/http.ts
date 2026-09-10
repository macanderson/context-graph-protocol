/**
 * The HTTP adapter: host a {@link Provider} behind a single POST endpoint,
 * speaking the same Context Graph Protocol wire as {@link runStdioProvider} —
 * the "streamable HTTP" transport (`SPEC.md` §3). The host POSTs one
 * {@link Envelope} as the request body and expects one {@link Envelope} back as
 * the response body; {@link handleEnvelope} is that request/response state
 * machine, framework-agnostic so it drops under Express, Fastify, or a plain
 * `node:http` server.
 *
 * The one deliberate difference from stdio: an HTTP provider is a long-lived
 * server reached by many independent hosts, so a `shutdown` envelope ends *that
 * exchange* — it never calls `process.exit`. (`contextgraph-inspect http` in
 * fact handshakes and shuts down twice per run: once to probe, once to run the
 * conformance suite. A server that exited on the first shutdown could not
 * answer the second handshake.)
 */
import type { IncomingMessage, ServerResponse } from "node:http";

import {
  type ErrorEnvelope,
  type Provider,
  type ProviderSinks,
  errorEnvelopeFor,
  isJsonObject,
} from "./provider.js";
import { type Envelope, type VerifyResponse, PROTOCOL_VERSION } from "./types.js";

function badRequest(message: string, id: string | undefined): ErrorEnvelope {
  const reply: ErrorEnvelope = { type: "error", code: "bad_request", message };
  if (id !== undefined) reply.id = id;
  return reply;
}

/**
 * The correlation `id` on an envelope, if it carries a usable one. `verify` has
 * no `id` in the wire schema today; reading it structurally means a host that
 * does pipeline verify still gets it echoed on an error reply.
 */
function correlationId(envelope: Envelope): string | undefined {
  const id = (envelope as { id?: unknown }).id;
  return typeof id === "string" ? id : undefined;
}

/**
 * Drive one request {@link Envelope} through `provider` and return the one
 * response envelope — or `null` for a `shutdown` (and for any host→provider
 * envelope a provider must ignore), which has no reply body.
 *
 * This is the whole protocol state machine, transport-free: hand it a decoded
 * envelope from whatever web framework you use and serialize what it returns.
 * It mirrors {@link handleLine}'s per-line handling exactly — including echoing
 * a `query`'s correlation `id` (H4), answering a
 * {@link import("./provider.js").ProviderError} with its own code (§E1), and
 * answering any other throw `internal` — minus the process lifecycle.
 *
 * It never rejects for a fault inside `provider`. That matters because
 * {@link respondToEnvelopeBody} is called from framework routes that have no
 * outer catch of their own: only {@link createHttpHandler} wraps it in a 500,
 * so relying on the throw escaping would leave every other caller answering
 * nothing at all (`SPEC.md` §11 R1).
 *
 * `sinks` exists for the same reason as {@link handleLine}'s: a test can
 * capture the stderr report instead of printing it. Both sinks default to the
 * real streams.
 */
export async function handleEnvelope(
  provider: Provider,
  envelope: Envelope,
  sinks: ProviderSinks = {},
): Promise<Envelope | null> {
  const logError = sinks.logError ?? ((message, error) => console.error(message, error));

  switch (envelope.type) {
    case "handshake":
      // `info()` and `capabilities()` are provider code and can throw; an
      // unanswered handshake strands the host before the session starts.
      try {
        return {
          type: "handshake_ack",
          protocol_version: PROTOCOL_VERSION,
          provider: provider.info(),
          capabilities: provider.capabilities(),
        };
      } catch (error) {
        return errorEnvelopeFor(error, "provider.info()/capabilities()", logError);
      }

    case "query": {
      const queryId = envelope.id;
      // A `query` envelope with no `query` payload is the host's mistake:
      // `bad_request`, not a provider crash reported as `internal`.
      if (!isJsonObject(envelope.query)) {
        return badRequest("query envelope is missing its `query` payload", queryId);
      }
      let reply: Envelope;
      try {
        const result = await provider.query(envelope.query);
        reply = { type: "frames", result };
      } catch (error) {
        // A deliberate, coded refusal of a request the provider can't honestly
        // serve (§E1) keeps its own code; anything else is answered `internal`
        // and reported to stderr, rather than thrown at a caller that may have
        // nothing to catch it.
        reply = errorEnvelopeFor(error, "provider.query()", logError);
      }
      // Echo the correlation id so the host can match reply to request (H4).
      if (queryId !== undefined) reply.id = queryId;
      return reply;
    }

    case "verify": {
      const verifyId = correlationId(envelope);
      if (!isJsonObject(envelope.request) || !Array.isArray(envelope.request.frames)) {
        return badRequest(
          "verify envelope is missing its `request.frames` payload",
          verifyId,
        );
      }
      try {
        const response: VerifyResponse = provider.verify
          ? await provider.verify(envelope.request)
          : {
              // No verify support ⇒ vouch for nothing; the host re-queries.
              verdicts: envelope.request.frames.map((frame) => ({
                frame,
                status: "unknown" as const,
              })),
            };
        return { type: "verified", response };
      } catch (error) {
        // Identical stranded-host outcome to a failing query, so identical
        // answer. `verified` carries no `id`, but an error reply does (H4).
        const reply = errorEnvelopeFor(error, "provider.verify()", logError);
        if (verifyId !== undefined) reply.id = verifyId;
        return reply;
      }
    }

    case "shutdown":
      // End the exchange, but keep the server alive for the next host — an HTTP
      // provider is not a child process to reap. The host expects no reply body.
      return null;

    default:
      // handshake_ack / frames / verified / error are host→provider-invalid
      // inputs; a provider ignores them (no reply).
      return null;
  }
}

/** A ready-to-send HTTP response: a status code and a JSON (or empty) body. */
export interface EnvelopeHttpResponse {
  status: number;
  body: string;
}

/**
 * Decode one raw request body, drive it through {@link handleEnvelope}, and
 * return the status + serialized envelope to send back. Use this when your
 * framework hands you the body as a string (Fastify, a hand-rolled route):
 * respond with `res.status(status).send(body)` or the equivalent.
 *
 * A body that is not a valid CGP envelope is answered `400` with a coded
 * `error` envelope rather than crashing — the HTTP mirror of the stdio
 * `malformed-input-tolerance` guarantee.
 */
export async function respondToEnvelopeBody(
  provider: Provider,
  rawBody: string,
  sinks: ProviderSinks = {},
): Promise<EnvelopeHttpResponse> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawBody);
  } catch {
    return {
      status: 400,
      body: JSON.stringify({
        type: "error",
        code: "bad_request",
        message: "request body was not a valid CGP envelope",
      }),
    };
  }
  // `JSON.parse` accepts `42`, `"x"`, `[]`, `null` and `true`; none of them has
  // a `type` to dispatch on, so they are bad requests, not envelopes.
  if (!isJsonObject(parsed)) {
    return {
      status: 400,
      body: JSON.stringify({
        type: "error",
        code: "bad_request",
        message: "request body was not a CGP envelope object",
      }),
    };
  }
  const reply = await handleEnvelope(provider, parsed as Envelope, sinks);
  // `shutdown` (and ignored inputs) has no reply body: 204 No Content.
  if (reply === null) return { status: 204, body: "" };
  return { status: 200, body: JSON.stringify(reply) };
}

/**
 * A `node:http`-compatible request listener that reads the raw request body,
 * drives it through the provider, and writes the envelope response — the
 * zero-config path:
 *
 * ```ts
 * import { createServer } from "node:http";
 * createServer(createHttpHandler(provider)).listen(8787);
 * ```
 *
 * It also mounts directly as an Express route
 * (`app.post("/contextgraph", createHttpHandler(provider))`) **as long as no
 * JSON body-parser runs first** — it reads the stream itself, so it stays
 * dependency-free and parser-agnostic. Under Fastify (which pre-reads the body)
 * call {@link respondToEnvelopeBody} with `request.body` instead.
 */
export function createHttpHandler(
  provider: Provider,
): (req: IncomingMessage, res: ServerResponse) => void {
  return (req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const rawBody = Buffer.concat(chunks).toString("utf8");
      respondToEnvelopeBody(provider, rawBody)
        .then(({ status, body }) => {
          res.writeHead(status, { "content-type": "application/json" });
          res.end(body);
        })
        .catch((error: unknown) => {
          // `respondToEnvelopeBody` already answers every provider fault with an
          // `error` envelope, so reaching this catch means the adapter itself
          // failed (an unserializable reply, say). Report it as a coded error
          // envelope with a 500, never a dangling socket.
          const message = error instanceof Error ? error.message : String(error);
          console.error("contextgraph: failed to answer an HTTP envelope", error);
          res.writeHead(500, { "content-type": "application/json" });
          res.end(JSON.stringify({ type: "error", code: "internal", message }));
        });
    });
    req.on("error", () => {
      res.writeHead(400, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          type: "error",
          code: "bad_request",
          message: "could not read request body",
        }),
      );
    });
  };
}
