/**
 * The stdio entrypoint: the host spawns this as a child process and exchanges
 * newline-delimited JSON over stdin/stdout. This is the transport the
 * conformance suite drives by default (`npm run conformance`).
 */
import { runStdioProvider } from "@contextgraphprotocol/typescript-sdk";

import { provider } from "./provider.js";

runStdioProvider(provider);
