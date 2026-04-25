# P3-2 W7 — Adversarial Review Transcript

**Date:** 2026-04-25
**Reviewer:** Claude general-purpose subagent
**Commit reviewed:** `07cc70d` ("feat(P3-2 W7): daemon SIGTERM handler")
**Base:** `f97832e` (P3-2 W6 fix-wave)
**Verdict:** `lockable-as-is`

## Verdict summary

| Severity | Count | Status |
|----------|-------|--------|
| BLOCKER  | 0 | n/a |
| CRITICAL | 0 | n/a |
| HIGH     | 0 | n/a |
| MEDIUM   | 0 | n/a |
| LOW      | 2 | both deferred |

**Aggregate:** zero correctness/safety findings. Reviewer ran a thorough audit (12 prompt items) and concluded the diff is "strictly mechanical" with no actionable issues. Two LOW items captured but deferred — neither is a lock blocker.

## LOW-1 — Forward version reference in playbook (deferred)

**File:** `docs/operations/2P-1-rollback.md:153`

**Reviewer rationale:**

> The note says "as of P3-2 W7 (v0.6.0-rc.2)" but W7 lands before the P3-2 close commit that actually bumps to rc.2. A contributor reading at HEAD between W7 and the close sees a forward reference. Risk is cosmetic — Cargo.toml is the source of truth and the forward-reference window is hours/days at most, owned by the same engineer driving P3-2 close.

**Status:** deferred. The version bump lands at P3-2 close (next commit), at which point the reference resolves. Window of inaccuracy is the gap between this commit and the close — owned by the same workflow.

## LOW-2 — No automated regression test for SIGTERM (deferred)

**File:** `crates/daemon/src/main.rs:391`

**Reviewer rationale:**

> Dogfood log evidence is recorded in the commit body, but there is no integration test that subprocess-spawns the daemon and asserts both signals produce graceful shutdown. Future regressions (e.g. someone collapses the Unix arm back to ctrl_c only) would not be caught by `cargo test`. Acceptable to skip given the cost of subprocess-based signal tests and the simplicity of the diff.

**Status:** deferred. Tracked as a future backlog item — `tests/integration/signal_shutdown.rs` would spawn the release binary, send SIGTERM/SIGINT, assert exit within drain budget. Out of scope for W7; the diff is small enough that visual review + dogfood evidence is sufficient.

## Notable non-findings (reviewer's own validation work)

The reviewer went deep on the tokio mechanics and ran 12+ correctness checks:

1. **`tokio::select!` pattern is idiomatic.** Both branches are cancel-safe (tokio::signal::ctrl_c and Signal::recv are both documented cancel-safe). If both signals arrive simultaneously, tokio::select! picks one arbitrarily and drops the other future. No deadlock possible.

2. **`sigterm` lifetime correct.** `let mut sigterm = …;` lives in the same block as the select!; the borrow extends through the await and ends when select! returns. The Signal value drops after `let _ = send`. No lifetime issue.

3. **Fallback path correct.** On SIGTERM-registration error, the fallback does exactly what pre-W7 main.rs did (ctrl_c().await + send(true)), then early-returns from the task. shutdown_for_signal channel ownership preserved.

4. **Windows path verified.** `tokio::signal::ctrl_c` on Windows uses SetConsoleCtrlHandler firing on CTRL_C_EVENT + CTRL_BREAK_EVENT + CTRL_CLOSE_EVENT — slightly broader than just Ctrl+C, fine here. Project doesn't ship Windows binaries; guard is defensive against future cross-compile.

5. **Multi-fire / second-signal:** spawned task exits after first signal (same as pre-W7). No "second Ctrl+C = force kill" escalation, but that wasn't there before W7 either — not a regression.

6. **shutdown_tx receiver chain verified:** http (line 509), grpc (line 607), Unix socket (line 647) all subscribe/observe the same broadcast. Both signal paths feed the identical drain.

7. **`let _ = shutdown_for_signal.send(true);`** — discarding SendError is intentional. watch::Sender::send only fails if all receivers were dropped, which during graceful shutdown means the transports already exited.

8. **Backward compat:** v0.5.x / rc.1 daemons predate the SIGTERM handler, so `kill -INT` (which the playbook keeps) still works against them — SIGINT was always handled.

9. **Signal-stream backlog:** tokio docs say signals received before the first `recv()` are coalesced (one pending bit), the next recv() returns. Registration happens before the select!.await point, so any SIGTERM arriving in the registration→recv window is delivered. No race.

10. **Diff hygiene:** strictly mechanical. No drive-by edits beyond the doc comment and the rollback playbook block — both directly tied to the W7 change.

11. **Doc-comment accuracy:** the new comment accurately describes the runtime behavior verified above (single shutdown channel feeds all three transports' drain paths).

12. **Dogfood evidence credible:** both SIGTERM and SIGINT logs show the identical sequence (`shutting down (signal=SIG…)` → `draining in-flight requests` → `all connections drained, exiting cleanly` → `daemon stopped`). 6s wall-clock budget per signal.

## Process check

* Wave timing: W7 commit (`07cc70d`) → review (this transcript) → no fix-wave needed (verdict: lockable-as-is).
* SIGTERM gap (P3-1 W5 review HIGH-1) **strategically closed.**
* All 7 P3-2 waves now complete; phase ready for close.
