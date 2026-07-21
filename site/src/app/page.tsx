"use client";

import Link from "next/link";
import { useState } from "react";
import ContextReceiptDemo from "./_components/ContextReceiptDemo";

const guarantees = [
  {
    key: "provenance",
    number: "01",
    title: "Provenance",
    short: "Know where every frame came from.",
    detail:
      "Frames carry origin URIs, ranges, digests, derivation methods, and producer identity. Context becomes evidence with a lineage—not anonymous text.",
    contract: "ContextFrame.provenance",
  },
  {
    key: "budget",
    number: "02",
    title: "Budget honesty",
    short: "Token cost is a hard contract.",
    detail:
      "Every query declares a maximum token budget. The host audits returned frames and drops a provider’s output loudly when its claimed costs exceed the allocation.",
    contract: "ContextQuery.max_tokens",
  },
  {
    key: "consent",
    number: "03",
    title: "Consent",
    short: "Egress is disclosed before data moves.",
    detail:
      "Providers declare whether they read, write, persist, or send data off-machine. Remote access is gated by named, revocable consent before the query payload leaves the host.",
    contract: "Capability.data_flow",
  },
  {
    key: "conformance",
    number: "04",
    title: "Conformance",
    short: "Compatibility is tested, not claimed.",
    detail:
      "A public adversarial suite checks frame validity, budget behavior, version compatibility, and failure handling. Providers can prove interoperability in CI.",
    contract: "contextgraph-conformance",
  },
  {
    key: "citation",
    number: "05",
    title: "Citation",
    short: "Human-readable references travel with context.",
    detail:
      "Every frame has a stable identity, title, and citation label. Hosts do not have to reverse-engineer a useful reference after the model has already responded.",
    contract: "ContextFrame.citation_label",
  },
  {
    key: "stability",
    number: "06",
    title: "Version stability",
    short: "Evolve without a flag day.",
    detail:
      "Capabilities and protocol families make optional behavior discoverable and let implementations move from draft to stable without breaking every deployed provider.",
    contract: "contextgraph/1.0-draft",
  },
  {
    key: "temporal",
    number: "07",
    title: "Temporal validity",
    short: "Truth is allowed to change.",
    detail:
      "Validity windows distinguish what was true in the represented world from when a provider learned it. Historical queries stop stale facts from masquerading as current truth.",
    contract: "valid_from · valid_until",
  },
] as const;

const graphNodes = [
  {
    id: "task",
    label: "Task",
    meta: "change checkout flow",
    detail: "The current goal anchors retrieval and sets the context budget.",
  },
  {
    id: "code",
    label: "Code",
    meta: "checkout/service.rs",
    detail: "A symbol frame points to exact source ranges and carries a digest.",
  },
  {
    id: "rule",
    label: "Directive",
    meta: "integration coverage",
    detail: "Confirmed repository steering applies to this change and is cited by ID.",
  },
  {
    id: "memory",
    label: "Memory",
    meta: "prior migration failure",
    detail: "A bounded historical episode informs the task without pretending to be policy.",
  },
  {
    id: "evidence",
    label: "Evidence",
    meta: "test result · sha256",
    detail: "Verification supports or challenges a claim through an immutable locator.",
  },
] as const;

const timeStates = [
  {
    label: "Mar 04",
    query: "valid_at: 2026-03-04",
    fact: "Deploy region: us-east-1",
    state: "valid",
    note: "The original fact is applicable and known to the provider.",
    oldWidth: "100%",
    newWidth: "0%",
  },
  {
    label: "Apr 18",
    query: "valid_at: 2026-04-18",
    fact: "Migration in progress",
    state: "transition",
    note: "Both revisions remain visible in lineage, but their validity windows do not overlap.",
    oldWidth: "58%",
    newWidth: "42%",
  },
  {
    label: "May 12",
    query: "valid_at: 2026-05-12",
    fact: "Deploy region: us-west-2",
    state: "valid",
    note: "The superseding fact is current. The earlier claim remains reconstructable, not deleted.",
    oldWidth: "0%",
    newWidth: "100%",
  },
] as const;

