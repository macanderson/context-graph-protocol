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

import { type Provider, ProviderError } from "./provider.js";
import { type Envelope, type VerifyResponse, PROTOCOL_VERSION } from "./types.js";

/**
 * Drive one request {@link Envelope} through `provider` and return the one
 * response envelope — or `null` for a `shutdown` (and for any host→provider
 * envelope a provider must ignore), which has no reply body.
 *
 * This is the whole protocol state machine, transport-free: hand it a decoded
 * envelope from whatever web framework you use and serialize what it returns.
 * It mirrors {@link runStdioProvider}'s per-line handling exactly — including
 * echoing a `query`'s correlation `id` (H4) and catching a {@link ProviderError}
 * into a coded `error` envelope (§E1) — minus the process lifecycle.
 */
export async function handleEnvelope(
  provider: Provider,
  envelope: Envelope,
): Promise<Envelope | null> {
  switch (envelope.type) {
    case "handshake":
      return {
        type: "handshake_ack",
        protocol_version: PROTOCOL_VERSION,
        provider: provider.info(),
        capabilities: provider.capabilities(),
      };

    case "query": {
      let reply: Envelope;
      try {
        const result = await provider.query(envelope.query);
        reply = { type: "frames", result };
      } catch (error) {
        // A deliberate, coded refusal of a request the provider can't honestly
        // serve (§E1) becomes an `error` envelope, not frames. Anything else is
        // a real crash; let it propagate to the transport's 500 handler.
        if (!(error instanceof ProviderError)) throw error;
        reply =
          error.code !== undefined
            ? { type: "error", message: error.message, code: error.code }
            : { type: "error", message: error.message };
      }
      // Echo the correlation id so the host can match reply to request (H4).
      if (envelope.id !== undefined) reply.id = envelope.id;
      return reply;
    }

    case "verify": {
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
): Promise<EnvelopeHttpResponse> {
  let envelope: Envelope;
  try {
    envelope = JSON.parse(rawBody) as Envelope;
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
  const reply = await handleEnvelope(provider, envelope);
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
          // A non-ProviderError crash in the handler: report it as a coded
          // error envelope with a 500, never a dangling socket.
          const message = error instanceof Error ? error.message : String(error);
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
