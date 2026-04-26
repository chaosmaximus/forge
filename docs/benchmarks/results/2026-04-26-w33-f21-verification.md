# W33 F21 verification — force-index error UX closed by W22

**Date:** 2026-04-26
**Phase:** P3-3.11 W33 (no commit needed — verifies the W22 fix
already shipped at SHA `611169b`).

## F21 dogfood symptom

> **F21 — force-index error UX.** It returns
> `daemon response timed out (30s)` — but it's not clear whether the
> indexer is still running, has crashed, or has succeeded silently.
> A `--background` flag with progress polling would resolve this.

## Resolution

W22 (commit `611169b`, fix(P3-3.9 W22): force-index runs async — writer
-actor unblocked) made `Request::ForceIndex` dispatch to a background
task on the daemon side, returning to the caller immediately. The CLI
surface (`crates/cli/src/commands/system.rs::force_index`) already
prints a user-friendly background-dispatch message. No code change
needed in W33.

## Live verification (post-W30 daemon, HEAD `fa19a54` build)

```
$ time forge-next force-index
Indexer dispatched in background. Watch ~/.forge/daemon.log or query
progress with `forge-next find-symbol <name>` / `forge-next code-search
<query>`.

real    0m0.009s
user    0m0.003s
sys     0m0.005s

$ time forge-next health
Health:
  decisions:   25
  lessons:     10
  ...

real    0m0.008s
```

* `force-index` returns in **9 ms** (was 30 s+ pre-W22 with timeout).
* Subsequent writes (`health` is still a write-route check via the
  shared writer actor pool) are not blocked — also 8 ms.
* The user-facing message clearly states what happened
  ("dispatched in background") and how to check progress
  (`find-symbol`, `code-search`).

The original F21 ambiguous-timeout symptom no longer reproduces. F21 is
closed.

## What changed since the dogfood

* W22 (`611169b`): writer-actor side dispatches ForceIndex via
  `process_force_index_async` → `run_force_index_in_task`. CLI surface
  unchanged but no longer races against the 30 s timeout.
* W32 (`fa19a54`, just landed): adds a fresh-mtime gate to the
  background indexer so file edits surface in `find-symbol` /
  `code-search` within 60 s instead of up to 5 minutes. Together W22
  + W32 close the F20 / F21 / F22 cluster end-to-end.

## Carry-forward

W33 had a budget of "1 commit OR no-op verify" in
`docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md`. This is the
no-op leg — the verification doc is the only artifact.
