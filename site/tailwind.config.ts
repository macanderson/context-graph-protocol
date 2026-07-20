import type { Config } from "tailwindcss";

/**
 * Context Graph Protocol's neutral design system, derived from the near-black ink and white field
 * in assets/contextgraph-logo.svg. Fumadocs' Tailwind v4 preset is imported from
 * src/app/globals.css; these tokens provide stable aliases for site-authored UI.
 */
const config: Config = {
  darkMode: "class",
  content: [
    "./src/**/*.{js,ts,jsx,tsx,md,mdx}",
    "./content/**/*.{md,mdx}",
  ],
  theme: {
    extend: {
      colors: {
        background: "var(--background)",
        foreground: "var(--foreground)",
        primary: {
          DEFAULT: "var(--primary)",
          foreground: "var(--primary-foreground)",
        },
        accent: {
          DEFAULT: "var(--accent)",
          foreground: "var(--accent-foreground)",
        },
        border: "var(--border)",
        muted: {
          DEFAULT: "var(--muted)",
          foreground: "var(--muted-foreground)",
        },
      },
      fontFamily: {
        sans: ["var(--font-body)", "ui-sans-serif", "sans-serif"],
        serif: ["var(--font-academic)", "ui-serif", "Georgia", "serif"],
        mono: ["var(--font-code)", "ui-monospace", "monospace"],
      },
    },
  },
  plugins: [],
};

export default config;
