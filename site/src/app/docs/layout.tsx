import Image from "next/image";
import type { ReactNode } from "react";
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { source } from "@/lib/source";

const repositoryUrl = "https://github.com/macanderson/context-graph-protocol";

function ProtocolTitle() {
  return (
    <span className="flex items-center gap-2.5 font-medium tracking-tight">
      <Image
        src="/contextgraph-logo.svg"
        alt=""
        aria-hidden="true"
        width={28}
        height={28}
        className="size-7 border border-fd-border"
      />
      <span>Context Graph Protocol</span>
      <span className="hidden border border-fd-border px-1.5 py-0.5 font-mono text-[10px] font-normal tracking-wide text-fd-muted-foreground sm:inline">
        1.0 DRAFT
      </span>
    </span>
  );
}

export default function DocumentationLayout({
  children,
}: Readonly<{ children: ReactNode }>) {
  return (
    <DocsLayout
      tree={source.pageTree}
      githubUrl={repositoryUrl}
      nav={{
        title: <ProtocolTitle />,
        url: "/docs",
        transparentMode: "none",
      }}
      links={[
        {
          text: "Specification",
          url: "/docs/protocol-surface",
          active: "nested-url",
        },
        {
          text: "Research",
          url: "/docs/protocol-advantages",
          active: "nested-url",
        },
      ]}
      sidebar={{
        defaultOpenLevel: 1,
      }}
    >
      {children}
    </DocsLayout>
  );
}
