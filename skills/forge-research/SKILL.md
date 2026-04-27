---
name: forge-research
description: "Bounded research workflow — explore a topic in 5-10 short cycles, store key findings as Forge memory. Use when user says 'research this', 'investigate how X works', 'explore the codebase', 'understand this module', or needs a structured deep-dive on a technical topic."
---

# Forge Research — Bounded Exploration

A manual, structured exploration workflow. Use existing tools (recall,
grep, code-search, web-fetch) inside a bounded loop so the investigation
stays focused and produces durable artifacts.

## When to Use

- User asks to "research", "investigate", "explore", "understand" a topic
- User wants to understand an unfamiliar codebase area
- User needs competitive/technical analysis

## Process

### 1. Frame the question

Restate the research question in one sentence. If it's vague, ask one
clarifying question before proceeding. Set an iteration budget:
default **5**, max **10**.

### 2. Iterate (each loop ≤ 2 min, max budget set in step 1)

For each iteration:

a. **Hypothesize** where to look next (one sentence — what would prove
   or disprove your current model?).
b. **Explore** using whichever of these is cheapest:
   - `forge-next recall "<keywords>"` — prior decisions and lessons
   - `forge-next code-search "<keywords>" --project <project-name>` —
     symbols and files matching the query
   - `forge-next blast-radius --file <path> --project <project-name>` —
     callers and dependents of a known file
   - `Grep` / `Read` for targeted source inspection
   - `WebFetch` for external docs (only when local sources are
     exhausted)
c. **Record** the finding: 1-2 sentences. Include file:line if the
   evidence is in code.
d. **Decide** keep (productive path) or discard (dead end). Do not
   keep going if your last 3 iterations produced no new evidence —
   stop and report the partial result.

### 3. Synthesize

Summarize the findings as a short conclusion + open questions. Use
this as the user-facing report.

### 4. Store the durable findings

For each "this changed my mental model" finding, store it as a
lesson:

```bash
forge-next remember --type lesson --title "<short title>" \
  --content "<finding + supporting evidence>"
```

For decisions the user makes during the research, use `--type decision`
instead.

## Constraints

- Maximum 10 iterations per research session (default 5)
- Each iteration should take <2 minutes
- If stuck after 3 iterations with no progress, stop and report
- Never modify production code during research — read-only exploration

## Output

A short user-facing report:
- The original question
- Iteration count + outcome (answered / partial / stuck)
- Top 3 findings with file:line / URL evidence
- Memories stored (titles)
- Open questions (if any)
