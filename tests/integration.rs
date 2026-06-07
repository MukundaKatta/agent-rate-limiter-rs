//! Integration tests exercising the public API of `agent-rate-limiter`.
//!
//! These mirror real usage: drive the deterministic clock and assert that the
//! limiter blocks, frees slots on the window boundary, and reports an accurate
//! `retry_after_ms`.

use agent_rate_limiter::{RateLimitExceeded, RateLimiter};

#[test]
fn end_to_end_throttle_cycle() {
    let mut limiter = RateLimiter::new(3, 1_000);

    // Burst up to the limit.
    assert_eq!(limiter.remaining(), 3);
    assert!(limiter.try_acquire());
    assert!(limiter.try_acquire());
    assert!(limiter.try_acquire());
    assert_eq!(limiter.remaining(), 0);
    assert!(!limiter.can_acquire());
    assert!(!limiter.try_acquire());

    // The whole window slides past; all three slots free up.
    limiter.advance(1_000);
    assert_eq!(limiter.remaining(), 3);
    assert!(limiter.try_acquire());
}

#[test]
fn acquire_reports_actionable_retry_after() {
    let mut limiter = RateLimiter::new(2, 10_000);
    limiter.acquire().unwrap(); // t = 0
    limiter.advance(3_000); // t = 3_000
    limiter.acquire().unwrap(); // second slot
    let err: RateLimitExceeded = limiter.acquire().unwrap_err();

    // Oldest call was at t = 0, window is 10_000, now is 3_000 => wait 7_000.
    assert_eq!(err.limit, 2);
    assert_eq!(err.window_ms, 10_000);
    assert_eq!(err.retry_after_ms, 7_000);

    // Honour the advice exactly and the next acquire succeeds.
    limiter.advance(err.retry_after_ms);
    assert!(limiter.acquire().is_ok());
}

#[test]
fn staggered_calls_free_one_slot_at_a_time() {
    let mut limiter = RateLimiter::new(2, 1_000);
    limiter.try_acquire(); // t = 0
    limiter.advance(500);
    limiter.try_acquire(); // t = 500
    assert_eq!(limiter.current_count(), 2);

    // At t = 1_000 only the first call has aged out.
    limiter.advance(500); // t = 1_000
    assert_eq!(limiter.current_count(), 1);
    assert_eq!(limiter.remaining(), 1);

    // At t = 1_500 the second call also ages out.
    limiter.advance(500); // t = 1_500
    assert_eq!(limiter.current_count(), 0);
}

#[test]
fn reset_returns_limiter_to_initial_state() {
    let mut limiter = RateLimiter::new(1, 5_000);
    limiter.advance(2_000);
    limiter.try_acquire();
    assert!(!limiter.can_acquire());

    limiter.reset();
    assert_eq!(limiter.current_count(), 0);
    assert_eq!(limiter.remaining(), 1);
    assert!(limiter.can_acquire());
}
