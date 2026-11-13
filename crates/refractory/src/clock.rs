//! Where the limiter gets the time from.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::gcra::nanos;

/// A source of time: the time elapsed since some fixed epoch.
///
/// The epoch is up to the clock, but it must not move, and the time must
/// not go backwards by much (a step back only makes the limiter stricter
/// for a moment).
pub trait Clock: Send + Sync + 'static {
    /// The time since this clock's epoch.
    fn now(&self) -> Duration;
}

/// The default clock: a monotonic [`Instant`], counted from the moment the
/// clock was made. Unaffected by changes to the system time. Right for a
/// store that lives in the process, like [`MemoryStore`](crate::MemoryStore).
#[derive(Clone, Copy, Debug)]
pub struct MonotonicClock {
    start: Instant,
}

impl MonotonicClock {
    /// A clock whose epoch is now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Wall-clock time since the Unix epoch. For a store shared by several
/// processes or kept across restarts, where a per-process epoch would not
/// do; the machines' clocks then need to agree (NTP).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
    }
}

/// A clock that only moves when told to, for tests. Clones share the time.
///
/// ```
/// use std::time::Duration;
/// use refractory::{Limiter, ManualClock, Rule};
///
/// const RULE: Rule = Rule::new("demo", 1, Duration::from_secs(60));
///
/// let clock = ManualClock::new();
/// let limiter = Limiter::builder().clock(clock.clone()).build();
///
/// assert!(limiter.check_sync(&RULE, "key").is_ok());
/// assert!(limiter.check_sync(&RULE, "key").is_err());
/// clock.advance(Duration::from_secs(60));
/// assert!(limiter.check_sync(&RULE, "key").is_ok());
/// ```
#[derive(Clone, Debug, Default)]
pub struct ManualClock {
    nanos: Arc<AtomicU64>,
}

impl ManualClock {
    /// A clock that stands at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        self.nanos.fetch_add(nanos(by), Ordering::SeqCst);
    }

    /// Sets the clock to `to` since its epoch.
    pub fn set(&self, to: Duration) {
        self.nanos.store(nanos(to), Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.nanos.load(Ordering::SeqCst))
    }
}
