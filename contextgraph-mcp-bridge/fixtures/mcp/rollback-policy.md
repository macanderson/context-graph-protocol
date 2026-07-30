# Rollback policy

A rollback is triggered automatically when the error rate exceeds 2% over a
five-minute window, or manually by an on-call engineer.

Rollbacks restore the previous known-good release; they never roll forward to a
patched build under incident pressure. The progressive rollout flag is flipped
off first so no new sessions land on the failing release while the previous one
is restored.
