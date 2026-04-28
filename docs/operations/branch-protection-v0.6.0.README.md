# Branch protection — `chaosmaximus/forge:master` at v0.6.0

**Status:** Staged for user submission per Plan A locked decision #3
("Marketplace + branch protection deferred to user, after P3-4
lands"). The repo does not have branch protection enforced today;
this is the rule set the audit + release workflow assume but do not
enforce themselves.

**Companion file:** `docs/operations/branch-protection-v0.6.0.json` —
pure GitHub branch-protection PUT payload (no commentary, no `_meta`).

## What it protects against

| Risk | Rule |
|------|------|
| Direct push without review | `required_pull_request_reviews.required_approving_review_count = 1` |
| Stale-approval merge | `dismiss_stale_reviews = true` |
| CODEOWNERS bypass | `require_code_owner_reviews = true` (relevant per `.github/CODEOWNERS`) |
| Bad merge | `required_status_checks.strict = true` (require branch up-to-date) |
| Failing test merge | 4 required status checks (fmt+clippy, Linux test, macOS test, plugin-surface — covers all 4 sanity gates per `.github/workflows/ci.yml`) |
| Untracked conversations | `required_conversation_resolution = true` |
| Force-push history rewrite | `allow_force_pushes = false` |
| Branch deletion | `allow_deletions = false` |

## Decisions encoded

* **Status checks: 4 contexts.** `Check (fmt + clippy)`, `Test (ubuntu-latest)`, `Test (macos-latest)`, `Plugin surface (schema + shellcheck + skills + agents)`. The `bench-fast` job is intentionally **not** required — it remains advisory pending GHA budget allowing 14-consecutive-green-master gate-promotion (per `docs/operations/v0.6.0-pre-iteration-deferrals.md` entry #1).
* **No admin enforcement.** `enforce_admins = false`. Single-maintainer reality; admin-bypass is a needed escape hatch during dogfood iteration. Flip to `true` once a second maintainer exists.
* **One approver.** `required_approving_review_count = 1`. Bumps to 2 once team grows past one human reviewer.
* **No `require_last_push_approval`.** Approval after the most recent push is ergonomically expensive at one-maintainer scale. Pair with `dismiss_stale_reviews = true` (which DOES dismiss stale on push).
* **CODEOWNERS enforced.** `.github/CODEOWNERS` already pins the four sanity-gate scripts to `@chaosmaximus`; turning on `require_code_owner_reviews` makes that pin load-bearing.

## How to apply

```bash
gh api --method PUT \
  -H 'Accept: application/vnd.github+json' \
  /repos/chaosmaximus/forge/branches/master/protection \
  --input docs/operations/branch-protection-v0.6.0.json
```

## How to verify

```bash
gh api /repos/chaosmaximus/forge/branches/master/protection \
  | jq '.required_status_checks.contexts, .required_pull_request_reviews'
```

Expected:
```json
[
  "Check (fmt + clippy)",
  "Test (ubuntu-latest)",
  "Test (macos-latest)",
  "Plugin surface (schema + shellcheck + skills + agents)"
]
{
  "dismiss_stale_reviews": true,
  "require_code_owner_reviews": true,
  "required_approving_review_count": 1,
  "require_last_push_approval": false
}
```

## How to roll back

```bash
gh api --method DELETE /repos/chaosmaximus/forge/branches/master/protection
```

## Status-check name fragility

The `contexts` array uses the workflow's `name:` field at job level
(after matrix expansion). If you rename a job in
`.github/workflows/ci.yml`, GitHub does NOT auto-update the
branch-protection rule — the old context becomes "expected but
never reported," and PRs get stuck in "Required status check is
expected." When renaming jobs, update this JSON in the same PR
that renames the workflow job.

## When this gets re-staged

* Repo gets a second maintainer → bump `required_approving_review_count` to 2; consider `enforce_admins = true`.
* Bench-fast gate gets promoted (deferral #1) → add `Bench-fast (forge-consolidation)` etc. to `contexts`.
* Workflow jobs renamed → update `contexts` to match new names.
* Switch from `master` to `main` → re-run `gh api PUT ...protection` against the new branch and DELETE on the old branch.
