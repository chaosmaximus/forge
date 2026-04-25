---
name: Bug report
about: Report a defect in the Forge daemon, CLI, plugin surface, or harness
title: "[bug] "
labels: ["bug"]
assignees: []
---

## Summary

<!-- One-sentence description of the misbehavior. -->

## Reproduction

**Forge version:** <!-- forge-next doctor  →  paste version + git_sha -->
**Platform:** <!-- linux x86_64 / macOS arm64 / etc -->
**Install method:** <!-- cargo install / homebrew / sideload / source build -->
**Plugin / harness layer involved:** <!-- daemon, cli, hooks, skills, agents, marketplace -->

```bash
# Exact commands to reproduce, starting from a clean state.
```

## Expected behavior

<!-- What should have happened. -->

## Actual behavior

<!-- What did happen, including any error output. Paste tracing logs
     or `/inspect` output if relevant. Include stderr verbatim. -->

## Daemon diagnostics

```text
# `forge-next doctor` output:

# `/metrics` output (if applicable, anonymise as needed):
```

## Additional context

<!-- Screenshots, transcripts, suspected root cause, anything else. -->

## Checklist

- [ ] I'm running the latest released version (or noted the version above).
- [ ] I've searched existing issues and didn't find a duplicate.
- [ ] I've included reproduction steps that don't depend on private state.
