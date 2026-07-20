import Image from "next/image";
import Link from "next/link";

export default function Home() {
  return (
    <main className="flex min-h-screen items-center bg-background px-6 py-16 text-foreground sm:px-10">
      <article className="mx-auto w-full max-w-2xl border-t border-border pt-10 sm:pt-14">
        <Image
          src="/contextgraph-logo.svg"
          alt="Context Graph Protocol logo"
          width={112}
          height={112}
          priority
          className="mb-10 size-24 border border-border sm:size-28"
        />

        <p className="mb-4 font-mono text-xs uppercase tracking-[0.16em] text-muted-foreground">
          Protocol specification · contextgraph/1.0-draft
        </p>
        <h1 className="max-w-xl font-serif text-4xl font-semibold leading-tight tracking-tight sm:text-5xl">
          Context Graph Protocol
        </h1>
        <p className="mt-6 max-w-xl text-base leading-7 text-muted-foreground sm:text-lg sm:leading-8">
          The canonical architecture for building context graphs that agents
          use to reason over.
        </p>

        <nav aria-label="Primary" className="mt-10 border-t border-border pt-5">
          <Link
            href="/docs"
            className="inline-flex items-center gap-2 text-sm font-medium underline decoration-border underline-offset-4 hover:decoration-foreground focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-foreground"
          >
            Read the documentation
            <span aria-hidden="true">→</span>
          </Link>
        </nav>
      </article>
    </main>
  );
}
