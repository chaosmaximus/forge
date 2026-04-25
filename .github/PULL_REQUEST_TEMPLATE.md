## Summary

<!-- One paragraph: what changes and why (the "why" is what reviewers
     read first). Skip if the PR title already conveys it. -->

## Scope

<!-- Tick every layer this PR touches. The 2P-1b harness-sync CI gate
     reads this to decide whether the protocol_hash interlock applies. -->

- [ ] `crates/core/src/protocol/` (protocol surface — also bump
      `protocol_hash` via `bash scripts/sync-protocol-hash.sh`)
- [ ] `crates/daemon/`
- [ ] `crates/cli/`
- [ ] `.claude-plugin/`
- [ ] `hooks/`, `scripts/hooks/`
- [ ] `skills/`
- [ ] `agents/`
- [ ] `docs/`
- [ ] CI / release tooling
- [ ] Other: ___

## Test plan

<!-- Bulleted checklist that proves the change works.
     Type-checking and tests pass != the feature works.
     Include dogfood steps when the PR touches a user-visible surface. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --features bench -- -W clippy::all -D warnings`
- [ ] `cargo test --workspace`
- [ ] `bash scripts/check-harness-sync.sh`
- [ ] `bash scripts/check-review-artifacts.sh` (if any reviews ship in
      this PR)
- [ ] `bash scripts/check-license-manifest.sh` (if any JSON file is
      added/moved)
- [ ] `bash scripts/check-protocol-hash.sh` (if `request.rs` changed)
- [ ] Dogfood (if user-visible behavior changed): _________________

## Adversarial review

<!-- For waves landing under the P3 production-readiness plan,
     attach the YAML artifact under docs/superpowers/reviews/<slug>.yaml
     and link the transcript. CI's review-artifacts gate enforces no
     open BLOCKER/HIGH findings. -->

- [ ] Review artifact attached at `docs/superpowers/reviews/______.yaml`
- [ ] All BLOCKER + HIGH findings resolved or deferred with rationale
- [ ] Verdict: lockable-as-is | lockable-with-fixes | not-lockable

## Linked issues

<!-- Closes #N, refs #M -->

## Risks / blast radius

<!-- What could break if this lands wrong? Database migrations,
     protocol changes, schema-incompatible changes — call out
     explicitly. Include rollback notes when relevant
     (`docs/operations/2P-1-rollback.md`). -->
