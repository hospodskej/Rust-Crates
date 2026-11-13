//! Where the counters live.

use core::fmt;
use core::future::Future;
use core::time::Duration;

use crate::{Exceeded, Rule};

/// What a [`Store`] keeps the state under: a 128-bit keyed hash of the rule's
/// name and the caller's key (an address, an e-mail, ...), made by
/// [`Limiter::key`](crate::Limiter::key).
///
/// The store never sees the original key. Its [`Display`](fmt::Display)
/// form is 32 hex digits, for stores that need a string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(u128);

impl Key {
    /// A key from its 128 bits.
    #[must_use]
    pub const fn from_u128(bits: u128) -> Self {
        Self(bits)
    }

    /// The key's 128 bits.
    #[must_use]
    pub const fn as_u128(self) -> u128 {
        self.0
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key({self})")
    }
}

/// Storage for the limiter's state: one theoretical arrival time per key
/// (see [`gcra`](crate::gcra)).
///
/// [`MemoryStore`](crate::MemoryStore) is the default and the right choice
/// for an app that runs as one process. A store shared by several processes
/// (SQLite, Redis, ...) can be added by implementing this trait; the method
/// returns a future so such a store can do I/O. It must apply
/// [`gcra::step`](crate::gcra::step) atomically per key: read the TAT, step,
/// and write the new TAT only if the event was allowed, with no other event
/// for the same key in between.
///
/// ```
/// use std::collections::HashMap;
/// use std::convert::Infallible;
/// use std::sync::Mutex;
/// use std::time::Duration;
/// use refractory::{Exceeded, Key, Rule, Store, gcra};
///
/// /// The simplest store: one map behind one lock, never cleaned up.
/// #[derive(Default)]
/// struct TinyStore(Mutex<HashMap<Key, Duration>>);
///
/// impl Store for TinyStore {
///     type Error = Infallible;
///
///     fn check(
///         &self,
///         key: Key,
///         rule: Rule,
///         now: Duration,
///     ) -> impl Future<Output = Result<Result<(), Exceeded>, Infallible>> + Send {
///         let mut map = self.0.lock().unwrap();
///         let result = gcra::step(&rule, map.get(&key).copied(), now).map(|tat| {
///             map.insert(key, tat);
///         });
///         std::future::ready(Ok(result))
///     }
/// }
///
/// const RULE: Rule = Rule::new("demo", 1, Duration::from_secs(60));
/// let limiter = refractory::Limiter::builder().store(TinyStore::default()).build();
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
/// assert!(limiter.check(&RULE, "key").await.is_ok());
/// assert!(limiter.check(&RULE, "key").await.is_err());
/// # });
/// ```
pub trait Store: Send + Sync + 'static {
    /// Why the store could not answer (a lost database connection, say).
    type Error: Send + 'static;

    /// Counts one event for `key` under `rule` at time `now` (from the
    /// limiter's [`Clock`](crate::Clock)), if the rule allows it.
    ///
    /// The outer `Result` is the store's own failure, the inner one the
    /// limiter's answer.
    fn check(
        &self,
        key: Key,
        rule: Rule,
        now: Duration,
    ) -> impl Future<Output = Result<Result<(), Exceeded>, Self::Error>> + Send;
}
