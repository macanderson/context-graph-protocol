import { docs } from "../../.source/server";
import { loader } from "fumadocs-core/source";

/**
 * Canonical Fumadocs source used by documentation routes, navigation, and
 * search. The generated collection comes from content/docs.
 */
export const source = loader({
  baseUrl: "/docs",
  source: docs.toFumadocsSource(),
});