const lifecycle = [
  {
    label: "Observe",
    noun: "Observation",
    copy: "A correction, validator result, Git change, or repeated behavior is recorded without instruction authority.",
  },
  {
    label: "Ground",
    noun: "Evidence",
    copy: "Trace spans, file hashes, tests, and user feedback establish why the observation should be trusted.",
  },
  {
    label: "Propose",
    noun: "Record proposal",
    copy: "Repeated evidence may suggest knowledge, a directive, or an artifact-contract amendment—but remains inert.",
  },
  {
    label: "Govern",
    noun: "Promotion event",
    copy: "Solo, team, or regulated policy decides whether the proposal becomes advisory, confirmed, published, or rejected.",
  },
  {
    label: "Apply",
    noun: "Compiled frame",
    copy: "The selected revision enters one bounded, deterministic invocation frame with manifest-backed lineage.",
  },
  {
    label: "Assess",
    noun: "Outcome",
    copy: "Tests, contract validation, and user response measure whether using that context helped—without rewriting history.",
  },
] as const;

function Arrow() {
  return <span aria-hidden="true">↗</span>;
}

export default function Home() {
  const [activeNode, setActiveNode] = useState(0);
  const [activeGuarantee, setActiveGuarantee] = useState(0);
  const [activeTime, setActiveTime] = useState(2);
  const [activeLifecycle, setActiveLifecycle] = useState(0);

  const graph = graphNodes[activeNode];
  const guarantee = guarantees[activeGuarantee];
  const time = timeStates[activeTime];
  const stage = lifecycle[activeLifecycle];

  return (
    <div className="marketing-page">
      <a className="skip-link" href="#main-content">Skip to content</a>
      <header className="site-header">
        <a className="brand" href="#top" aria-label="Context Graph Protocol home">
          <span className="brand-mark" aria-hidden="true">
            <span />
          </span>
          <span>Context Graph</span>
        </a>
        <nav className="nav-links" aria-label="Primary navigation">
          <a href="#problem">Problem</a>
          <a href="#protocol">Protocol</a>
          <a href="#receipt">Receipt</a>
          <a href="#architecture">Architecture</a>
          <a href="#future">Future</a>
          <Link href="/docs">Docs</Link>
        </nav>
        <a
          className="header-cta"
          href="https://github.com/macanderson/context-graph-protocol"
          target="_blank"
          rel="noreferrer"
        >
          GitHub <Arrow />
        </a>
      </header>

      <main id="main-content">
      <section className="hero section-grid" id="top">
        <div className="hero-noise" aria-hidden="true" />
        <div className="hero-copy">
          <div className="eyebrow light">
            <span className="status-dot" />
            Open protocol · contextgraph/1.0-draft
          </div>
          <h1>
            Context you can
            <br />
            <span>account for.</span>
          </h1>
          <p className="hero-lede">
            AI agents run on context. Today that context is usually an opaque pile of text. Context
            Graph Protocol turns it into typed, budgeted, cited, time-aware evidence—with consent
            and conformance built into the wire.
          </p>
          <div className="hero-actions">
            <a className="button button-light" href="#protocol">
              Explore the protocol <span aria-hidden="true">↓</span>
            </a>
            <Link
              className="button button-ghost-dark"
              href="/docs"
            >
              Read the specification <Arrow />
            </Link>
          </div>
          <div className="hero-proof" aria-label="Protocol facts">
            <div>
              <strong>3</strong>
              <span>Rust crates</span>
            </div>
            <div>
              <strong>7</strong>
              <span>wire guarantees</span>
            </div>
            <div>
              <strong>2</strong>
              <span>permissive licenses</span>
            </div>
          </div>
        </div>

        <div className="hero-visual" aria-label="Interactive context graph">
          <div className="graph-orbit" aria-hidden="true" />
          <div className="graph-lines" aria-hidden="true">
            <span className="line line-a" />
            <span className="line line-b" />
            <span className="line line-c" />
            <span className="line line-d" />
          </div>
          <div className="graph-center">
            <span>ContextFrame</span>
            <strong>4,096</strong>
            <small>token budget</small>
          </div>
          {graphNodes.map((node, index) => (
            <button
              type="button"
              className={`graph-node node-${index + 1} ${activeNode === index ? "active" : ""}`}
              key={node.id}
              onClick={() => setActiveNode(index)}
              aria-pressed={activeNode === index}
            >
              <span>{node.label}</span>
              <small>{node.meta}</small>
            </button>
          ))}
          <div className="graph-readout" aria-live="polite">
            <span>{graph.label}</span>
            <p>{graph.detail}</p>
          </div>
        </div>
      </section>

      <section className="trust-bar" aria-label="Protocol attributes">
        <span>MIT OR Apache-2.0</span>
        <span>In-process · stdio · HTTP</span>
        <span>Language-neutral JSON wire</span>
        <span>Local-first by design</span>
      </section>

      <section className="section section-grid" id="problem">
        <div className="section-intro sticky-intro">
          <div className="eyebrow">01 · The problem</div>
          <h2>Your agent has a context supply chain. It just cannot see it.</h2>
          <p>
            Search results, memories, documentation, policies, and code snippets all enter the
            model through the same untyped channel. Once flattened into a prompt, origin,
            authority, freshness, cost, and consent disappear.
          </p>
        </div>
        <div className="problem-content">
          <div className="comparison-panel blob-panel">
            <div className="panel-heading">
              <span className="mini-label">Before</span>
              <strong>The blob pipe</strong>
            </div>
            <div className="blob-flow" aria-label="Opaque context retrieval flow">
              <div className="source-stack">
                <span>vector search</span>
                <span>grep</span>
                <span>memory</span>
                <span>remote API</span>
              </div>
              <span className="flow-arrow" aria-hidden="true">→</span>
              <div className="blob-object">
                <span>raw text</span>
                <span>?</span>
                <span>?</span>
              </div>
              <span className="flow-arrow" aria-hidden="true">→</span>
              <div className="model-box">MODEL</div>
            </div>
            <ul className="failure-list">
              <li><span>01</span>No enforceable token budget</li>
              <li><span>02</span>No reliable provenance or citation</li>
              <li><span>03</span>No record of what left the machine</li>
              <li><span>04</span>No temporal truth or revision lineage</li>
              <li><span>05</span>No way to measure whether context helped</li>
            </ul>
          </div>

          <div className="comparison-panel graph-panel">
            <div className="panel-heading">
              <span className="mini-label inverted">After</span>
              <strong>The context graph</strong>
            </div>
            <div className="typed-flow" aria-label="Accountable context retrieval flow">
              <div className="provider-chip">Provider</div>
              <span className="typed-connector">capability</span>
              <div className="query-chip">Budgeted query</div>
              <span className="typed-connector">frames</span>
              <div className="frame-chip">
                <span>typed</span>
                <span>cited</span>
                <span>verified</span>
              </div>
              <span className="typed-connector">compose</span>
              <div className="model-chip">MODEL</div>
            </div>
            <p className="panel-quote">
              “Where did this come from, was it allowed, how much did it cost, and was it valid?”
              become protocol questions with machine-checkable answers.
            </p>
          </div>
        </div>
      </section>

      <section className="section dark-section" id="protocol">
        <div className="section-shell">
          <div className="section-intro wide-intro">
            <div className="eyebrow light">02 · The contract</div>
            <h2>Seven guarantees. One accountable frame.</h2>
            <p>
              These guarantees reinforce each other. Provenance without consent can still leak
              data. Budgeting without conformance still relies on trust. The protocol makes the
              combination the unit of interoperability.
            </p>
          </div>

          <div className="guarantee-explorer">
            <div className="guarantee-list" role="group" aria-label="Protocol guarantees">
              {guarantees.map((item, index) => (
                <button
                  type="button"
                  key={item.key}
                  aria-pressed={activeGuarantee === index}
                  className={activeGuarantee === index ? "active" : ""}
                  onClick={() => setActiveGuarantee(index)}
                >
                  <span>{item.number}</span>
                  <strong>{item.title}</strong>
                  <small>{item.short}</small>
                </button>
              ))}
            </div>
            <div className="guarantee-detail" aria-live="polite">
              <div className="detail-index">{guarantee.number}</div>
              <div>
                <span className="mini-label inverted">Wire guarantee</span>
                <h3>{guarantee.title}</h3>
                <p>{guarantee.detail}</p>
                <code>{guarantee.contract}</code>
              </div>
              <div className="assurance-rings" aria-hidden="true">
                <span />
                <span />
                <span />
                <i />
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="section section-grid wire-section">
        <div className="section-intro sticky-intro">
          <div className="eyebrow">03 · The wire surface</div>
          <h2>Three shapes carry the protocol.</h2>
          <p>
            A provider describes its behavior before seeing data. A host sends a bounded request.
            The provider returns inspectable frames that remain untrusted content until the host
            validates and composes them.
          </p>
        </div>
        <div className="wire-flow">
          <article className="wire-card">
            <div className="wire-number">01</div>
            <div>
              <span className="mini-label">Handshake</span>
              <h3>Capability</h3>
              <p>Identity, version, supported frame kinds, transport limits, and declared data flow.</p>
              <pre><code>{`data_flow: {
  reads: true,
  writes: false,
  egress: false
}`}</code></pre>
            </div>
          </article>
          <div className="wire-arrow"><span>declare before disclosure</span>↓</div>
          <article className="wire-card">
            <div className="wire-number">02</div>
            <div>
              <span className="mini-label">Request</span>
              <h3>Query</h3>
              <p>A goal, anchors, desired kinds, temporal filters, frame limits, and a hard token ceiling.</p>
              <pre><code>{`goal: "change checkout flow"
anchors: ["CheckoutService"]
max_frames: 12
max_tokens: 4096`}</code></pre>
            </div>
          </article>
          <div className="wire-arrow"><span>retrieve within contract</span>↓</div>
          <article className="wire-card featured-wire">
            <div className="wire-number">03</div>
            <div>
              <span className="mini-label inverted">Response</span>
              <h3>ContextFrame</h3>
              <p>Typed content plus relevance, token cost, provenance, citations, and temporal validity.</p>
              <div className="frame-anatomy">
                <span><b>kind</b> symbol</span>
                <span><b>score</b> 0.94</span>
                <span><b>tokens</b> 382</span>
                <span><b>digest</b> sha256:8f…</span>
                <span><b>valid</b> current</span>
                <span><b>source</b> L120–L184</span>
              </div>
            </div>
          </article>
        </div>
      </section>

      <section className="section temporal-section">
        <div className="section-shell">
          <div className="section-intro wide-intro dark-text">
            <div className="eyebrow">04 · Time-aware context</div>
            <h2>A fact is not timeless just because it was embedded.</h2>
            <p>
              Context Graph Protocol preserves revision lineage and validity windows. Future
              lifecycle work adds explicit origin-observation and provider-receipt semantics so a
              host can reconstruct both what was true and what it knew at the time.
            </p>
          </div>

          <div className="temporal-explorer">
            <div className="time-controls" role="group" aria-label="Historical query date">
              {timeStates.map((item, index) => (
                <button
                  type="button"
                  key={item.label}
                  aria-pressed={activeTime === index}
                  className={activeTime === index ? "active" : ""}
                  onClick={() => setActiveTime(index)}
                >
                  <span>{String(index + 1).padStart(2, "0")}</span>
                  {item.label}
                </button>
              ))}
            </div>
            <div className="timeline-stage" aria-live="polite">
              <div className="timeline-ruler">
                <span>MAR</span><span>APR</span><span>MAY</span><span>JUN</span>
              </div>
              <div className="timeline-track old-track">
                <span className="track-label">us-east-1 · revision 01</span>
                <i style={{ width: time.oldWidth }} />
              </div>
              <div className="timeline-track new-track">
                <span className="track-label">us-west-2 · revision 02</span>
                <i style={{ width: time.newWidth }} />
              </div>
              <div className="query-marker" style={{ left: `${20 + activeTime * 30}%` }}>
                <span>{time.label}</span>
              </div>
              <div className="timeline-result">
                <div>
                  <span className="mini-label">Historical result</span>
                  <strong>{time.fact}</strong>
                </div>
                <code>{time.query}</code>
                <p>{time.note}</p>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="section taxonomy-section">
        <div className="section-shell">
          <div className="section-intro wide-intro dark-text">
            <div className="eyebrow">05 · A vocabulary agents can reason over</div>
            <h2>Not everything remembered should become an instruction.</h2>
            <p>
              The lifecycle profile keeps episodes, claims, steering, and proof separate. That
              boundary is the defense against an agent silently converting “this happened” into
              “always do this.”
            </p>
          </div>
          <div className="taxonomy-grid">
            <article>
              <span className="taxonomy-symbol">○</span>
              <small>What was detected</small>
              <h3>Observation</h3>
              <p>An occurrence or interpretation with no truth or instruction authority.</p>
              <code>immutable event</code>
            </article>
            <article>
              <span className="taxonomy-symbol">◇</span>
              <small>What is believed</small>
              <h3>Knowledge</h3>
              <p>A fact, provisional assumption, or recorded architectural decision.</p>
              <code>fact · assumption · decision</code>
            </article>
            <article>
              <span className="taxonomy-symbol">◌</span>
              <small>What happened</small>
              <h3>Memory</h3>
              <p>A bounded episode or summary—not automatically current truth.</p>
              <code>episode · summary</code>
            </article>
            <article>
              <span className="taxonomy-symbol">▰</span>
              <small>How to behave</small>
              <h3>Directive</h3>
              <p>Governed steering with explicit scope, authority, and enforcement.</p>
              <code>preference · rule · constraint · procedure</code>
            </article>
            <article>
              <span className="taxonomy-symbol">⌁</span>
              <small>Why trust it</small>
              <h3>Evidence</h3>
              <p>An addressable source that supports or challenges another record.</p>
              <code>locator · digest · relation</code>
            </article>
            <article>
              <span className="taxonomy-symbol">✓</span>
              <small>What completion means</small>
              <h3>Artifact contract</h3>
              <p>A versioned, machine-checkable definition of an acceptable deliverable.</p>
              <code>requirements · validation</code>
            </article>
          </div>
        </div>
      </section>

      <section className="section lifecycle-section">
        <div className="section-shell">
          <div className="section-intro wide-intro dark-text">
            <div className="eyebrow">06 · The governed learning loop</div>
            <h2>Agents can improve without promoting their own guesses to policy.</h2>
            <p>
              Evidence can move through a reviewable promotion ladder, enter a compiled context
              frame, and be evaluated against real outcomes. Every transition is an event, not an
              opaque mutation.
            </p>
          </div>
          <div className="lifecycle-explorer">
            <div className="lifecycle-track" role="group" aria-label="Adaptive context lifecycle">
              {lifecycle.map((item, index) => (
                <button
                  type="button"
                  key={item.label}
                  aria-pressed={activeLifecycle === index}
                  className={activeLifecycle === index ? "active" : ""}
                  onClick={() => setActiveLifecycle(index)}
                >
                  <i>{String(index + 1).padStart(2, "0")}</i>
                  <span>{item.label}</span>
                </button>
              ))}
            </div>
            <div className="lifecycle-detail" aria-live="polite">
              <div className="lifecycle-glyph" aria-hidden="true">
                <span>{String(activeLifecycle + 1).padStart(2, "0")}</span>
              </div>
              <div>
                <span className="mini-label inverted">{stage.label}</span>
                <h3>{stage.noun}</h3>
                <p>{stage.copy}</p>
              </div>
            </div>
          </div>
          <div className="governance-split">
            <article>
              <span>Solo</span>
              <code>observation → advisory → keep / edit / ignore</code>
              <p>Low-friction learning, local by default, with explicit confirmation for blocking behavior.</p>
            </article>
            <article>
              <span>Team</span>
              <code>proposal → owner review → repository publication</code>
              <p>Shared steering gains accountable identity, review history, and repository scope.</p>
            </article>
            <article>
              <span>Regulated</span>
              <code>proposal → policy gate → signed publication</code>
              <p>RBAC, attestations, retention commitments, and auditable organization policy.</p>
            </article>
          </div>
        </div>
      </section>

      <ContextReceiptDemo />

      <section className="section architecture-section" id="architecture">
        <div className="section-shell">
          <div className="section-intro wide-intro">
            <div className="eyebrow">08 · The ecosystem</div>
            <h2>Open mechanism. Independent hosts. Optional enterprise control.</h2>
            <p>
              The protocol is deliberately smaller than the products built around it. Hosts own
              learning and prompt policy. Providers own retrieval. Enterprise platforms may add
              governance without becoming a runtime dependency.
            </p>
          </div>
          <div className="ecosystem-map" aria-label="Stella, Context Graph Protocol, providers, and Oxagen architecture">
            <div className="ecosystem-column local-column">
              <span className="column-kicker">Open-source host</span>
              <h3>Stella</h3>
              <p>Local/BYOK agent, code graph, context compiler, trace mining, governance, contracts.</p>
              <ul>
                <li>No account required</li>
                <li>Local SQLite stores</li>
                <li>Git-native repository rules</li>
              </ul>
            </div>
            <div className="protocol-spine">
              <span>QUERY</span>
              <div>
                <small>open wire contract</small>
                <strong>Context Graph Protocol</strong>
                <code>capability · query · frame · lifecycle</code>
              </div>
              <span>FRAME</span>
            </div>
            <div className="ecosystem-column provider-column">
              <span className="column-kicker">Provider ecosystem</span>
              <h3>Any provider</h3>
              <p>Code search, documentation, memory, policy, data catalogs, or specialized graphs.</p>
              <ul>
                <li>In-process, stdio, or HTTP</li>
                <li>Independent implementation</li>
                <li>Conformance-tested</li>
              </ul>
            </div>
            <div className="ecosystem-column enterprise-column">
              <span className="column-kicker">Optional control plane</span>
              <h3>Oxagen</h3>
              <p>Hosted workspaces, RBAC, organization policy, encrypted sync, audit, and integrations.</p>
              <ul>
                <li>Commercial—not protocol-required</li>
                <li>Multi-user governance</li>
                <li>Enterprise operations</li>
              </ul>
            </div>
          </div>

          <div className="mcp-comparison">
            <div>
              <span className="mini-label">Complementary protocols</span>
              <h3>MCP connects actions. Context Graph connects evidence.</h3>
            </div>
            <div className="mcp-lanes">
              <div><strong>Context Graph</strong><span>retrieve → verify → compose</span><i>prompt context</i></div>
              <div><strong>MCP</strong><span>discover → call → return</span><i>tools &amp; actions</i></div>
            </div>
          </div>
        </div>
      </section>

      <section className="section adoption-section">
        <div className="section-shell">
          <div className="section-intro wide-intro dark-text">
            <div className="eyebrow">09 · Why adopt it</div>
            <h2>Build the context layer once. Keep your choices open.</h2>
          </div>
          <div className="adoption-grid">
            <article>
              <span>For agent builders</span>
              <h3>Compose retrieval without hard-coding every backend.</h3>
              <p>One host interface for local code, memory, docs, policies, and remote providers—with budgets and citations that compose.</p>
              <ul><li>Provider portability</li><li>Deterministic prompt inputs</li><li>Auditable failures</li></ul>
            </article>
            <article>
              <span>For provider builders</span>
              <h3>Ship one integration that any conforming host can understand.</h3>
              <p>Declare capabilities, accept typed queries, return frames, and prove behavior with the conformance suite.</p>
              <ul><li>Stable JSON surface</li><li>CI-verifiable compatibility</li><li>No host-specific plugin maze</li></ul>
            </article>
            <article>
              <span>For platform teams</span>
              <h3>Make context movement visible before scaling agents.</h3>
              <p>Consent, provenance, scope, temporal validity, and retention become explicit integration boundaries.</p>
              <ul><li>Data-flow governance</li><li>Historical reconstruction</li><li>Vendor-neutral controls</li></ul>
            </article>
          </div>
        </div>
      </section>

      <section className="section future-section" id="future">
        <div className="section-shell">
          <div className="section-intro wide-intro">
            <div className="eyebrow">10 · What comes next</div>
            <h2>From accountable retrieval to accountable learning.</h2>
            <p>
              The protocol works today for typed retrieval. The roadmap extends the same
              provenance and conformance discipline to lifecycle exchange—without moving host
              governance into the wire.
            </p>
          </div>
          <div className="roadmap">
            <article className="roadmap-now">
              <span className="roadmap-state">Available now</span>
              <h3>Accountable retrieval</h3>
              <ul>
                <li>Capability negotiation</li>
                <li>Budgeted context queries</li>
                <li>Typed frames and citations</li>
                <li>Consent-gated egress</li>
                <li>Host runtime and conformance suite</li>
              </ul>
            </article>
            <article>
              <span className="roadmap-state">Lifecycle profile</span>
              <h3>Durable context exchange</h3>
              <ul>
                <li>Immutable typed context records</li>
                <li>Observed and valid-time semantics</li>
                <li>Full, compact, and reference frames</li>
                <li>Append receipts and idempotency</li>
                <li>Record rehydration and feedback</li>
              </ul>
            </article>
            <article>
              <span className="roadmap-state">Ecosystem future</span>
              <h3>Context as infrastructure</h3>
              <ul>
                <li>Provider SDKs beyond Rust</li>
                <li>Portable artifact contracts</li>
                <li>Signed organization policy</li>
                <li>Optional synchronization profile</li>
                <li>Independent conformance certification</li>
              </ul>
            </article>
          </div>
          <p className="roadmap-note">
            Roadmap items are directional design work, not promises of shipped behavior. Continuous
            synchronization requires a separate profile for cursors, change feeds, tombstones,
            conflicts, acknowledgements, and offline replay.
          </p>
        </div>
      </section>

      <section className="closing-section">
        <div className="closing-grid" aria-hidden="true" />
        <div className="closing-mark" aria-hidden="true"><span /></div>
        <div className="eyebrow light">The context layer should be open</div>
        <h2>Help define the protocol agents will reason over.</h2>
        <p>
          Read the draft. Build a provider. Run conformance. Challenge the semantics. The goal is
          not another private memory silo—it is a context ecosystem any host can inspect and trust.
        </p>
        <div className="closing-actions">
          <a
            className="button button-light"
            href="https://github.com/macanderson/context-graph-protocol"
            target="_blank"
            rel="noreferrer"
          >
            Star and contribute on GitHub <Arrow />
          </a>
          <a
            className="button button-ghost-dark"
            href="https://github.com/macanderson/context-graph-protocol/blob/main/docs/implementing-a-provider.md"
            target="_blank"
            rel="noreferrer"
          >
            Build a provider <Arrow />
          </a>
        </div>
      </section>
      </main>

      <footer>
        <a className="brand footer-brand" href="#top">
          <span className="brand-mark dark-mark" aria-hidden="true"><span /></span>
          <span>Context Graph Protocol</span>
        </a>
        <p>Open context infrastructure for accountable agents.</p>
        <div>
          <a href="https://github.com/macanderson/context-graph-protocol" target="_blank" rel="noreferrer">GitHub</a>
          <a href="https://github.com/macanderson/context-graph-protocol/tree/main/docs" target="_blank" rel="noreferrer">Docs</a>
          <span>MIT OR Apache-2.0</span>
        </div>
      </footer>
    </div>
  );
}
