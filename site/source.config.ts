import { defineConfig, defineDocs } from "fumadocs-mdx/config";

/**
 * Upstream provenance for the MDX copies below: path: "../docs".
 * The active collection remains local so frontmatter and route links can be
 * maintained independently.
 */

/**
 * Protocol documentation collection.
 *
 * Content is kept separate from application code so the source Markdown can be
 * reviewed and versioned like the specification it documents.
 */
export const docs = defineDocs({
  dir: "content/docs",
});

export default defineConfig();
