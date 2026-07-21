import type { Metadata } from "next";
import { RootProvider } from "fumadocs-ui/provider/next";
import { IBM_Plex_Mono, Inter, Source_Serif_4 } from "next/font/google";
import "./globals.css";
import "./marketing.css";

const inter = Inter({
  variable: "--font-body",
  subsets: ["latin"],
  display: "swap",
});

const sourceSerif = Source_Serif_4({
  variable: "--font-academic",
  subsets: ["latin"],
  display: "swap",
});

const ibmPlexMono = IBM_Plex_Mono({
  variable: "--font-code",
  subsets: ["latin"],
  weight: ["400", "500", "600"],
  display: "swap",
});

const canonicalOrigin =
  process.env.NEXT_PUBLIC_SITE_URL ??
  "https://context-graph-protocol.macanderson-mail.chatgpt.site";

export const metadata: Metadata = {
  metadataBase: new URL(canonicalOrigin),
  title: {
    default: "Context Graph Protocol — Accountable context for AI agents",
    template: "%s — Context Graph Protocol",
  },
  description:
    "An open protocol for typed, budgeted, provenance-rich, consent-gated context that AI agents can cite and trust.",
  keywords: [
    "Context Graph Protocol",
    "AI agents",
    "agent context",
    "context engineering",
    "provenance",
    "agent memory",
  ],
  openGraph: {
    title: "Context you can account for.",
    description:
      "Context Graph Protocol turns opaque retrieval into typed, budgeted, cited, time-aware evidence.",
    type: "website",
    images: [
      {
        url: "/og.png",
        width: 1731,
        height: 909,
        alt: "Context Graph Protocol",
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: "Context you can account for.",
    description:
      "An open protocol for typed, budgeted, provenance-rich context for AI agents.",
    images: ["/og.png"],
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      suppressHydrationWarning
      className={`${inter.variable} ${sourceSerif.variable} ${ibmPlexMono.variable} h-full antialiased`}
    >
      <body className="min-h-full flex flex-col">
        <RootProvider
          theme={{
            attribute: "class",
            defaultTheme: "system",
            enableSystem: true,
            disableTransitionOnChange: true,
          }}
        >
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
