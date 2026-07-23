/**
 * `@contextgraphprotocol/typescript-sdk` — a zero-dependency TypeScript SDK for building
 * conformant Context Graph Protocol providers.
 *
 * ```ts
 * import { runStdioProvider, budgetTokens, type Provider } from "@contextgraphprotocol/typescript-sdk";
 *
 * const provider: Provider = {
 *   info: () => ({ name: "my-provider", version: "0.1.0",
 *     data_flow: { reads: true, writes: false, egress: false, egress_scopes: ["local-only"] } }),
 *   capabilities: () => ({ query: { kinds: ["doc"] }, correlation: true }),
 *   query: () => ({ frames: [], truncated: false }),
 * };
 * runStdioProvider(provider);
 * ```
 */
export * from "./types.js";
export { budgetTokens, BYTES_PER_BUDGET_TOKEN } from "./budget.js";
export { runStdioProvider, type Provider } from "./provider.js";
