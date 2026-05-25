/*!
agent-rate-limiter: token bucket rate limiter for LLM agent API calls.

```rust
use agent_rate_limiter::RateLimiter;

// Allow 10 calls per 60s window
let mut limiter = RateLimiter::new(10, 60_000);
for _ in 0..10 {
    assert!(limiter.try_acquire());
}
assert!(!limiter.try_acquire()); // exhausted
```
*/

use std::collections::VecDeque;
use std::fmt;

/// Raised when rate limit is exceeded.
#[derive(Debug)]
pub struct RateLimitExceeded {
    pub limit: usize,
    pub window_ms: u64,
    pub retry_after_ms: u64,
}

impl fmt::Display for RateLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rate limit exceeded: {}/{} ms, retry after {} ms", self.limit, self.window_ms, self.retry_after_ms)
    }
}

impl std::error::Error for RateLimitExceeded {}

/// A sliding-window rate limiter.
pub struct RateLimiter {
    /// Max calls per window.
    limit: usize,
    /// Window size in milliseconds.
    window_ms: u64,
    /// Timestamps of recent calls (monotonic ms, mocked by counter here).
    calls: VecDeque<u64>,
    /// Internal clock (incremented on demand for deterministic tests).
    now_ms: u64,
}

impl RateLimiter {
    pub fn new(limit: usize, window_ms: u64) -> Self {
        Self { limit, window_ms, calls: VecDeque::new(), now_ms: 0 }
    }

    /// Advance the internal clock by `ms` milliseconds.
    pub fn advance(&mut self, ms: u64) {
        self.now_ms += ms;
        self.evict_old();
    }

    /// Set clock to an absolute ms value.
    pub fn set_now(&mut self, ms: u64) {
        self.now_ms = ms;
        self.evict_old();
    }

    fn evict_old(&mut self) {
        let cutoff = self.now_ms.saturating_sub(self.window_ms);
        while let Some(&front) = self.calls.front() {
            if front < cutoff {
                self.calls.pop_front();
            } else {
                break;
            }
        }
    }

    /// Number of calls in the current window.
    pub fn current_count(&self) -> usize {
        self.calls.len()
    }

    /// True if a call can be made now.
    pub fn can_acquire(&self) -> bool {
        self.calls.len() < self.limit
    }

    /// Try to acquire a slot. Returns true on success.
    pub fn try_acquire(&mut self) -> bool {
        self.evict_old();
        if self.calls.len() < self.limit {
            self.calls.push_back(self.now_ms);
            true
        } else {
            false
        }
    }

    /// Acquire or return Err with retry delay.
    pub fn acquire(&mut self) -> Result<(), RateLimitExceeded> {
        if self.try_acquire() {
            Ok(())
        } else {
            let oldest = *self.calls.front().unwrap_or(&self.now_ms);
            let retry_after_ms = (oldest + self.window_ms).saturating_sub(self.now_ms);
            Err(RateLimitExceeded { limit: self.limit, window_ms: self.window_ms, retry_after_ms })
        }
    }

    pub fn limit(&self) -> usize { self.limit }
    pub fn window_ms(&self) -> u64 { self.window_ms }

    /// Remaining slots in current window.
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.calls.len())
    }

    /// Reset all counters.
    pub fn reset(&mut self) {
        self.calls.clear();
        self.now_ms = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_acquire_fills_window() {
        let mut r = RateLimiter::new(3, 60_000);
        assert!(r.try_acquire());
        assert!(r.try_acquire());
        assert!(r.try_acquire());
        assert!(!r.try_acquire());
    }

    #[test]
    fn window_slide_frees_slots() {
        let mut r = RateLimiter::new(3, 60_000);
        r.try_acquire();
        r.try_acquire();
        r.try_acquire();
        r.advance(61_000);
        assert!(r.try_acquire());
    }

    #[test]
    fn acquire_ok_under_limit() {
        let mut r = RateLimiter::new(5, 60_000);
        assert!(r.acquire().is_ok());
    }

    #[test]
    fn acquire_err_over_limit() {
        let mut r = RateLimiter::new(1, 60_000);
        r.acquire().unwrap();
        assert!(r.acquire().is_err());
    }

    #[test]
    fn retry_after_positive() {
        let mut r = RateLimiter::new(1, 60_000);
        r.acquire().unwrap();
        let err = r.acquire().unwrap_err();
        assert!(err.retry_after_ms > 0);
        assert!(err.retry_after_ms <= 60_000);
    }

    #[test]
    fn remaining_decrements() {
        let mut r = RateLimiter::new(5, 60_000);
        assert_eq!(r.remaining(), 5);
        r.try_acquire();
        assert_eq!(r.remaining(), 4);
    }

    #[test]
    fn reset_clears_state() {
        let mut r = RateLimiter::new(3, 60_000);
        r.try_acquire();
        r.try_acquire();
        r.reset();
        assert_eq!(r.current_count(), 0);
        assert_eq!(r.remaining(), 3);
    }

    #[test]
    fn current_count_after_slide() {
        let mut r = RateLimiter::new(5, 1000);
        r.try_acquire();
        r.try_acquire();
        r.advance(2000);
        assert_eq!(r.current_count(), 0);
    }

    #[test]
    fn error_display() {
        let e = RateLimitExceeded { limit: 10, window_ms: 60000, retry_after_ms: 30000 };
        assert!(e.to_string().contains("10"));
    }

    #[test]
    fn can_acquire_true_when_slots_available() {
        let r = RateLimiter::new(5, 60_000);
        assert!(r.can_acquire());
    }

    #[test]
    fn can_acquire_false_when_full() {
        let mut r = RateLimiter::new(1, 60_000);
        r.try_acquire();
        assert!(!r.can_acquire());
    }

    #[test]
    fn limit_zero_never_allows() {
        let mut r = RateLimiter::new(0, 60_000);
        assert!(!r.try_acquire());
    }

    #[test]
    fn partial_slide_keeps_recent_calls() {
        let mut r = RateLimiter::new(5, 10_000);
        r.try_acquire(); // at t=0
        r.advance(5_000);
        r.try_acquire(); // at t=5000
        r.advance(6_000); // now at t=11000, t=0 call evicted
        assert_eq!(r.current_count(), 1); // t=5000 call still in window
    }

    #[test]
    fn getters() {
        let r = RateLimiter::new(10, 30_000);
        assert_eq!(r.limit(), 10);
        assert_eq!(r.window_ms(), 30_000);
    }
}
