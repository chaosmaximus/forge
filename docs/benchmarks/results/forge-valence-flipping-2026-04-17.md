# Phase 2A-4a Forge-Valence-Flipping Dogfood Results

**Date:** 2026-04-17 (run 2026-04-18)
**Daemon binary:** commit `32d19ba` (rebuilt from source at 12:50 local time)
**Port:** 8430
**Steps exercised:** remember → flip_preference → list_flipped → compile_context

## State preservation

- Pre-rebuild: 83 decisions / 25 lessons / 15 patterns / 2 preferences / 39353 edges (total: 125 memories)
- Post-rebuild: 83 decisions / 25 lessons / 15 patterns / 2 preferences / 39353 edges (total: 125 memories)
- Delta: 0 (no change)

State preserved: yes.

Note: killing PID 30235 triggered the launchctl watchdog, which restarted the daemon via the
`~/.local/bin/forge-daemon` symlink (now pointing to the rebuilt binary at `target/release/forge-daemon`,
size 27367728 bytes) before the manual `nohup` command ran. The `nohup` attempt (PID 17157) hit the PID
lock and exited cleanly with a single "already running" ERROR in `/tmp/forge-daemon-t14.log`. The serving
daemon (PID 16142) is confirmed to be running the rebuilt binary via `lsof`.

## HTTP flow results

1. **Remember** → created preference id `01KPG6W0ZR7F3TQQ98G0YBQKX6`
   - memory_type: preference, title: "dogfood-tabs-pref-2a4a", confidence: 0.9
   - valence defaulted to "neutral" (expected — Request::Remember always defaults to neutral)

2. **FlipPreference** → new memory id `01KPG6WGWDJFKKEKAHF8P7PTFC`, flipped_at `2026-04-18 11:51:14`
   - new_valence: "negative", new_intensity: 0.8
   - reason: "T14 dogfood — testing FlipPreference end-to-end via HTTP"
   - old memory marked status: "superseded", superseded_by set to new_id

3. **ListFlipped** → returns 1 item, dogfood memory present
   - old memory: id `01KPG6W0ZR7F3TQQ98G0YBQKX6`, status=superseded, valence=neutral
   - flipped_to_id: `01KPG6WGWDJFKKEKAHF8P7PTFC`, flipped_at=2026-04-18 11:51:14

4. **CompileContext** → dynamic_suffix contains `<preferences-flipped>` with `old_valence="neutral"` and
   `new_valence="negative"` (Remember defaults valence to neutral; flip still demonstrates the full path)

   Rendered XML:
   ```xml
   <preferences-flipped>
     <flip at="2026-04-18 11:51:14" old_valence="neutral" new_valence="negative">
       <topic>dogfood-tabs-pref-2a4a</topic>
     </flip>
   </preferences-flipped>
   ```

## Doctor check

All checks passed. Daemon healthy after restart:
- daemon: running (uptime: 45s)
- memories: 126 memories stored (125 pre-rebuild + 1 new preference from dogfood)
- embeddings: 1643 embeddings indexed
- db_size: 218.4 MB
- workers: all 8 running (watcher, extractor, embedder, consolidator, indexer, perception, disposition, diagnostics)
- edge_count: 40176 (post-restart, background indexing increased from 39353)

## Log check

No ERROR-level entries from the flip operations.

The only errors in `/Users/dsskonuru/.forge/daemon.log` are background `[embedder] ollama embed failed`
entries — a pre-existing condition (ollama not running locally). These are unrelated to FlipPreference.

The single ERROR in `/tmp/forge-daemon-t14.log` is from the redundant PID 17157 startup attempt hitting
the PID lock ("another forge-daemon is already running") — not from any flip operation.

## Outcome

Phase 2A-4a Forge-Valence-Flipping shipped to live daemon. Full HTTP flow verified:
- Rebuilt binary (32d19ba) serving on port 8430
- remember → flip_preference → list_flipped → compile_context all returned HTTP 200 with correct payloads
- `<preferences-flipped>` section rendered in dynamic_suffix as designed
- State preserved across restart (zero delta on decisions/lessons/patterns/preferences/edges)
- Doctor healthy, no flip-related errors in daemon logs
