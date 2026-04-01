---
name: forge-new
description: Use when building a new project from scratch in an empty or near-empty directory
---

# Forge — Greenfield Mode

Building something new. Focus: understand WHAT to build before building it.

**Proactive-but-user-guided principle:** Forge is proactive in COMMUNICATION — it announces phases, explains what is happening, presents clear options with recommendations. But it NEVER acts autonomously on decisions that affect the user's project. The user is always the guide.
- ALWAYS present options with a recommendation, then WAIT for the user's choice
- ALWAYS ask before: starting a build, merging code, creating PRs, modifying architecture
- ALWAYS surface findings and let the user decide the response
- NEVER skip a user approval gate because "it's obvious"
- NEVER auto-fix evaluator findings without presenting them first
- Proactive = "Here's what I found, here's what I recommend, what do you want to do?"
- NOT proactive = "I found an issue and fixed it for you"

## Checklist

You MUST create a TaskCreate item for each phase and complete them in order:

1. **Classify project** — match against project-types.csv and domain-complexity.csv
2. **Discover requirements** — structured questioning, one at a time
3. **Draft PRD** — using template + CSV-driven sections
4. **User approves PRD**
5. **Visual design** (if Stitch available) — generate UI mockups
6. **Create build plan** — wave groupings from PRD deliverables
7. **Build** — agent team execution
8. **Review** — invoke forge-review
9. **Ship** — invoke forge-ship

---

## Phase 1: Classify

0. Create STATE.md from the template at `${CLAUDE_PLUGIN_ROOT}/templates/STATE.md`. Set mode to 'greenfield' and phase to 'classify'. This tracks session state for handoff/resume.

1. Read `${CLAUDE_PLUGIN_ROOT}/data/project-types.csv`
2. Match the user's description against `detection_signals`. If ambiguous, ask:
   "This sounds like a [type]. Is that right, or is it more of a:
   (a) [alternative 1]
   (b) [alternative 2]
   (c) Something else"
3. Read `${CLAUDE_PLUGIN_ROOT}/data/domain-complexity.csv`
4. If the domain matches an entry, IMMEDIATELY surface `key_concerns`:
   "Since this is a [domain] project, we need to address: [key_concerns].
   I'll make sure the PRD covers these."
   Do NOT wait for the user to ask about compliance. They may not know they need it.

---

## Phase 2: Discover

Use `key_questions` from the matched project type. Ask ONE question at a time.
- Prefer multiple choice with your recommendation first
- Lead with why your recommendation makes sense
- Use `[NEEDS CLARIFICATION]` if the user's answer is ambiguous
- Do NOT generate any content yet — this is pure conversation

Minimum discovery (even for "simple" projects):
- What problem does this solve?
- Who are the users? (all types: primary, admin, API consumers)
- What does success look like? (user success + business success)
- What is out of scope for v1?

If the user says "just build it" or "skip the questions", present ONLY the 4 minimum discovery questions above (not the CSV key_questions). If they still want to skip, acknowledge and proceed with `[NEEDS CLARIFICATION]` markers for anything unresolved. Never force the full question set on an impatient user — but always get the minimum 4.

---

## Phase 3: Draft PRD

1. Read `${CLAUDE_PLUGIN_ROOT}/templates/PRD.md`
2. Include ONLY `required_sections` for the matched project type
3. Skip `skip_sections` — do not include them even as empty headers
4. Frame functional requirements as capability contracts: "FR#: [Actor] can [capability]"
5. For NFRs: only include categories that MATTER for this product. Ask:
   "Which of these are critical for your project?
   (a) Performance — response time, throughput targets
   (b) Security — auth, data protection, compliance
   (c) Scalability — expected load, growth projections
   (d) Accessibility — WCAG level, i18n requirements
   (e) None of these are critical right now"
6. Include domain-specific sections from domain-complexity.csv `special_sections`
7. Present the FULL PRD to the user at once. Ask: "Here is the complete PRD. Does everything look right, or would you like to discuss any specific section?" Only go section-by-section if the user requests it.

---

## Phase 4: User Approves PRD

<HARD-GATE>
Do NOT proceed to design or build until the user has approved the PRD.
No "this is simple enough to skip PRD" exceptions.
The PRD can be 1 page for a simple project. But it must exist and be approved.
</HARD-GATE>

