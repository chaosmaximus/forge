# Changelog

All notable changes to Forge are recorded here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — P3-4 Wave Z

CC voice first-run setup unblock (per `feedback/2026-04-26-setup-and-isolation.md`).
User-facing vocabulary is now **project everywhere** — internal `reality`
table is implementation detail. Internal struct/module rename is
deferred to a follow-up cleanup pass.

### Breaking — protocol

These rename existing JSON-RPC method names. External daemon clients
(third-party scripts hardcoding `{"method": "..."}` payloads) must
migrate. The `protocol_hash` field in `.claude-plugin/plugin.json`
moved from `1b3dec55ffa4…` to `68432a815353…`; the harness-sync CI
gate enforces that downstream callers stay in sync.

* `Request::DetectReality { path }` → `Request::ProjectDetect { path }`
* `Request::ListRealities { organization_id }` → `Request::ProjectList { organization_id }`
* `ResponseData::RealityDetected { reality_id, reality_type, ... }` →
  `ResponseData::ProjectDetected { id, engine, ... }`
* `ResponseData::RealitiesList { realities }` →
  `ResponseData::ProjectList { projects }`

### Breaking — CLI

* `forge-next detect-reality` removed; use `forge-next project detect [<path>]`.
* `forge-next realities` removed; use `forge-next project list`.

### Added

* `forge-next project init <name> [--path PATH] [--domain DOMAIN]` —
  explicit project creation. Lets users bind a project before the
  first SessionStart fires (CC voice §2.4).
* `forge-next project show <name>` — detail view with indexed file +
  symbol counts.
* `forge-next compile-context --cwd <path>` — auto-create a project
  record from CWD when `--project <name>` is supplied for an unknown
  project. Surfaces `<code-structure project="<name>"
  resolution="auto-created" ...>` instead of `resolution="no-match"`
  on a fresh project's first turn (CC voice §1.2 fix #2).
* `forge-next compile-context --dry-run` — preview the assembled
  context without recording an injection event or touching memory
  access counts (CC voice §2.9).
* `forge-next update-session --id <SESSION> --project <NAME> [--cwd <PATH>]` —
  fix a session whose project label was set incorrectly by the
  SessionStart hook (CC voice §2.6).
* `FORGE_HOOK_VERBOSE=1` — opt-in env var that surfaces SessionStart
  hook errors to stderr (CC voice §2.10). Default-quiet stays.
* `forge-next doctor` now reports backup hygiene (`*.bak` accumulation
  in `~/.forge/`, CC voice §2.7) and CLI-vs-daemon `git_sha` drift
  even when `CARGO_PKG_VERSION` matches (CC voice §1.3 fix #2).
* `<code-structure>` XML tag now carries a `resolution=` attribute
  whose value is one of `exact`, `no-match`, `unscoped`, `auto-created`
  (Z2 + Z7).

### Changed

* `<code-structure>` XML attribute renamed `reality=` → `project=`.
  The unscoped path no longer emits `project=` at all (legacy callers
  who got "all" rendered as if a particular project owned the
  aggregate now see no claim).
* `compile-context` honors `--project <name>` correctly — pre-Z2 the
  inner SQL ignored the parameter and rendered the most-recently
  indexed project for ANY caller, leaking 188 forge files into
  cc-voice's first-turn context (CC voice §1.2).
* `forge-setup` skill rewritten — drops references to a non-existent
  `forge` CLI binary, points new users at `project init` and
  `compile-context --dry-run` (CC voice §2.1 + §2.2 + §2.8 + §2.9).
* `detect-reality` (now `project detect`) accepts positional `<path>`
  in addition to `--path` flag (CC voice §2.3).
* `.claude-plugin/plugin.json` no longer hard-references
  `./hooks/hooks.json` — Claude Code auto-loads that path; the
  duplicate reference was breaking `/plugin list` for forge (CC voice
  §1.1).

### Deprecated

* The `reality` SQL table name is internal-only and stable for v0.6.x;
  a future cleanup pass will rename it. Code that issues raw SQL
  against the schema must continue to use `reality` until then.
