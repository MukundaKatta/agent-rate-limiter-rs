/*!
`agent-rate-limiter` is a small, dependency-free **sliding-window** rate limiter
for throttling LLM agent API calls (or anything else that needs a "no more than
`N` events per window" guard).

A call made at time `t` counts against the window `(now - window, now]`. Once a
call falls out of that window it stops counting, freeing a slot for a new call.

# Quick start

```rust
use agent_rate_limiter::RateLimiter;

// Allow 10 calls per 60s window.
let mut limiter = RateLimiter::new(10, 60_000);
for _ in 0..10 {
    assert!(limiter.try_acquire());
}
assert!(!limiter.try_acquire()); // exhausted
```

# Deterministic clock

The limiter keeps an internal millisecond clock so behaviour is fully
deterministic and testable. Advance it with [`RateLimiter::advance`] (relative)
or [`RateLimiter::set_now`] (absolute):

```rust
use agent_rate_limiter::RateLimiter;

let mut limiter = RateLimiter::new(1, 1_000);
assert!(limiter.try_acquire());   // call at t = 0
assert!(!limiter.try_acquire());  // window full
limiter.advance(1_000);           // t = 1_000, the t = 0 call has expired
assert!(limiter.try_acquire());   // slot freed
```

# Handling rejection

[`RateLimiter::acquire`] returns a [`RateLimitExceeded`] error carrying
`retry_after_ms`, the number of milliseconds to wait before a slot is
guaranteed to be available:

```rust
use agent_rate_limiter::RateLimiter;

let mut limiter = RateLimiter::new(1, 5_000);
limiter.acquire().unwrap();
if let Err(e) = limiter.acquire() {
    // Wait `e.retry_after_ms` (here advancing the deterministic clock) and retry.
    limiter.advance(e.retry_after_ms);
    assert!(limiter.acquire().is_ok());
}
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
        write!(
            f,
            "rate limit exceeded: {}/{} ms, retry after {} ms",
            self.limit, self.window_ms, self.retry_after_ms
        )
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
        Self {
            limit,
            window_ms,
            calls: VecDeque::new(),
            now_ms: 0,
        }
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
        // A call made at time `t` expires once it is at least `window_ms` old,
        // i.e. when `now - t >= window`. Comparing the *age* of the call this
        // way (rather than `t < now - window`) keeps the eviction boundary in
        // step with the `retry_after_ms` value reported by
        // [`RateLimiter::acquire`]: a blocked caller that waits exactly
        // `retry_after_ms` then finds a slot free, instead of being off by one
        // millisecond. Using `saturating_sub` on `now - t` (rather than on
        // `now - window`) also avoids spuriously evicting a just-made call when
        // `window_ms > now_ms`.
        while let Some(&front) = self.calls.front() {
            if self.now_ms.saturating_sub(front) >= self.window_ms {
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

    /// Acquire a slot, or return [`RateLimitExceeded`] when the window is full.
    ///
    /// On failure the error's `retry_after_ms` is the exact number of
    /// milliseconds after which a slot is guaranteed to free up: advancing the
    /// clock by that amount (or sleeping that long in wall-clock usage) and
    /// retrying will succeed, assuming no other call is acquired in the
    /// meantime.
    pub fn acquire(&mut self) -> Result<(), RateLimitExceeded> {
        if self.try_acquire() {
            Ok(())
        } else {
            let oldest = *self.calls.front().unwrap_or(&self.now_ms);
            let retry_after_ms = (oldest + self.window_ms).saturating_sub(self.now_ms);
            Err(RateLimitExceeded {
                limit: self.limit,
                window_ms: self.window_ms,
                retry_after_ms,
            })
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

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
        let e = RateLimitExceeded {
            limit: 10,
            window_ms: 60000,
            retry_after_ms: 30000,
        };
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

    #[test]
    fn call_expires_exactly_at_window_boundary() {
        // A call at t=0 with a 1000ms window should free its slot at exactly
        // t=1000 (window is `(now - window, now]`), not at t=1001.
        let mut r = RateLimiter::new(1, 1_000);
        assert!(r.try_acquire()); // call at t=0
        assert!(!r.try_acquire()); // window full
        r.set_now(999);
        assert!(!r.can_acquire()); // still inside the window
        r.set_now(1_000);
        assert!(r.can_acquire()); // boundary reached, slot freed
        assert!(r.try_acquire());
    }

    #[test]
    fn retry_after_is_exact() {
        // Waiting exactly `retry_after_ms` must free a slot. This guards the
        // off-by-one between the eviction boundary and the reported delay.
        let mut r = RateLimiter::new(1, 5_000);
        r.acquire().unwrap(); // call at t=0
        let err = r.acquire().unwrap_err();
        assert_eq!(err.retry_after_ms, 5_000);
        r.advance(err.retry_after_ms);
        assert!(
            r.acquire().is_ok(),
            "advancing by retry_after_ms should free a slot"
        );
    }

    #[test]
    fn retry_after_shrinks_as_clock_advances() {
        let mut r = RateLimiter::new(1, 10_000);
        r.acquire().unwrap(); // call at t=0
        r.advance(4_000); // t=4000
        let err = r.acquire().unwrap_err();
        assert_eq!(err.retry_after_ms, 6_000);
    }

    #[test]
    fn limit_zero_acquire_reports_window_as_retry() {
        let mut r = RateLimiter::new(0, 7_500);
        let err = r.acquire().unwrap_err();
        assert_eq!(err.limit, 0);
        assert_eq!(err.retry_after_ms, 7_500);
    }

    #[test]
    fn set_now_backwards_does_not_panic() {
        let mut r = RateLimiter::new(2, 1_000);
        r.set_now(10_000);
        r.try_acquire();
        // Moving the clock backwards must not underflow or panic.
        r.set_now(0);
        assert_eq!(r.current_count(), 1);
    }

    #[test]
    fn reset_restores_clock_to_zero() {
        let mut r = RateLimiter::new(3, 1_000);
        r.advance(5_000);
        r.try_acquire();
        r.reset();
        assert_eq!(r.current_count(), 0);
        // After reset a fresh call lands at t=0.
        assert!(r.try_acquire());
        assert_eq!(r.current_count(), 1);
    }

    #[test]
    fn error_is_std_error() {
        // Ensure RateLimitExceeded participates in the std error ecosystem.
        fn as_error(e: RateLimitExceeded) -> Box<dyn std::error::Error> {
            Box::new(e)
        }
        let boxed = as_error(RateLimitExceeded {
            limit: 1,
            window_ms: 1_000,
            retry_after_ms: 500,
        });
        assert!(boxed.to_string().contains("retry after 500"));
    }
}
