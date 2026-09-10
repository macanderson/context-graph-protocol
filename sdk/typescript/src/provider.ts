/**
 * The provider runtime: implement {@link Provider}, hand it to
 * {@link runStdioProvider}, and you have a conformant Context Graph Protocol
 * provider speaking the line-oriented JSON wire over stdio.
 *
 * The runtime handles the whole lifecycle a host drives — handshake, query
 * (echoing the correlation `id`), verify, shutdown — and, crucially, stays
 * alive and replies with a typed error on a malformed line rather than
 * crashing (the `malformed-input-tolerance` guarantee).
 */
import * as readline from "node:readline";

import {
  type Capabilities,
  type ContextQuery,
  type ContextQueryResult,
  type Envelope,
  type ProviderInfo,
  type VerifyRequest,
  type VerifyResponse,
  PROTOCOL_VERSION,
} from "./types.js";

/**
 * A Context Graph Protocol provider. `query` is mandatory; `verify` is optional
 * (a provider that omits it is treated as unable to vouch for its frames, and
 * the host re-queries). Both may be sync or async.
 */
export interface Provider {
  /** Identity and data-flow posture, reported at handshake. */
  info(): ProviderInfo;
  /** What this provider can do, negotiated at handshake. */
  capabilities(): Capabilities;
  /** Answer a retrieval request with budgeted, provenance-carrying frames. */
  query(query: ContextQuery): ContextQueryResult | Promise<ContextQueryResult>;
  /** Revalidate frames a host already holds (identities only — never bodies). */
  verify?(request: VerifyRequest): VerifyResponse | Promise<VerifyResponse>;
}

/**
 * A protocol-level error a provider throws from `query` to reply with an
 * `error` envelope carrying a machine-readable `code` instead of frames.
 *
 * This is how a provider refuses a request it cannot honestly serve — e.g.
 * rejecting a query embedding whose length contradicts its declared
 * `embeddings_fingerprint` dimension with `bad_request` (`SPEC.md` §E1). The
 * runtime catches it, echoes the request's correlation `id`, and writes an
 * `error` envelope carrying `code`.
 *
 * A thrown value that is *not* a `ProviderError` is an unexpected crash rather
 * than a refusal, so it gets no `code` of its own: the runtime answers `internal`
 * and logs the real error to stderr. Either way the host gets exactly one reply
 * per request — silence is never a valid answer (`SPEC.md` §11 R1).
 */
export class ProviderError extends Error {
  readonly code?: string;
  constructor(message: string, code?: string) {
    super(message);
    this.name = "ProviderError";
    this.code = code;
  }
}

function writeEnvelope(envelope: Envelope): void {
  process.stdout.write(`${JSON.stringify(envelope)}\n`);
}

/**
 * Where a line handler sends its two outputs. Both default to the real process
 * streams; {@link handleLine} takes them as an argument purely so a unit test
 * can observe the envelope a handler *would* have written without spawning a
 * child process and parsing its stdout.
 */
export interface ProviderSinks {
  /** Sink for reply envelopes. Default: one JSON line on `process.stdout`. */
  write?: (envelope: Envelope) => void;
  /** Sink for operator-facing crash reports. Default: `console.error`. */
  logError?: (message: string, error: unknown) => void;
}

/** The `error` arm of {@link Envelope} — the only reply shape that carries a `code`. */
export type ErrorEnvelope = Extract<Envelope, { type: "error" }>;

/**
 * The message on an unexpected throw — the same shape the HTTP adapter reports,
 * so the two transports describe an identical failure identically.
 */
function crashMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Turn a caught throw into the reply envelope it deserves.
 *
 * A {@link ProviderError} is a deliberate, coded refusal of a request the
 * provider cannot honestly serve (§E1) and keeps its own `code`. Anything else
 * is an unexpected crash: it becomes `internal` and is reported to `logError`,
 * because the operator still needs to see the real stack even though the host
 * only gets a code (`SPEC.md` §11 R1).
 */
export function errorEnvelopeFor(
  error: unknown,
  context: string,
  logError: (message: string, error: unknown) => void,
): ErrorEnvelope {
  if (error instanceof ProviderError) {
    return error.code !== undefined
      ? { type: "error", message: error.message, code: error.code }
      : { type: "error", message: error.message };
  }
  logError(`contextgraph: unexpected error in ${context}`, error);
  return { type: "error", code: "internal", message: crashMessage(error) };
}

/**
 * Whether a decoded value is a JSON object — the only shape a CGP envelope, or
 * a `query`/`request` payload inside one, can legally take. `JSON.parse`
 * happily returns `42`, `"x"`, `[]`, `null` and `true`, none of which carry a
 * `type`; a well-formed envelope can also arrive with its payload missing.
 * Both are the host's mistake, so they earn `bad_request` rather than a crash
 * or (worse) silence (`SPEC.md` §11 R1).
 */
export function isJsonObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function badRequest(message: string, id: string | undefined): ErrorEnvelope {
  const reply: ErrorEnvelope = { type: "error", code: "bad_request", message };
  if (id !== undefined) reply.id = id;
  return reply;
}

/**
 * The correlation `id` on an envelope, if it carries a usable one.
 *
 * `query` declares `id` in its type; `verify` does not (the wire schema has no
 * `id` on it today). Reading it structurally rather than by type means a host
 * that *does* pipeline verify still gets its id echoed back on an error reply,
 * instead of an uncorrelated envelope it cannot match to a request.
 */
function correlationId(envelope: Envelope): string | undefined {
  const id = (envelope as { id?: unknown }).id;
  return typeof id === "string" ? id : undefined;
}

