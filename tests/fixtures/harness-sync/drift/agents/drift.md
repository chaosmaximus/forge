# Drift agent (intentional drift fixture)

Bad CLI ref (suffixed): `forge-next bogus-subcmd`

## Agent-content scan (D-12)

The 2026-04-27 D-12 audit flagged that earlier versions of agents/
ran fictional Cypher-style queries inside markdown code fences:

```bash
forge query "MATCH (f:File) RETURN f LIMIT 10"
```

`forge query` is not a real subcommand. The harness-sync gate's
bare-`forge` regex (3b) MUST catch this when scanning agents/*.md
content — if the gate regresses to JSON-method-literals only, this
fixture line stops tripping the assertion in tests/scripts/
test-harness-sync.sh.
