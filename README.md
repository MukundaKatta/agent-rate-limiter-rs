# agent-rate-limiter

A small, dependency-free **token/sliding-window rate limiter** for throttling LLM agent API calls in Rust.

It tracks the timestamps of recent calls inside a fixed time window and rejects requests once the configured limit is reached for that window. When an attempt is rejected, you get back a structured error that includes how long to wait before retrying.

## Features

- **Sliding-window limiting** — a call is allowed only if fewer than `limit` calls fall inside the trailing `window_ms` window.
- **Deterministic, testable clock** — advance time manually with `advance` / `set_now`, so behavior is fully reproducible in tests (no wall-clock flakiness).
- **Structured rejection** — `acquire` returns `Result<(), RateLimitExceeded>`, where the error carries `limit`, `window_ms`, and `retry_after_ms`.
- **Zero dependencies** — pure standard library (`std::collections::VecDeque`).

## Installation

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
agent-rate-limiter = "0.1"
```

## Usage

```rust
use agent_rate_limiter::RateLimiter;

// Allow at most 10 calls per 60s window.
let mut limiter = RateLimiter::new(10, 60_000);

for _ in 0..10 {
    assert!(limiter.try_acquire());
}

// The 11th call within the window is rejected.
assert!(!limiter.try_acquire());
```

### Structured errors with retry timing

```rust
use agent_rate_limiter::RateLimiter;

let mut limiter = RateLimiter::new(1, 60_000);
limiter.acquire().unwrap();

match limiter.acquire() {
    Ok(()) => { /* proceed with the API call */ }
    Err(e) => {
        eprintln!("{e}");
        // wait e.retry_after_ms before trying again
    }
}
```

### Advancing the clock

The limiter uses an internal millisecond clock that you advance explicitly, which makes window-sliding behavior easy to reason about and test:

```rust
use agent_rate_limiter::RateLimiter;

let mut limiter = RateLimiter::new(3, 60_000);
for _ in 0..3 {
    limiter.try_acquire();
}
assert!(!limiter.try_acquire());      // window full

limiter.advance(61_000);              // old calls fall out of the window
assert!(limiter.try_acquire());       // slot freed
```

## API overview

| Method | Description |
| --- | --- |
| `RateLimiter::new(limit, window_ms)` | Create a limiter allowing `limit` calls per `window_ms`. |
| `try_acquire() -> bool` | Attempt to take a slot; returns `true` on success. |
| `acquire() -> Result<(), RateLimitExceeded>` | Take a slot or return a structured error with `retry_after_ms`. |
| `can_acquire() -> bool` | Whether a slot is currently available (no state change). |
| `remaining() -> usize` | Slots left in the current window. |
| `current_count() -> usize` | Calls currently inside the window. |
| `advance(ms)` / `set_now(ms)` | Advance / set the internal clock and evict expired calls. |
| `limit()` / `window_ms()` | Read back the configuration. |
| `reset()` | Clear all recorded calls and reset the clock. |

## Tech stack

- **Language:** Rust (edition 2021)
- **Dependencies:** none (standard library only)
- **License:** MIT

## Development

```sh
cargo build
cargo test
```

## License

Licensed under the [MIT](LICENSE) license.