/**
 * Run `provider` as a stdio child process, the shape the reference host and the
 * conformance suite drive. Reads one envelope per line from stdin and writes
 * one envelope per line to stdout until a `shutdown` (or EOF).
 */
export function runStdioProvider(provider: Provider): void {
  const rl = readline.createInterface({ input: process.stdin, terminal: false });

  // Serialize line handling: readline emits buffered lines synchronously in a
  // single tick, but `handleLine` is async (a `query`/`verify` handler suspends
  // at its `await` before writing its reply). Chaining each line's handler
  // after the previous one's promise ensures a later `shutdown` line only runs
  // — and calls `process.exit(0)` — after all prior replies have been written,
  // instead of racing ahead of their pending microtasks (which pipelined hosts
  // rely on, since this SDK advertises `correlation: true`).
  let queue: Promise<void> = Promise.resolve();
  rl.on("line", (line: string) => {
    // Keep the chain alive even if a handler rejects, so a single failing line
    // never drops the replies for lines that follow it. `handleLine` already
    // answers every provider throw with an `error` envelope, so reaching this
    // catch means the *runtime* failed (a closed stdout, say) — report it to
    // stderr rather than discarding it, which is how this catch used to turn a
    // provider crash into silence the host waited on forever.
    queue = queue.then(() => handleLine(provider, line)).catch((error: unknown) => {
      console.error("contextgraph: failed to handle a line", error);
    });
  });
  // EOF / a broken pipe means the host is gone; exit cleanly.
  rl.on("close", () => process.exit(0));
}

/**
 * Handle exactly one input line and write at most one reply.
 *
 * Exported as the runtime's testable seam: {@link runStdioProvider} is a
 * process-wide affair (it owns stdin, stdout and `process.exit`), so the only
 * way to assert on what a provider *replies* without spawning a child process
 * is to drive one line at a time with the sinks redirected. `sinks.write`
 * collects the reply envelope; `sinks.logError` collects what would otherwise
 * go to stderr. Both default to the real streams, so production behaviour is
 * unchanged by the seam's existence.
 *
 * It never rejects for a fault inside `provider`: every throw — expected or
 * not — becomes a reply. That is the §11 R1 contract, and it is what makes a
 * host's correlation slot always resolve.
 */
export async function handleLine(
  provider: Provider,
  line: string,
  sinks: ProviderSinks = {},
): Promise<void> {
  const write = sinks.write ?? writeEnvelope;
  const logError = sinks.logError ?? ((message, error) => console.error(message, error));

  const trimmed = line.trim();
  if (trimmed.length === 0) return;

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    // A malformed line: a robust provider stays alive and says so with a code
    // rather than crashing (the `malformed-input-tolerance` guarantee).
    write({
      type: "error",
      code: "bad_request",
      message: "line was not a valid CGP envelope",
    });
    return;
  }

  // A line that parses but is not an object (`42`, `"x"`, `[]`, `null`, `true`)
  // has no `type` to dispatch on. Ignoring it silently leaves a host that sent
  // a mangled envelope waiting; say `bad_request` instead.
  if (!isJsonObject(parsed)) {
    write({
      type: "error",
      code: "bad_request",
      message: "line was not a CGP envelope object",
    });
    return;
  }
  const envelope = parsed as Envelope;

  switch (envelope.type) {
    case "handshake": {
      // `info()` and `capabilities()` are provider code too, so they can throw.
      // An unanswered handshake strands the host before the session even
      // starts, so it gets the same treatment as a failed query.
      let reply: Envelope;
      try {
        reply = {
          type: "handshake_ack",
          protocol_version: PROTOCOL_VERSION,
          provider: provider.info(),
          capabilities: provider.capabilities(),
        };
      } catch (error) {
        reply = errorEnvelopeFor(error, "provider.info()/capabilities()", logError);
      }
      write(reply);
      break;
    }

    case "query": {
      const queryId = envelope.id;
      // A `query` envelope with no `query` payload is malformed, not a provider
      // fault: answer `bad_request` instead of handing `undefined` to the
      // provider and reporting its resulting crash as `internal`.
      if (!isJsonObject(envelope.query)) {
        write(badRequest("query envelope is missing its `query` payload", queryId));
        break;
      }
      let reply: Envelope;
      try {
        const result = await provider.query(envelope.query);
        reply = { type: "frames", result };
      } catch (error) {
        // Every throw becomes a reply: a ProviderError keeps its own code (§E1),
        // anything else is answered `internal` and logged to stderr. Rethrowing
        // here used to reach `runStdioProvider`'s catch and vanish, leaving the
        // host correlating an id that never came back (`SPEC.md` §11 R1).
        reply = errorEnvelopeFor(error, "provider.query()", logError);
      }
      // Echo the correlation id so the host can match reply to request (H4).
      if (envelope.id !== undefined) reply.id = envelope.id;
      write(reply);
      break;
    }

    case "verify": {
      const verifyId = correlationId(envelope);
      if (!isJsonObject(envelope.request) || !Array.isArray(envelope.request.frames)) {
        write(
          badRequest(
            "verify envelope is missing its `request.frames` payload",
            verifyId,
          ),
        );
        break;
      }
      let reply: Envelope;
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
        reply = { type: "verified", response };
      } catch (error) {
        // Identical stuck-host outcome to a failing query, so identical answer.
        reply = errorEnvelopeFor(error, "provider.verify()", logError);
        // `verified` carries no `id` in the wire schema, but an error reply
        // does — echo one if the host sent it (H4).
        if (verifyId !== undefined) reply.id = verifyId;
      }
      write(reply);
      break;
    }

    case "shutdown":
      process.exit(0);
      break;

    default:
      // handshake_ack / frames / verified / error are host→provider-invalid
      // inputs; a provider ignores them.
      break;
  }
}
