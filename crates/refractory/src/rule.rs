use core::fmt;
use core::time::Duration;

/// How often something may happen: at most `limit` times per `window`.
///
/// A rule is plain data and `const`-constructible, so the limits of an app
/// can live next to each other as constants:
///
/// ```
/// use std::time::Duration;
/// use refractory::Rule;
///
/// const LOGIN: Rule = Rule::new("login", 10, Duration::from_secs(3 * 60));
/// const LOGIN_PER_ACCOUNT: Rule = Rule::new("login-account", 20, Duration::from_secs(60 * 60));
/// const REGISTRATION: Rule = Rule::new("registration", 5, Duration::from_secs(60 * 60));
///
/// assert_eq!(LOGIN.limit(), 10);
/// assert_eq!(LOGIN.interval(), Duration::from_secs(18));
/// ```
///
/// The name keeps the rule's counters apart from those of other rules, so
/// the same key (an address, say) is counted separately for the login and
/// for the registration. Give every rule its own name.
///
/// # Semantics
///
/// `limit` events are allowed at once (a burst exactly at the limit is
/// fine, one more is refused). After that, one more is allowed every
/// [`interval`](Rule::interval) (`window / limit`), and a key that keeps
/// quiet for the whole `window` has its full burst back. See the
/// [crate documentation](crate#the-algorithm) for the algorithm.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rule {
    name: &'static str,
    limit: u32,
    window: Duration,
}

impl Rule {
    /// A rule that allows `limit` events per `window`.
    ///
    /// # Panics
    ///
    /// If `limit` is zero, if `window` is shorter than `limit` nanoseconds,
    /// or longer than `u64::MAX` nanoseconds (about 584 years). In a `const`
    /// these are compile errors.
    ///
    /// ```compile_fail
    /// # use std::time::Duration;
    /// # use refractory::Rule;
    /// const NEVER: Rule = Rule::new("never", 0, Duration::from_secs(60));
    /// # let _ = NEVER;
    /// ```
    #[must_use]
    #[allow(clippy::cast_lossless)] // `u128::from` is not const.
    pub const fn new(name: &'static str, limit: u32, window: Duration) -> Self {
        assert!(limit > 0, "a rate limit rule must allow at least one event");
        assert!(
            window.as_nanos() >= limit as u128,
            "a rate limit window must be at least `limit` nanoseconds long"
        );
        assert!(
            window.as_nanos() <= u64::MAX as u128,
            "a rate limit window must be shorter than u64::MAX nanoseconds"
        );
        Self {
            name,
            limit,
            window,
        }
    }

    /// The name that keeps this rule's counters apart from other rules'.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// How many events are allowed per [`window`](Rule::window).
    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }

    /// The period the [`limit`](Rule::limit) applies to.
    #[must_use]
    pub const fn window(&self) -> Duration {
        self.window
    }

    /// The time it takes to earn back one event: `window / limit`, rounded
    /// down to whole nanoseconds.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        Duration::from_nanos(self.interval_nanos())
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_lossless)]
    pub(crate) const fn interval_nanos(&self) -> u64 {
        // `new` checked that the window fits in a u64.
        (self.window.as_nanos() / self.limit as u128) as u64
    }
}

impl fmt::Debug for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Rule({:?}: {} per {:?})",
            self.name, self.limit, self.window
        )
    }
}
