# Blocked / Partial Features

Features that cannot be completed without a larger frontend change or a design
decision needing human input. Each entry records what works, what is missing,
and why it is blocked.

## Switch exhaustiveness diagnostics

**Status:** `@unknown default` parses and runs (treated as a catch-all `default`).
Exhaustiveness *checking* — diagnosing a `switch` over an enum that omits cases
without a `default` — is not implemented.

**Why deferred**
Exhaustiveness is a sema/type-checker diagnostic that needs the subject's
resolved enum type, the full case set, and pattern-coverage analysis (including
ranges, tuples, and `where` guards). It is a self-contained but non-trivial sema
feature; the runtime executes partial switches correctly today (an unmatched
value simply falls through), so it is a diagnostics-quality gap rather than a
behavioural one.
