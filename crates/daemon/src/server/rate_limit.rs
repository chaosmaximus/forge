//! Lightweight in-memory rate limiter for HTTP API endpoints.
//!
//! Uses a token bucket per IP address. No external dependencies.
//! Configurable: requests per window, window duration.
//! Stale entries auto-evicted on cleanup interval.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Rate limit configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per window per IP.
    pub max_requests: u32,
    /// Window duration.
    pub window: Duration,
    /// Maximum failed auth attempts per window before lockout.
    pub max_auth_failures: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 100,
            window: Duration::from_secs(60),
            max_auth_failures: 5,
        }
    }
}

/// Per-IP request counter with sliding window.
struct IpBucket {
    count: u32,
    auth_failures: u32,
    window_start: Instant,
}

impl IpBucket {
    fn new() -> Self {
        Self {
            count: 0,
            auth_failures: 0,
            window_start: Instant::now(),
        }
    }

    fn is_expired(&self, window: Duration) -> bool {
        self.window_start.elapsed() > window
    }

    fn reset(&mut self) {
        self.count = 0;
        self.auth_failures = 0;
        self.window_start = Instant::now();
    }
}

/// Thread-safe rate limiter state.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, IpBucket>>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    /// Check if a request from the given IP should be allowed.
    /// Returns Ok(()) if allowed, Err(retry_after_secs) if rate limited.
    pub async fn check(&self, ip: &str) -> Result<(), u64> {
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets.entry(ip.to_string()).or_insert_with(IpBucket::new);

        // Reset if window expired
        if bucket.is_expired(self.config.window) {
            bucket.reset();
        }

        // Check auth failure lockout
        if bucket.auth_failures >= self.config.max_auth_failures {
            let remaining = self
                .config
                .window
                .as_secs()
                .saturating_sub(bucket.window_start.elapsed().as_secs());
            return Err(remaining.max(1));
        }

        // Check request count
        if bucket.count >= self.config.max_requests {
            let remaining = self
                .config
                .window
                .as_secs()
                .saturating_sub(bucket.window_start.elapsed().as_secs());
            return Err(remaining.max(1));
        }

        bucket.count += 1;
        Ok(())
    }

    /// Record an auth failure for the given IP.
    pub async fn record_auth_failure(&self, ip: &str) {
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets.entry(ip.to_string()).or_insert_with(IpBucket::new);
        if bucket.is_expired(self.config.window) {
            bucket.reset();
        }
        bucket.auth_failures += 1;
        tracing::warn!(
            ip = ip,
            failures = bucket.auth_failures,
            max = self.config.max_auth_failures,
            "auth failure recorded"
        );
    }

    /// Evict stale entries older than 2x the window.
    pub async fn cleanup(&self) {
        let mut buckets = self.buckets.lock().await;
        let stale_threshold = self.config.window * 2;
        buckets.retain(|_, bucket| !bucket.is_expired(stale_threshold));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limit_allows_within_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 5,
            window: Duration::from_secs(60),
            max_auth_failures: 3,
        });

        for _ in 0..5 {
            assert!(limiter.check("127.0.0.1").await.is_ok());
        }
        // 6th request should be denied
        assert!(limiter.check("127.0.0.1").await.is_err());
    }

    #[tokio::test]
    async fn test_rate_limit_separate_ips() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 2,
            window: Duration::from_secs(60),
            max_auth_failures: 3,
        });

        assert!(limiter.check("10.0.0.1").await.is_ok());
        assert!(limiter.check("10.0.0.1").await.is_ok());
        assert!(limiter.check("10.0.0.1").await.is_err());

        // Different IP should still be allowed
        assert!(limiter.check("10.0.0.2").await.is_ok());
    }

    #[tokio::test]
    async fn test_auth_failure_lockout() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 100,
            window: Duration::from_secs(60),
            max_auth_failures: 3,
        });

        limiter.record_auth_failure("10.0.0.1").await;
        limiter.record_auth_failure("10.0.0.1").await;
        assert!(limiter.check("10.0.0.1").await.is_ok()); // Still under limit
        limiter.record_auth_failure("10.0.0.1").await;
        assert!(limiter.check("10.0.0.1").await.is_err()); // Locked out
    }

    #[tokio::test]
    async fn test_window_expiry_resets_count() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 2,
            window: Duration::from_millis(50),
            max_auth_failures: 3,
        });

        assert!(limiter.check("10.0.0.1").await.is_ok());
        assert!(limiter.check("10.0.0.1").await.is_ok());
        assert!(limiter.check("10.0.0.1").await.is_err());

        // Wait for window to expire
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(limiter.check("10.0.0.1").await.is_ok()); // Reset
    }

    #[tokio::test]
    async fn test_cleanup_removes_stale() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 100,
            window: Duration::from_millis(10),
            max_auth_failures: 3,
        });

        limiter.check("10.0.0.1").await.ok();
        limiter.check("10.0.0.2").await.ok();

        // Wait for entries to become stale
        tokio::time::sleep(Duration::from_millis(30)).await;
        limiter.cleanup().await;

        let buckets = limiter.buckets.lock().await;
        assert!(buckets.is_empty(), "stale entries should be evicted");
    }
}
