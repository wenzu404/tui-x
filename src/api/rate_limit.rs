use rand::Rng;
use std::time::Duration;

/// Add jitter to a base delay (±50%).
pub fn with_jitter(base_ms: u64) -> Duration {
    let mut rng = rand::rng();
    let half = base_ms / 2;
    let jittered = rng.random_range(half..=(base_ms + half));
    Duration::from_millis(jittered)
}

/// Random delay for write operations (uniform between min and max).
pub fn write_delay(min_ms: u64, max_ms: u64) -> Duration {
    let mut rng = rand::rng();
    Duration::from_millis(rng.random_range(min_ms..=max_ms))
}

/// Exponential backoff: min(2 * 2^attempt + random(0,1000), 60000) ms.
pub fn backoff(attempt: u32) -> Duration {
    let mut rng = rand::rng();
    let base = 2000u64 * 2u64.saturating_pow(attempt);
    let jitter = rng.random_range(0..1000);
    let delay = (base + jitter).min(60_000);
    Duration::from_millis(delay)
}
