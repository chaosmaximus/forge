# Adversarial review — 2A-6 spec v2 (second review)

**Target:** `docs/superpowers/specs/2026-04-26-multi-agent-coordination-bench-design.md` (v2)
**Target SHA:** `7329eb1`
**Date:** 2026-04-26
**Reviewer:** claude (general-purpose)
**Verdict:** `not-lockable`

## Findings count

| Severity | Count |
|----------|-------|
| BLOCKER  | 1 NEW |
| HIGH     | 2 NEW |
| MEDIUM   | 3 NEW |
| LOW      | 0     |

## v1 findings closure (Pass 1)

| v1 finding | v2 closure |
|------------|------------|
| B1 column count 11→14 | closed |
| B2 indexes 3→4 | closed |
| B3 organization_id (reviewer error) | closed (defused with cite) |
| B4 cross-project msg count 6→36 | closed |
| H1 D1 runtime denominator | closed |
| H2 sentinel-row pinning | closed (paragraph rewritten in v2.1) |
| H3 Grant/Revoke citation | closed |
| M1 65536-byte boundary | closed |
| M2 .contains() assertion | closed |
| M3 HEAD restamp | closed |

All 10 v1 findings independently verified resolved at v2 commit `7329eb1`.

## NEW findings (Pass 2)

### NEW-BLOCKER-1 — D6 trial 2 chain shape

**Spec §3.1 D6 trial 2:**
> Trial 2 chain: planner_beta → evaluator_beta (M5, kind=request) → evaluator responds 'rejected' (M6) → planner_beta → generator_beta (M7, kind=request) → generator responds 'failed' (M8).

**Spec §3.3 assertions:**
> (d) M4/M8 exists with `from_session=responder2, to_session=responder1, kind='response', in_reply_to=M_inner.id, status='<response_status>'`
> (f) M4/M8 retrievable via `list_messages(responder1.id, None, ...)`

**Bug:** Trial 2 has TWO parallel request-response pairs both originating at planner — not a linear chain. M7 is sent by planner (not by evaluator). When generator responds to M7, the new M8 has `from_session=generator, to_session=orig.from=planner` per `respond_to_message` at `sessions.rs:466-470`:

```rust
"INSERT INTO session_message (id, from_session, to_session, kind, ...
 VALUES (?1, ?2, ?3, 'response', ..., from_session, orig_from, ...)"
```

So M8.to_session = planner_beta, not evaluator_beta. Assertion (d) requires "to_session=responder1" where responder1=evaluator (the M5/M6 responder). M8.to_session ≠ evaluator. ✗

Assertion (f) requires M8 retrievable in evaluator's inbox. Since M8.to_session=planner, it appears in PLANNER's inbox, not evaluator's. ✗

Trial 2 silently scores 4/6 (assertions a/b/c/e pass; d/f fail) on every green run. D6 max becomes 4/12 + 6/12 = 10/12 = 0.833 > 0.90 min threshold — trial 2 burns 2/6 of the score for no signal value.

**Fix:** trial 2 must be a true linear chain `r1 → r2 → r3`:
- Step 1: planner_beta sends request M5 to evaluator_beta
- Step 2: evaluator_beta sends request M7 to generator_beta
- Step 3: generator responds to M7 → M8 (from=generator, to=evaluator)
- Step 4: evaluator responds to M5 → M6 (from=evaluator, to=planner)

Now M8.to=evaluator (responder1), M8 retrievable in evaluator's inbox.

### NEW-HIGH-1 — §3.1a paragraph contradicts §3.1+§4 D11

**Spec §3.1a:**
> D2 sends brand-new messages with non-colliding ids; ... D6 pipeline creates new messages between role pairs that exclude the (planner_alpha, generator_alpha) pair on purpose (D6 trial 1 uses planner_beta → generator_beta → evaluator_beta → generator_beta → planner_beta; D6 trial 2 uses planner_alpha → generator_beta → evaluator_alpha [cross-project on purpose] OR a same-project chain THAT excludes the sentinel pair — see §4 D11 below).

This describes a 5-hop chain in trial 1 + a cross-project trial 2.

**Spec §3.1 D6:**
> Trial 1 chain: planner_beta → generator_beta (M1, kind=request) → generator responds (M2) → generator_beta → evaluator_beta (M3, kind=request) → evaluator responds (M4)

That's a 4-message chain in trial 1 (planner→generator→evaluator), not a 5-hop chain.

**Spec §4 D11:**
> All D6 pipeline trials run in `team-beta`

Direct contradiction: §3.1a says trial 2 uses planner_alpha (cross-project); §4 D11 forbids alpha entirely.

Stale draft from an earlier design pass. Implementer following §3.1a would code an alpha-touching trial 2, breaking the sentinel-row hash invariant.

**Fix:** Rewrite §3.1a paragraph to match §3.1 D6 + §4 D11.

### NEW-HIGH-2 — D6 K=2 trials gives no alpha coverage

D11 mandate forces beta-only D6, but the chain `planner_alpha → evaluator_alpha → generator_alpha` AVOIDS the sentinel pair (planner_alpha, generator_alpha) entirely — neither pair (planner→evaluator) nor (evaluator→generator) is the forbidden one. K=3 with this trial 3 adds alpha coverage while preserving sentinel disjointness.

**Fix:** Add trial 3 = planner_alpha → evaluator_alpha → generator_alpha. Score denominator 12 → 18. Min 0.90 unchanged.

### NEW-MED-1 — §3.2 self-contradicting line

> "6 inboxes × 6 = 36 cross-project messages total (16 alpha→beta + 20 beta→alpha is symmetric: 18 alpha→beta + 18 beta→alpha)"

"16+20" is residual draft typo (16+20 ≠ 36 anyway). Drop, keep only "18+18=36 (3 senders × 3 recipients × 2 msgs/pair, symmetric)".

### NEW-MED-2 — D1 hardcoded /6 + check 1 ≥14 looseness

D1 formula: `pre_d1_total - (pre_d1_total / 6)` — divisor 6 hardcoded; future 12-inbox corpus extension would silently miscompute.

Check 1: `≥14` instead of `==14` — future migration adding a 15th column would still pass ≥14 silently.

**Fix:** D1: `num_inboxes = corpus.sessions.len()` + `debug_assert!(pre_d1_total % num_inboxes == 0)`. Check 1: tighten to exact `==14` with named const `SESSION_MESSAGE_COLUMN_COUNT`.

### NEW-MED-3 — Check 6 bundles two assertions

`session_distribution_correct_and_pre_d1_count_60` — two failure modes share one name. Split into 6 (distribution) + 7 (count==60). Total infra checks 8 → 9.

## Math sanity (re-verified at v2)

- Composite weights: 0.20+0.15+0.15+0.20+0.15+0.15 = 1.00 ✓
- Cross-project msg total: 36 ✓ (post v2 BLOCKER-4 fix)
- ULID = 26 chars ✓
- Broadcast fan-out = 2 ✓
- D5 probe 2 boundary 65536 ✓ (sessions.rs:375 is `> 65536`)
- D5 probe 1 substring "exceed 64KB limit" ✓ (sessions.rs:377)

## Resolution path

Per project policy: **Path A — rewrite spec to v2.1** addressing 1 BLOCKER + 2 HIGH + 3 MED in one commit. Re-verify at v2.1 lock.
