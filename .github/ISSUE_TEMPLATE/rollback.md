---
name: Rollback tracking
about: Track a Phase 2P-1 rollback as it executes
title: "[rollback] v0.x.y — <one-line cause>"
labels: ["rollback", "incident"]
assignees: []
---

> Operator: follow `docs/operations/2P-1-rollback.md` step-by-step.
> Update this issue as you go. RTO target: 20 min from decision.

## Cause

<!-- One-paragraph description of the regression that triggered the
     rollback. Include link to the offending commit / PR / release. -->

## Decision rationale

- [ ] Reached production users (binary install via Homebrew /
      cargo install / tarball)
- [ ] Blast radius > rollback cost (memory-corrupting / data-loss /
      security-affecting — NOT cosmetic)

If either is unchecked, the right tool is fix-forward, not rollback.

## Step status

- [ ] Step 0 — declared in this issue + flagged
      `.github/pending-rollback`
- [ ] Step 1 — `gh release delete <TAG> --yes`
- [ ] Step 2 — Homebrew formula reverted (if Formula/forge.rb was
      bumped)
- [ ] Step 3 — public source reverted (`git revert` for forward-revert,
      or `git reset --hard` + `git push --force-with-lease` for the
      destructive Mode B)
- [ ] Step 4 — sideload-user advisory posted
- [ ] Step 5 — post-mortem scheduled, `.github/pending-rollback`
      removed

## Affected versions

| From (bad)     | To (rollback target)            |
|----------------|---------------------------------|
| v<bad-version> | v<previous-known-good-version>  |

DB compatibility: <!-- check matrix in playbook §"DB compatibility matrix" -->

## Post-mortem

<!-- After Step 5: link the post-mortem doc here. Even if root cause
     is obvious, run a post-mortem — missed documentation is usually
     the real root cause. -->
