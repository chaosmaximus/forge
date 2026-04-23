---
name: forge-research
description: "Autonomous research loop — explore a topic with bounded iterations, git-backed experiments. Use when user says 'research this', 'investigate how X works', 'explore the codebase', 'understand this module', 'do a deep dive on', or needs bounded autonomous exploration of a technical topic."
---

# AutoResearch

Bounded autonomous exploration. Investigates a topic through iterative cycles of hypothesis, exploration, measurement, and keep/discard.

## When to Use

- User asks to "research", "investigate", "explore", "understand" a topic
- User wants to understand an unfamiliar codebase area
- User needs competitive/technical analysis

## Process

1. **Frame** the research question clearly
2. **Run** `forge research "<topic>" --max-iterations N --workdir .`
3. **For each iteration:**
   a. Form a hypothesis about where to look
   b. Explore (read code, search, fetch docs)
   c. Record finding
   d. Decide: productive path (keep) or dead end (discard)
4. **Synthesize** findings into a conclusion
5. **Store** key findings as Lesson nodes: Run `forge-next remember --type lesson --title '...' --content '...'`

## Constraints

- Maximum 10 iterations per research session (default 5)
- Each iteration should take <2 minutes
- If stuck after 3 iterations with no progress, stop and report
- Never modify production code during research — read-only exploration
- Git checkpoint before each iteration for safe rollback

## CLI

```bash
forge research "How does the auth module work?" --max-iterations 5
forge research "Compare JWT vs session tokens" --max-iterations 8
```

## Output

Structured JSON with iteration details + human-readable summary.
