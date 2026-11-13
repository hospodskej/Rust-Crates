use core::fmt;
use core::future::Future;
use core::hash::{BuildHasher, Hash, Hasher};
use std::collections::hash_map::RandomState;
use std::sync::Arc;

use siphasher::sip128::{Hasher128, SipHasher13};

use crate::{Clock, Exceeded, Key, MemoryStore, MonotonicClock, Rule, Store};

/// Counts events per rule and key, and says whether one more is allowed.
///
/// A limiter is a handle: clones share the same state, so make one at
/// start-up and put it in the app's state. The default stores the state in
/// memory ([`MemoryStore`]) and reads a monotonic clock ([`MonotonicClock`]);
/// [`Limiter::builder`] changes either.
///
/// ```
/// use std::time::Duration;
/// use refractory::{Limiter, Rule};
///
/// const DISCOUNT_CODE: Rule = Rule::new("discount-code", 10, Duration::from_secs(10 * 60));
///
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
/// let limiter = Limiter::new();
/// for _ in 0..10 {
///     assert!(limiter.check(&DISCOUNT_CODE, "203.0.113.7").await.is_ok());
/// }
/// let refused = limiter.check(&DISCOUNT_CODE, "203.0.113.7").await.unwrap_err();
/// assert_eq!(refused.retry_after_secs(), 60); // 10 minutes / 10
///
/// // Another key is counted on its own.
/// assert!(limiter.check(&DISCOUNT_CODE, "198.51.100.1").await.is_ok());
/// # });
/// ```
pub struct Limiter<S = MemoryStore, C = MonotonicClock> {
    inner: Arc<Inner<S, C>>,
}

struct Inner<S, C> {
    store: S,
    clock: C,
    secret: (u64, u64),
    fail_closed: bool,
}

impl Limiter {
    /// A limiter with a [`MemoryStore`], a [`MonotonicClock`] and a random
    /// secret for hashing keys.
    #[must_use]
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// A builder that starts from the defaults of [`Limiter::new`].
    pub fn builder() -> Builder {
        Builder {
            store: MemoryStore::new(),
            clock: MonotonicClock::new(),
            secret: None,
            fail_closed: false,
        }
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Store, C: Clock> Limiter<S, C> {
    /// Counts one event for `key` under `rule`, if the rule allows it.
    ///
    /// `key` is whatever the events are counted per: an IP address, an
    /// e-mail (see [`normalize_email`](crate::normalize_email)), a user ID,
    /// a tuple of those. Keys are compared by their [`Hash`], so `"a"` and
    /// `String::from("a")` are the same key.
    ///
    /// # Errors
    ///
    /// [`Exceeded`] when the key has used up the rule's limit. The refused
    /// event is not counted.
    ///
    /// If the store fails, the event is allowed, unless the limiter was
    /// built with [`Builder::fail_closed`]. Use [`Limiter::try_check`] to
    /// see store errors.
    pub fn check(
        &self,
        rule: &Rule,
        key: impl Hash,
    ) -> impl Future<Output = Result<(), Exceeded>> + Send {
        self.check_key(rule, self.key(rule, key))
    }

    /// [`Limiter::check`] with a key already made by [`Limiter::key`].
    ///
    /// # Errors
    ///
    /// As [`Limiter::check`].
    pub fn check_key(
        &self,
        rule: &Rule,
        key: Key,
    ) -> impl Future<Output = Result<(), Exceeded>> + Send + use<'_, S, C> {
        let rule = *rule;
        let fail_closed = self.inner.fail_closed;
        let checked = self.try_check_key(rule, key);
        async move {
            match checked.await {
                Ok(result) => result,
                Err(_) if fail_closed => Err(Exceeded::new(&rule, rule.interval())),
                Err(_) => Ok(()),
            }
        }
    }

    /// [`Limiter::check`], with the store's own errors.
    ///
    /// # Errors
    ///
    /// The outer error is the store's, the inner one the limiter's answer.
    pub fn try_check(
        &self,
        rule: &Rule,
        key: impl Hash,
    ) -> impl Future<Output = Result<Result<(), Exceeded>, S::Error>> + Send {
        self.try_check_key(*rule, self.key(rule, key))
    }

    fn try_check_key(
        &self,
        rule: Rule,
        key: Key,
    ) -> impl Future<Output = Result<Result<(), Exceeded>, S::Error>> + Send + use<'_, S, C> {
        let now = self.inner.clock.now();
        self.inner.store.check(key, rule, now)
    }
}

impl<S, C> Limiter<S, C> {
    /// The key the store keeps `key`'s state under for `rule`: a 128-bit
    /// SipHash-1-3 of the rule's name and `key`, with the limiter's secret.
    #[must_use]
    pub fn key(&self, rule: &Rule, key: impl Hash) -> Key {
        let (k0, k1) = self.inner.secret;
        let mut hasher = SipHasher13::new_with_keys(k0, k1);
        rule.name().hash(&mut hasher);
        key.hash(&mut hasher);
        Key::from_u128(hasher.finish128().as_u128())
    }

