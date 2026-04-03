//! sync.rs — Hybrid Logical Clock + memory sync protocol
//!
//! HLC format: "{wall_ms}-{counter}-{node_id}"
//! - wall_ms: milliseconds since epoch
//! - counter: monotonic counter for same-millisecond events
//! - node_id: 8-char hex identifier for this daemon instance

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Hybrid Logical Clock for causal ordering across machines.
pub struct Hlc {
    node_id: String,
    state: Mutex<HlcState>,
}

struct HlcState {
    last_wall_ms: u64,
    counter: u64,
}

impl Hlc {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            state: Mutex::new(HlcState {
                last_wall_ms: 0,
                counter: 0,
            }),
        }
    }

    /// Generate a new HLC timestamp. Always monotonically increasing.
    pub fn now(&self) -> String {
        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut state = self.state.lock().unwrap();
        if wall_ms > state.last_wall_ms {
            state.last_wall_ms = wall_ms;
            state.counter = 0;
        } else {
            state.counter += 1;
        }
        format!("{}-{}-{}", state.last_wall_ms, state.counter, self.node_id)
    }

    /// Merge with a remote HLC timestamp to maintain causal ordering.
    pub fn merge(&self, remote_ts: &str) {
        let parts: Vec<&str> = remote_ts.splitn(3, '-').collect();
        if parts.len() < 2 {
            return;
        }
        let remote_ms: u64 = parts[0].parse().unwrap_or(0);
        let remote_counter: u64 = parts[1].parse().unwrap_or(0);

        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut state = self.state.lock().unwrap();
        if remote_ms > state.last_wall_ms && remote_ms > wall_ms {
            state.last_wall_ms = remote_ms;
            state.counter = remote_counter + 1;
        } else if remote_ms == state.last_wall_ms {
            state.counter = state.counter.max(remote_counter) + 1;
        }
        // If wall_ms > both, next now() call will advance naturally
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

/// Generate a stable 8-char hex node ID from hostname + process info.
pub fn generate_node_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
        hostname.trim().hash(&mut hasher);
    } else {
        "unknown".hash(&mut hasher);
    }
    // Include a stable machine identifier
    std::env::consts::OS.hash(&mut hasher);
    std::env::consts::ARCH.hash(&mut hasher);
    if let Ok(home) = std::env::var("HOME") {
        home.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hlc_new() {
        let hlc = Hlc::new("node1");
        let ts = hlc.now();
        assert!(ts.contains("node1"));
        assert!(ts.len() > 20); // "1712345678000-0-node1"
    }

    #[test]
    fn test_hlc_monotonic() {
        let hlc = Hlc::new("node1");
        let ts1 = hlc.now();
        let ts2 = hlc.now();
        assert!(ts2 > ts1, "HLC should be monotonically increasing");
    }

    #[test]
    fn test_hlc_merge_remote() {
        let hlc = Hlc::new("local");
        let _local_ts = hlc.now();
        // Simulate a remote timestamp from the future
        let future_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 10000;
        let remote_ts = format!("{}-5-remote", future_ms);
        hlc.merge(&remote_ts);
        let after_merge = hlc.now();
        assert!(
            after_merge > remote_ts,
            "after merge, HLC should be ahead of remote: {} vs {}",
            after_merge,
            remote_ts
        );
    }

    #[test]
    fn test_generate_node_id() {
        let id = generate_node_id();
        assert_eq!(id.len(), 8); // 8-char hex
        // Same machine should produce same ID
        let id2 = generate_node_id();
        assert_eq!(id, id2);
    }

    #[test]
    fn test_hlc_node_id_accessor() {
        let hlc = Hlc::new("mynode");
        assert_eq!(hlc.node_id(), "mynode");
    }

    #[test]
    fn test_hlc_format() {
        let hlc = Hlc::new("abc12345");
        let ts = hlc.now();
        let parts: Vec<&str> = ts.splitn(3, '-').collect();
        assert_eq!(parts.len(), 3, "HLC should have 3 parts: wall_ms-counter-node_id");
        assert!(parts[0].parse::<u64>().is_ok(), "wall_ms should be a number");
        assert!(parts[1].parse::<u64>().is_ok(), "counter should be a number");
        assert_eq!(parts[2], "abc12345");
    }
}
