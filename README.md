# agent-rate-limiter

[![CI](https://github.com/MukundaKatta/agent-rate-limiter-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/MukundaKatta/agent-rate-limiter-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A small, **dependency-free** sliding-window rate limiter for Rust. It was built
to throttle LLM agent API calls (e.g. "no more than 10 requests per minute"),
but it works for any "at most `N` events per time window" guard.

- **No dependencies** — pure `std`.
- **Sliding window** — a call at time `t` counts against the window
  `(now - window, now]`; it stops counting the instant it ages out.
- **Deterministic clock** — the limiter owns an internal millisecond clock, so
  behaviour is fully reproducible and easy to unit test (no `sleep`, no flaky
  wall-clock dependence).
- **Actionable rejection** — when a slot is unavailable, [`acquire`] returns the
  exact number of milliseconds to wait before retrying.

## Installation

Add it to your `Cargo.toml`:

```toml
[dependencies]
agent-rate-limiter = { git = "https://github.com/MukundaKatta/agent-rate-limiter-rs" }
```

(The crate is named `agent-rate-limiter`; import it in code as
`agent_rate_limiter`.)

## Usage

```rust
use agent_rate_limiter::RateLimiter;

// Allow at most 10 calls per 60s window.
let mut limiter = RateLimiter::new(10, 60_000);

// `try_acquire` returns a bool — no error type to handle.
for _ in 0..10 {
    assert!(limiter.try_acquire());
}
assert!(!limiter.try_acquire()); // window is full

// Advance the deterministic clock past the window; slots free up again.
limiter.advance(60_000);
assert!(limiter.try_acquire());
```

### Reacting to rejection with `acquire`

`acquire` returns `Result<(), RateLimitExceeded>`. On failure the error carries
`retry_after_ms` — the exact delay after which a slot is guaranteed to be free:

```rust
use agent_rate_limiter::RateLimiter;

let mut limiter = RateLimiter::new(1, 5_000);
limiter.acquire().unwrap();

match limiter.acquire() {
    Ok(()) => { /* slot acquired */ }
    Err(e) => {
        println!("rate limited, retry after {} ms", e.retry_after_ms);
        // In real code you'd sleep; here we just advance the test clock.
        limiter.advance(e.retry_after_ms);
        assert!(limiter.acquire().is_ok());
    }
}
```

### Driving the clock in production

This limiter does not read the system clock itself, which keeps it allocation-
and dependency-free and trivially testable. In a real service you advance it
from your own time source, for example at the start of each request:

```rust
use std::time::Instant;
use agent_rate_limiter::RateLimiter;

let start = Instant::now();
let mut limiter = RateLimiter::new(100, 1_000);

// On each incoming request:
let now_ms = start.elapsed().as_millis() as u64;
limiter.set_now(now_ms);
if limiter.try_acquire() {
    // handle the request
} else {
    // reject / queue
}
```

## API

Construct with `RateLimiter::new(limit, window_ms)`.

| Method | Description |
| --- | --- |
| `try_acquire(&mut self) -> bool` | Acquire a slot, returning `true` on success. |
| `acquire(&mut self) -> Result<(), RateLimitExceeded>` | Acquire a slot or return an error with `retry_after_ms`. |
| `can_acquire(&self) -> bool` | Whether a slot is currently available (does not consume one). |
| `current_count(&self) -> usize` | Number of calls currently inside the window. |
| `remaining(&self) -> usize` | Slots left in the current window. |
| `advance(&mut self, ms: u64)` | Move the internal clock forward by `ms` milliseconds. |
| `set_now(&mut self, ms: u64)` | Set the internal clock to an absolute millisecond value. |
| `reset(&mut self)` | Clear all recorded calls and reset the clock to `0`. |
| `limit(&self) -> usize` | The configured per-window limit. |
| `window_ms(&self) -> u64` | The configured window size in milliseconds. |

`RateLimitExceeded` is a `std::error::Error` with public fields `limit`,
`window_ms`, and `retry_after_ms`.

## Testing

```sh
cargo test          # unit, integration, and doc tests
cargo fmt --check   # formatting
cargo clippy --all-targets -- -D warnings
```

## License

Licensed under the [MIT License](LICENSE).
