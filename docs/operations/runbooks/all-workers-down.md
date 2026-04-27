# Runbook — `ForgeAllWorkersDown`

## Symptom

`All Forge workers are down` — `count(forge_worker_healthy == 1) == 0`
for ≥ 1 min. **Severity: critical.**

## Likely causes

1. Daemon process crashed or was killed.
2. Daemon process is alive but every worker hung simultaneously
   (catastrophic SQLite issue or shared resource starvation).
3. systemd / launchd unit is in `failed` state.
4. Out-of-disk on the SQLite backing store; every write blocks.
5. Out-of-memory: the OS killed daemon workers.

## First-response steps

```bash
# Process alive?
ps -p $(pgrep forge-daemon)

# systemd state (Linux)
systemctl status forge-daemon

# launchd state (macOS)
launchctl list | grep forge

# Disk space
df -h ~/.forge

# Recent kernel logs
dmesg | tail -50

# Daemon log
tail -n 200 ~/.forge/daemon.log
```

## Remediation

* If daemon process is missing: restart `forge-next restart`.
* If `systemctl status forge-daemon` shows `failed`: check journal
  (`journalctl -u forge-daemon -n 200`); restart with `systemctl
  restart forge-daemon`.
* If disk-full: free space first (`forge-next admin truncate-events
  --older-than 7d`), then restart.
* If OOM kill: increase memory limits in unit file + restart.
* If daemon alive but workers hung: graceful shutdown via
  `forge-next service stop` then `start` (gives 30s drain on SIGTERM
  per P3-2 W7).

## Escalation

* Critical — page immediately.
* This alert means the daemon is non-functional. All clients will fail
  recall calls. Recovery is mandatory before resuming any agent work.
