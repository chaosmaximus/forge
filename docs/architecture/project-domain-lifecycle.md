# Project / domain lifecycle

**Owner:** P3-4 Wave X (cc-voice Round 3 §E design answer).
**Status:** v0.6.0 ships the bind-time logic; v0.6.1+ ships the upgrade
worker. This doc pins the contract so the v0.6.1 work doesn't silently
reverse the design.

## Background

A Forge "project" (internal struct: `Reality`, SQL table: `reality`)
carries a `domain` column — a string that identifies the project's
primary technology family (`rust`, `node`, `python`, `go`, …). The
column is set when the project row is created and is consumed by
`<code-structure>` rendering (so agents see the right boundaries) and
by indexer dispatch (so language-specific extractors fire).

Three creation paths exist:

| Path | Trigger | Domain source |
|------|---------|---------------|
| **Z3** `forge-next project init <name> --path <p> [--domain <d>]` | Explicit user CLI | User-supplied or `CodeRealityEngine.detect()` |
| **Z7 / X1** `compile-context --project <name> --cwd <p>` | First SessionStart hook contact | `CodeRealityEngine.detect(<p>)` or `"unknown"` (Y2 fallback) |
| **Z6 / Y2** `forge-next project detect <p>` | Explicit user CLI | `CodeRealityEngine.detect(<p>)` or `"unknown"` (Y2 fallback) |

Pre-Wave-Y the synthetic-fallback path (Y2) didn't exist — the daemon
errored on code-less directories, blocking single-`.md` design dirs
from binding. Wave Y / Y2 added the `"unknown"` fallback so agents can
auto-bind regardless of file-system shape.

## The Round 3 §E question

cc-voice surfaced the natural follow-up: when a project is bound
auto-mode with `domain="unknown"` (because the directory had no
language markers at the time), and the user later adds a marker file
(`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, etc.), what
should happen to the row's `domain`?

Two design poles:

1. **Lock** — `domain` is set once at bind time and never changes
   without an explicit user action (`forge-next project re-detect
   <name>`, deferred to v0.6.1 as task #217).
2. **Hint** — `domain` is a current-best-guess; the indexer worker
   re-runs detection on the next contact and upgrades the row in
   place when it finds a higher-confidence domain.

## Decision (locked 2026-04-27)

**`domain` is a HINT. The indexer upgrades in place when the previous
value is `"unknown"` and `CodeRealityEngine.detect()` now returns a
real domain.**

### Rationale

* **No surprises for the lock-leaning user.** Once the row carries a
  real domain (`rust`, `node`, etc.), the upgrade is a no-op — we
  only ever climb upward from `"unknown"`. The flow that explicitly
  set `domain="rust"` via `project init` is preserved untouched.
* **Matches the spirit of Y2.** Y2's whole point is that
  `"unknown"` is a temporary placeholder so agents can bind from
  turn 1; making it sticky would defeat the placeholder semantics.
* **Avoids a friction step** (`forge-next project re-detect`) that
  most users wouldn't think to run. The natural workflow is "I added
  `Cargo.toml`, my agent should know" — not "I added `Cargo.toml`,
  remember to re-detect."
* **Idempotent + observable.** The upgrade fires once per (row,
  successful detection) and emits a `tracing::info!` line so
  operators can audit the transition. Subsequent indexer passes
  re-detect to the same value and short-circuit.

### Lock semantics if you want them

A user who wants `domain` lock semantics for paranoia (e.g., a
polyglot repo where `Cargo.toml` is incidental but the project is
primarily Python) can:

1. Run `project init <name> --path <p> --domain <chosen-domain>`
   *before* the auto-create path fires — explicit init wins, and
   the upgrade rule never triggers because the value is no longer
   `"unknown"`.
2. Or, after auto-create, run `project init <name> --path <p>
   --domain <chosen-domain>` — but per Y5, `project init` is
   idempotent (existing rows are never overwritten), so this only
   works pre-bind. Lock-after-bind requires the (deferred) `project
   re-detect` and `project update` commands.

The escape valves are explicit and discoverable; the default is the
forgiving one.

## Upgrade contract (for v0.6.1 implementer)

When the indexer worker (or any other writer that observes
`CodeRealityEngine.detect()` succeed) runs against a project whose
existing row has `domain="unknown"`, it MUST:

1. Treat the new detection as authoritative and emit:
   ```sql
   UPDATE reality
   SET domain = ?, detected_from = COALESCE(detected_from, 'indexer_upgrade'), last_active = ?
   WHERE name = ? AND organization_id = ? AND domain = 'unknown';
   ```
   The `domain = 'unknown'` clause is a guard — it makes the
   UPDATE a no-op for rows that already carry a real domain.
   `detected_from` keeps the original bind-time value when set
   (so audit trails distinguish "compile_context_cwd" from the
   later upgrade); when null, the upgrade marks the row.
2. Emit a `tracing::info!(target: "forge::indexer", project, old =
   "unknown", new = <new>, "domain upgraded in place")` log line
   so the transition is observable in the daemon's structured log.
3. NOT delete or recreate the row. The `id` (ULID) is stable; only
   `domain` (and possibly `last_active`) change. Memories and
   `code_file` rows that JOIN against `reality.id` are unaffected.

The implementation is a small UPDATE inside the indexer's per-pass
loop or the perception worker's first-detection branch. Tracked as
the v0.6.1 follow-up to Wave X.

## Test contract

The v0.6.1 implementer ships a regression test that:

1. Creates a project with `domain="unknown"` (via the X1 auto-create
   path against an empty tempdir).
2. Adds a marker file (`Cargo.toml`) to the tempdir.
3. Triggers the indexer / perception worker.
4. Asserts `get_reality_by_name(name, org).domain == Some("rust")`.
5. Re-triggers the indexer and asserts a second pass is a no-op
   (idempotency).
6. Negative case: a project with `domain="rust"` set explicitly does
   NOT get downgraded if `CodeRealityEngine.detect()` later returns
   `"unknown"` (e.g., user moved files around).

## Cross-references

* Wave Y / Y2 (`b4078eb`) — synthetic `unknown` fallback in
  `Request::ProjectDetect` and `compile-context --cwd` paths.
* Wave Z / Z7 (`23cc4b6`) — auto-create on first `compile-context
  --cwd` contact.
* Wave X / X1 (`97b6caf`) — auto-create write path landed under
  read-only routing.
* Memory `feedback_xml_attribute_resolution_pattern.md` — `auto-created`
  resolution attribute that `<code-structure>` emits when the row
  exists but has zero indexed files.
* feedback `2026-04-27-round-3-post-wave-y.md` §E — cc-voice's
  original question that drove this doc.