    /// The limiter's store.
    #[must_use]
    pub fn store(&self) -> &S {
        &self.inner.store
    }

    /// The limiter's clock.
    #[must_use]
    pub fn clock(&self) -> &C {
        &self.inner.clock
    }
}

impl<C: Clock> Limiter<MemoryStore, C> {
    /// [`Limiter::check`] without the future, for the in-memory store,
    /// which answers at once. For code that is not async.
    ///
    /// # Errors
    ///
    /// [`Exceeded`] when the key has used up the rule's limit.
    ///
    /// ```
    /// use std::time::Duration;
    /// use refractory::{Limiter, Rule};
    ///
    /// const INQUIRY: Rule = Rule::new("inquiry", 5, Duration::from_secs(600));
    /// let limiter = Limiter::new();
    /// assert!((0..5).all(|_| limiter.check_sync(&INQUIRY, "203.0.113.7").is_ok()));
    /// assert!(limiter.check_sync(&INQUIRY, "203.0.113.7").is_err());
    /// ```
    pub fn check_sync(&self, rule: &Rule, key: impl Hash) -> Result<(), Exceeded> {
        let key = self.key(rule, key);
        self.inner
            .store
            .check_sync(key, rule, self.inner.clock.now())
    }

    /// Forgets every key that has its full allowance back, in all shards
    /// of the store now. The store also does this by itself as it is used;
    /// call this from a periodic task to give memory back after a flood
    /// even when no requests come.
    pub fn purge(&self) {
        self.inner.store.purge(self.inner.clock.now());
    }
}

impl<S, C> Clone for Limiter<S, C> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S: fmt::Debug, C: fmt::Debug> fmt::Debug for Limiter<S, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Limiter")
            .field("store", &self.inner.store)
            .field("clock", &self.inner.clock)
            .field("fail_closed", &self.inner.fail_closed)
            .finish_non_exhaustive()
    }
}

/// Configures a [`Limiter`]; from [`Limiter::builder`].
///
/// ```
/// use refractory::{Limiter, ManualClock, MemoryStore};
///
/// let limiter = Limiter::builder()
///     .store(MemoryStore::new().max_keys(50_000))
///     .clock(ManualClock::new())
///     .build();
/// # let _ = limiter;
/// ```
#[must_use]
pub struct Builder<S = MemoryStore, C = MonotonicClock> {
    store: S,
    clock: C,
    secret: Option<[u8; 16]>,
    fail_closed: bool,
}

impl<S, C> Builder<S, C> {
    /// Where the state is kept (default: a [`MemoryStore`]).
    pub fn store<S2: Store>(self, store: S2) -> Builder<S2, C> {
        Builder {
            store,
            clock: self.clock,
            secret: self.secret,
            fail_closed: self.fail_closed,
        }
    }

    /// Where the time comes from (default: a [`MonotonicClock`]).
    pub fn clock<C2: Clock>(self, clock: C2) -> Builder<S, C2> {
        Builder {
            store: self.store,
            clock,
            secret: self.secret,
            fail_closed: self.fail_closed,
        }
    }

    /// The secret keys are hashed with (default: random, made for each
    /// limiter). Set the same secret in every process that shares a store,
    /// and keep it out of the store.
    pub fn secret(mut self, secret: [u8; 16]) -> Self {
        self.secret = Some(secret);
        self
    }

    /// Whether to refuse events while the store fails (default: no, allow
    /// them). A refusal then asks to retry after one
    /// [`interval`](Rule::interval).
    pub fn fail_closed(mut self, fail_closed: bool) -> Self {
        self.fail_closed = fail_closed;
        self
    }

    /// The limiter.
    pub fn build(self) -> Limiter<S, C> {
        let secret = self.secret.map_or_else(random_secret, |bytes| {
            let (k0, k1) = bytes.split_at(8);
            (
                u64::from_le_bytes(k0.try_into().unwrap_or_default()),
                u64::from_le_bytes(k1.try_into().unwrap_or_default()),
            )
        });
        Limiter {
            inner: Arc::new(Inner {
                store: self.store,
                clock: self.clock,
                secret,
                fail_closed: self.fail_closed,
            }),
        }
    }
}

impl<S: fmt::Debug, C: fmt::Debug> fmt::Debug for Builder<S, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("store", &self.store)
            .field("clock", &self.clock)
            .field("fail_closed", &self.fail_closed)
            .finish_non_exhaustive()
    }
}

/// 128 random bits from the standard library's randomly seeded hasher
/// (seeded from the operating system once per process, then varied).
fn random_secret() -> (u64, u64) {
    let state = RandomState::new();
    let mut a = state.build_hasher();
    a.write_u8(0);
    let mut b = state.build_hasher();
    b.write_u8(1);
    (a.finish(), b.finish())
}
