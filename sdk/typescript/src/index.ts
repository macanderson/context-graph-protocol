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
export { runStdioProvider, ProviderError, type Provider } from "./provider.js";
export {
  handleEnvelope,
  respondToEnvelopeBody,
  createHttpHandler,
  type EnvelopeHttpResponse,
} from "./http.js";
export {
  ALGORITHM_ED25519,
  digestString,
  encodeProvenanceLink,
  frameCommitment,
  fromHex,
  inclusionProof,
  isValid,
  merkleRoot,
  parseDigest,
  provenanceChainHead,
  rootFromProof,
  toHex,
  verifyCommitment,
  verifyFrameAttestation,
  type AttestableFrame,
  type AttestationVerdict,
  type InclusionProof,
  type InclusionStep,
  type ProvenanceAttestation,
} from "./attest.js";
