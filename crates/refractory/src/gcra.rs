//! The Generic Cell Rate Algorithm, for [`Store`](crate::Store)
//! implementations.
//!
//! Per key, a store keeps one number: the *theoretical arrival time* (TAT),
//! the moment at which the key would have used up nothing at all. Each
//! allowed event pushes it [`Rule::interval`] further into the future. An
//! event is allowed while the TAT, after pushing it, is no further than
//! `limit × interval` (the window) ahead of now; otherwise the event is
//! refused, nothing is stored, and the wait is how far it sticks out.
//!
//! A key whose TAT has passed is in the same state as a key never seen,
//! so a store can forget it then (for example with Redis' `PEXPIREAT`).
//!
//! ```
//! use std::time::Duration;
//! use refractory::{Rule, gcra};
//!
//! let rule = Rule::new("demo", 2, Duration::from_secs(10)); // one per 5 s
//! let now = Duration::from_secs(100);
//!
//! let tat = gcra::step(&rule, None, now).unwrap();
//! assert_eq!(tat, Duration::from_secs(105));
//! let tat = gcra::step(&rule, Some(tat), now).unwrap();
//! assert_eq!(tat, Duration::from_secs(110));
//!
//! let refused = gcra::step(&rule, Some(tat), now).unwrap_err();
//! assert_eq!(refused.retry_after, Duration::from_secs(5));
//!
//! assert!(!gcra::is_idle(tat, Duration::from_secs(109)));
//! assert!(gcra::is_idle(tat, Duration::from_secs(110)));
//! ```

use core::time::Duration;

use crate::{Exceeded, Rule};

/// One event for a key under `rule` at time `now`.
///
/// `tat` is the key's stored theoretical arrival time, `None` for a key the
/// store has not seen (or has forgotten). Returns the TAT to store if the
/// event is allowed, or the refusal (store nothing then).
///
/// # Errors
///
/// [`Exceeded`] when the key has used up its limit; its `retry_after` is
/// the time until the next event would be allowed.
pub fn step(rule: &Rule, tat: Option<Duration>, now: Duration) -> Result<Duration, Exceeded> {
    let now = nanos(now);
    let interval = rule.interval_nanos();
    // `interval * limit <= window <= u64::MAX`, so this cannot overflow.
    let burst = interval * u64::from(rule.limit());
    let tat = tat.map_or(now, nanos).max(now);
    let new_tat = tat.saturating_add(interval);
    let allowed_at = new_tat.saturating_sub(burst);
    if allowed_at <= now {
        Ok(Duration::from_nanos(new_tat))
    } else {
        Err(Exceeded::new(rule, Duration::from_nanos(allowed_at - now)))
    }
}

/// Whether a key with this TAT is back to its full allowance at `now`, and
/// so can be forgotten.
#[must_use]
pub fn is_idle(tat: Duration, now: Duration) -> bool {
    tat <= now
}

/// A duration in whole nanoseconds, saturating at `u64::MAX` (584 years).
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn nanos(d: Duration) -> u64 {
    d.as_nanos().min(u128::from(u64::MAX)) as u64
}
