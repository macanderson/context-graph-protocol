# Design sketch — query filters

> **Status: sketch, not specification.** `QueryCapability.filters` was removed
> from the `contextgraph/1.0` surface by
> [ADR 0004](../adr/0004-dead-capability-surface.md): a provider could *declare*
> filters at handshake, but `ContextQuery` had no field with which to *send*
> one, so the capability was unreachable by construction. Nothing here is
> normative.

## What was wrong with keeping it

Specifying a filter grammar is not a small job — it means deciding key
namespacing, value escaping, whether multiple filters conjoin or disjoin, and
what a provider does with a key it does not recognise. Every one of those is a
decision better made by someone who has to live with the result. Freezing a
guess would have been worse than freezing nothing.

## The forcing function

The reference providers in issue #18 — ripgrep snippets, tree-sitter symbols,
git history — are the intended consumer. `kinds`, `anchors`, and `query_text`
are expected to carry them. If writing those providers proves filters genuinely
necessary (the likely candidate is a path restriction: "search only
`crates/foo/**`"), the design lands additively in 1.x with a real implementation
behind it.

## Sketch, if it returns

Named key/value pairs, provider-declared keys, conjunctive:

```jsonc
{
  "type": "query",
  "id": "q-1",
  "query": {
    "goal": "why does the retry loop give up",
    "filters": [
      { "key": "path", "value": "crates/net/**" },
      { "key": "language", "value": "rust" }
    ],
    "max_frames": 8,
    "max_tokens": 2000
  }
}
```

with `QueryCapability.filters` restored as the declaration of which keys a
provider honours.

### The decision that actually matters

**What a provider does with an unrecognised key.** Two defensible answers, and
they are not interchangeable:

- **Ignore it** — resilient, but a host silently gets broader results than it
  asked for, which is the class of silent-wrongness this protocol exists to make
  loud. A host that filters to `path:crates/net/**` for privacy reasons and is
  silently ignored has been failed badly.
- **Reject with `bad_request`** — loud and safe, but brittle: adding a filter
  key to a host breaks every older provider.

The resolution is probably to make it explicit per-query rather than
per-provider — an `unknown_filters: "ignore" | "reject"` selector, defaulting to
`reject`, so the host states its own tolerance instead of guessing the
provider's. That asymmetry (safe default, opt-in looseness) matches how the rest
of the protocol handles unknown values.

Because the capability declaration lists honoured keys, a careful host can also
avoid the question entirely by intersecting its desired filters with the
declared set before sending.
