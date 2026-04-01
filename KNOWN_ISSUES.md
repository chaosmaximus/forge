# Known Issues — Forge v0.1.x

## All Critical, Important, and Medium Issues Fixed

Through 3 rounds of Claude adversarial smoke tests + 1 round of Codex (GPT-5.4) adversarial review, 36+ issues have been identified and fixed across 12 commits.

## Remaining Low-Priority Items

These are design gaps with low risk, tracked for future improvement:

- Healthcare PRD template could have dedicated HIPAA subsection (currently handled via generic Regulatory Compliance)
- `index_status`/`index_repository` graph tools work via lead but not explicitly in agent tool lists
- `get_code_snippet` listed in README but not used by any agent
- `user_journeys_visual` skip-only signal enforcement is implicit (works correctly but ambiguous)
- Stitch MCP loads unconditionally via .mcp.json (delete the file if unused)
- CONSTITUTION.md article structure could map to specific domain compliance areas
