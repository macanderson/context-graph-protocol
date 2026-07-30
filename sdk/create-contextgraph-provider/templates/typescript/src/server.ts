/**
 * The HTTP entrypoint: host the same provider behind one POST endpoint (the
 * "streamable HTTP" transport, SPEC.md §3). Point a host — or
 * `contextgraph-inspect http http://127.0.0.1:8787` — at it.
 *
 * `createHttpHandler` reads the raw request body itself, so this plain
 * `node:http` server needs no framework and no body parser. Under Express, mount
 * `app.post("/contextgraph", createHttpHandler(provider))` with no JSON parser
 * on that route; under Fastify, call `respondToEnvelopeBody` with `request.body`.
 */
import { createServer } from "node:http";

import { createHttpHandler } from "@contextgraphprotocol/typescript-sdk";

import { provider } from "./provider.js";

const port = Number(process.env.PORT ?? "8787");
const host = process.env.HOST ?? "127.0.0.1";
const server = createServer(createHttpHandler(provider));
server.listen(port, host, () => {
  process.stdout.write(`{{PROJECT_NAME}} listening on http://${host}:${port}\n`);
});

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
