## What does this change?

<!-- One or two sentences. Link the issue: "Fixes #123". -->

## How was it tested?

<!-- Tests added, example flows run, screenshots of Studio changes. -->

## Checklist

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass
- [ ] New logic has tests, and the tests don't move the mouse, type, open windows, send mail or use the network
- [ ] If an action changed: schema and action reference regenerated (`autoflow schema`, `autoflow docs`)
- [ ] New Studio texts use `tr("…")`
- [ ] No passwords or personal data in flows, logs or tests; dry runs change nothing
- [ ] New dependencies (if any) have permissive licences and are explained above
- [ ] User-facing changes are described in the README or docs/GUIDE.md

I agree that this contribution is published under the project's MIT licence.
