# Reference corpus for contextgraph-ripgrep

This file is the default search corpus for the `contextgraph-ripgrep`
reference provider. The conformance probe searches it, so it deliberately
mentions the words a probe query uses.

## Conformance

A provider is Context Graph Protocol conformant when the conformance suite is
green against it for its declared capabilities. Running the conformance probe
over this corpus returns snippet frames with real provenance.

## Provenance

Every snippet frame cites a file URI, a line range, and a sha256 digest of the
exact bytes on disk, so a host can re-read and verify the claim.

## Budget honesty

A snippet frame declares an honest token cost, computed from its content bytes,
and the response reports truncation when the budget or frame cap is reached.

## Graph

A snippet frame carries one labelled edge locating the match in its file, so a
graph-aware host can anchor a follow-up query on that file.
