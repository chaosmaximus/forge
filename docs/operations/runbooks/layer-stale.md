# Runbook — `ForgeLayerFreshnessStaleHour`

## Symptom

`Layer {{ $labels.layer }} not updated in over 1 hour` —
`forge_layer_freshness_seconds > 3600` for ≥ 10 min on a specific layer.
**Severity: warning.**

## Likely causes

1. The layer's worker has stalled (extractor for memory layer,
   consolidator for healing layer, etc.).
2. The source feeding that layer is empty (no new transcripts → no new
   memories).
3. The freshness gauge update path is broken on this specific layer
   (gauge sticks at last value, even though layer is being written).
4. The worker is running but in a long pass that hasn't completed yet
   (e.g. consolidator on a multi-thousand-memory backlog).
5. Layer was deprecated but still has a metric registration.

## First-response steps

```bash
# Per-layer freshness
forge-next observe layer-freshness

# Recent activity for this specific layer
forge-next observe layer-activity --layer {{ $labels.layer }} --window 2h

# Worker status for the responsible worker
forge-next observe worker-status
```

## Remediation

* If worker stalled: check `worker-down.md`; restart daemon if needed.
* If source is empty: legitimate — silence the alert for that layer
  during low-activity periods.
* If gauge is broken: file a bug — the layer-freshness emit site is
  in `crates/daemon/src/server/metrics.rs`. Worker may need an explicit
  `record_layer_freshness()` call after each successful pass.
* If long pass in flight: wait it out; pass completion will refresh
  the gauge.
* If deprecated layer: remove its registration from
  `register_layer_metrics()`.

## Escalation

* Warning — investigate within 4h.
* If multiple layers stale simultaneously, suspect a daemon-wide issue
  (cross-reference `all-workers-down.md`).
