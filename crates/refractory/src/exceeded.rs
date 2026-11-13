use core::fmt;
use core::time::Duration;

use crate::Rule;

/// A refused event: the key has used up the rule's limit for now.
///
/// ```
/// use std::time::Duration;
/// use refractory::{Exceeded, Rule};
///
/// const LOGIN: Rule = Rule::new("login", 10, Duration::from_secs(180));
/// let refused = Exceeded::new(&LOGIN, Duration::from_millis(17_200));
/// assert_eq!(refused.retry_after_secs(), 18);
/// assert_eq!(refused.to_string(), "rate limit `login` exceeded, retry in 18 s");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Exceeded {
    /// The [name](Rule::name) of the rule that refused.
    pub rule: &'static str,
    /// How long until the key may try again (one more event will be
    /// allowed then).
    pub retry_after: Duration,
}

impl Exceeded {
    /// A refusal by `rule`, to be retried after `retry_after`. For
    /// [`Store`](crate::Store) implementations that do the arithmetic
    /// themselves (in a database, say).
    #[must_use]
    pub const fn new(rule: &Rule, retry_after: Duration) -> Self {
        Self {
            rule: rule.name(),
            retry_after,
        }
    }

    /// [`retry_after`](Exceeded::retry_after) in whole seconds, rounded up
    /// and at least 1, as the `Retry-After` header wants it.
    #[must_use]
    pub fn retry_after_secs(&self) -> u64 {
        let secs = self.retry_after.as_secs();
        let secs = if self.retry_after.subsec_nanos() > 0 {
            secs.saturating_add(1)
        } else {
            secs
        };
        secs.max(1)
    }
}

impl fmt::Display for Exceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rate limit `{}` exceeded, retry in {} s",
            self.rule,
            self.retry_after_secs()
        )
    }
}

impl std::error::Error for Exceeded {}
