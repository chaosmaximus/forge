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
     Include dogfood steps when the PR touches a user-visible surface.
     The clippy command below matches what CI runs (.github/workflows/
     ci.yml `check` job); a stricter `--workspace --features bench`
     run is recommended locally before push. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy -p forge-daemon -p forge-core -p forge-cli -p forge-hud -- -W clippy::all -D warnings`
- [ ] `cargo test --workspace`
- [ ] `bash scripts/check-harness-sync.sh`
- [ ] `bash scripts/check-review-artifacts.sh` (if any reviews ship in
      this PR)
- [ ] `bash scripts/check-license-manifest.sh` (if any JSON file is
      added/moved)
- [ ] `bash scripts/check-protocol-hash.sh` (if `request.rs` changed —
      run `bash scripts/sync-protocol-hash.sh` first)
- [ ] Dogfood (if user-visible behavior changed): _________________

## Adversarial review

<!-- Required only for PRs landing a wave/phase of the P3 production-
     readiness plan. Skip this section for routine bug fixes / docs /
     deps updates. Reviewers: the W2 review-artifacts CI gate only
     fires on artifacts ALREADY in docs/superpowers/reviews/, so a
     skip-and-merge here doesn't bypass the gate. -->

- [ ] (P3 waves only) Review artifact attached at
      `docs/superpowers/reviews/______.yaml`
- [ ] (P3 waves only) All BLOCKER + HIGH findings resolved or deferred
      with rationale
- [ ] (P3 waves only) Verdict: lockable-as-is | lockable-with-fixes |
      not-lockable
- [ ] Not a P3 wave — adversarial review section N/A

## Linked issues

<!-- Closes #N, refs #M -->

## Risks / blast radius

<!-- What could break if this lands wrong? Database migrations,
     protocol changes, schema-incompatible changes — call out
     explicitly. Include rollback notes when relevant
     (`docs/operations/2P-1-rollback.md`). -->