Present the complete PRD and ask for explicit approval:
"Here's the complete PRD. Please review it and let me know:
(a) Approved — let's proceed
(b) Changes needed — tell me what to adjust
(c) Start over — the direction is wrong"

Do NOT interpret silence or partial acknowledgment as approval. Wait for an explicit "yes" or "approved."

If the user chooses "Start over": reset STATE.md to phase=classify, delete the draft PRD, and return to Phase 1. The user can start fresh with a different direction.

---

## Phase 5: Visual Design (Optional)

First, check if `ui_design` is in the `skip_sections` for the matched project type (from Phase 1). If `ui_design` is skipped, this project has no frontend — skip Phase 5 entirely and note in STATE.md: "Visual design skipped (no UI for this project type)."

If Stitch MCP is available (`stitch_enabled` in userConfig):
1. Announce: "I can generate visual UI mockups using Google Stitch before we build. Want me to? This helps ensure the UI matches your vision before writing code."
2. If yes: use Stitch MCP tools to generate designs from the PRD's user journeys
3. Present designs to user for approval/iteration
4. Export design tokens / design.md for the generator to reference

If Stitch is not available:
1. Check if `frontend-design` plugin is installed
2. If yes: "I'll use the frontend-design skill during build to ensure high-quality UI code."
3. If no: skip visual design, note the skip in STATE.md, and recommend setting it up for next time. Proceed to plan.

---

## Phase 6: Create Build Plan

1. Extract deliverables from the approved PRD
2. Group into dependency-aware waves:
   - Wave 1: independent foundational pieces (models, auth, core APIs)
   - Wave 2: features that depend on Wave 1
   - Wave 3: integration, UI, cross-cutting concerns

Wave sizing guidelines:
- Each wave should produce a testable increment (something you can verify works)
- Wave 1 is always: data models + core business logic (the foundation)
- Wave 2 is typically: API/interface layer (depends on Wave 1)
- Wave 3 is typically: integration, UI, cross-cutting concerns
- Maximum 4 tasks per wave (more than 4 parallel generators causes diminishing returns)
- If a single feature is complex enough for 3+ waves, consider decomposing it

3. Present wave plan to user for approval. Do NOT start build without user sign-off on the plan.

<HARD-GATE>Do NOT start building until the user has approved the wave plan. Even for simple projects.</HARD-GATE>

---

## Phase 7: Build

Follow the build workflow defined in `${CLAUDE_PLUGIN_ROOT}/skills/forge-build-workflow.md`. Read that file and execute its steps.

<HARD-GATE>
Do NOT merge any generator output without evaluator review passing.
No "looks fine to me" overrides from the lead. The evaluator must run.
Exception: if the task is trivial (1-2 files, < 50 lines changed), the lead
can review directly instead of spawning an evaluator agent.
</HARD-GATE>

---

## Phase 8: Review

Invoke `forge-review` skill.

The review skill runs a two-stage evaluation:
1. Internal evaluator (forge-evaluator) reviews for code quality, architecture, and security
2. Cross-model adversarial review via Codex for different-perspective analysis

Present all findings to the user. Let the user decide which findings to address.

---

## Phase 9: Ship

Invoke `forge-ship` skill.

The ship skill handles:
1. PR creation with structured summary
2. Final gate verification
3. Episodic memory save for future sessions

---

## Rationalization Prevention

| If you're thinking... | The answer is... |
|----------------------|-----------------|
| "The user knows what they want, skip discovery" | Discovery surfaces what they DON'T know they need. Run it. |
| "This domain doesn't need compliance checks" | Check the CSV. If it lists key_concerns, surface them. |
| "PRD is overkill for this project" | A 1-page PRD is not overkill. It's a 5-minute investment that prevents hours of rework. |
| "Let's just start building" | <HARD-GATE> prevents this. Follow the process. |
| "Stitch isn't set up, skip design entirely" | Note the skip in STATE.md. Recommend setting it up for next time. |
| "The evaluator is slowing us down" | The evaluator catches bugs before users do. It stays. |
| "I can review this myself, no need for evaluator" | Self-review has blind spots. The evaluator uses different criteria. No merge without it. |
| "This is just a prototype, quality doesn't matter" | Prototypes become production code. Build it right or mark it explicitly disposable in the PRD. |
| "The user seems impatient, skip the approval gate" | Skipping approval leads to rework. A 30-second approval saves hours. |
