---
name: Feature request
about: Propose a new capability or enhancement
title: "[feat] "
labels: ["enhancement"]
assignees: []
---

## Problem statement

<!-- What can't you do today? What forces are pushing you to file this? -->

## Proposed solution

<!-- Optional. If you have a concrete shape in mind, describe it.
     Otherwise leave blank — the maintainer will scope it. -->

## Alternatives considered

<!-- Other approaches you weighed and why you rejected them. -->

## Affected layer(s)

<!-- Tick the harness layers your proposal touches; reviewers use this
     to route the issue to the right CODEOWNER and to check the 2A-4d
     interlock. -->

- [ ] daemon (`crates/daemon/`)
- [ ] CLI (`crates/cli/`)
- [ ] core protocol (`crates/core/src/protocol/`) — bumps
      `protocol_hash` in `.claude-plugin/plugin.json` via
      `bash scripts/sync-protocol-hash.sh` (W4 interlock)
- [ ] plugin manifest (`.claude-plugin/`)
- [ ] hooks (`hooks/`, `scripts/hooks/`)
- [ ] skills (`skills/`)
- [ ] agents (`agents/`)
- [ ] docs (`docs/`)

## Acceptance criteria

<!-- How will we know the feature is shipped and working?
     Concrete, testable bullet points. -->

## Out of scope

<!-- What you're NOT proposing — guards against scope creep. -->
