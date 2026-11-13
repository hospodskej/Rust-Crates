use core::convert::Infallible;
use core::future::{Future, ready};
use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::gcra::{self, nanos};
use crate::{Exceeded, Key, Rule, Store};

const SHARDS: usize = 32;
/// Below this many keys a shard is not cleaned up for its size alone.
const MIN_PURGE_LEN: usize = 64;

/// The default [`Store`]: the state in the process's memory, in a sharded
/// map. Right for an app that runs as one process; the state is lost on
/// restart (which gives everyone a fresh allowance).
///
/// Each key takes about 40 bytes. Memory is kept bounded in two ways:
///
/// - **Cleanup.** A key is forgotten once its allowance is full again
///   ([`gcra::is_idle`]), which loses nothing. Each shard does this when it
///   has doubled in size since the last cleanup, and at least every
///   [`purge_interval`](MemoryStore::purge_interval) (one minute by default)
///   when it is used. [`purge`](MemoryStore::purge) does it for all shards
///   at once, for a periodic task if you like.
/// - **A cap.** Under a flood of distinct keys that are all still counting,
///   the store holds at most [`max_keys`](MemoryStore::max_keys) (100,000
///   by default, a few megabytes). Past that, a shard forgets its eighth of
///   keys that are closest to being idle anyway. Those keys get their full
///   allowance back early, so keep the cap well above the number of keys
///   you expect in a window.
///
/// ```
/// use std::time::Duration;
/// use refractory::{Limiter, MemoryStore};
///
/// let store = MemoryStore::new()
///     .max_keys(10_000)
///     .purge_interval(Duration::from_secs(30));
/// let limiter = Limiter::builder().store(store).build();
/// assert!(limiter.store().is_empty());
/// ```
#[derive(Debug)]
pub struct MemoryStore {
    shards: Box<[Mutex<Shard>]>,
    max_per_shard: usize,
    purge_interval: u64,
}

#[derive(Debug, Default)]
struct Shard {
    /// Key → theoretical arrival time, in nanoseconds of the limiter's clock.
    map: HashMap<u128, u64>,
    /// Clean up once the map grows past this.
    next_purge_len: usize,
    last_purge: u64,
}

impl MemoryStore {
    /// An empty store with the default cap (100,000 keys) and cleanup
    /// interval (one minute).
    #[must_use]
    pub fn new() -> Self {
        let shards = (0..SHARDS)
            .map(|_| {
                Mutex::new(Shard {
                    next_purge_len: MIN_PURGE_LEN,
                    ..Shard::default()
                })
            })
            .collect();
        Self {
            shards,
            max_per_shard: 100_000 / SHARDS,
            purge_interval: 60_000_000_000,
        }
    }

    /// Keeps at most about `max` keys (rounded up to a multiple of the
    /// number of shards, 32).
    #[must_use]
    pub fn max_keys(mut self, max: usize) -> Self {
        self.max_per_shard = max.div_ceil(SHARDS).max(1);
        self
    }

    /// Cleans up a shard that is in use at least this often (one minute by
    /// default).
    #[must_use]
    pub fn purge_interval(mut self, every: Duration) -> Self {
        self.purge_interval = nanos(every);
        self
    }

    /// How many keys the store holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shards.iter().map(|shard| lock(shard).map.len()).sum()
    }

    /// Whether the store holds no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(|shard| lock(shard).map.is_empty())
    }

    /// Forgets every key that is idle at `now` (the limiter's
    /// [`Clock::now`](crate::Clock::now)); see
    /// [`Limiter::purge`](crate::Limiter::purge).
    pub fn purge(&self, now: Duration) {
        let now = nanos(now);
        for shard in &self.shards {
            lock(shard).purge(now);
        }
    }

    /// [`Store::check`], without the future.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn check_sync(&self, key: Key, rule: &Rule, now: Duration) -> Result<(), Exceeded> {
        let bits = key.as_u128();
        // The key is a keyed hash, so any of its bits pick a shard evenly.
        let mut shard = lock(&self.shards[(bits >> 64) as usize % SHARDS]);
        let now_nanos = nanos(now);
        if now_nanos.saturating_sub(shard.last_purge) >= self.purge_interval {
            shard.purge(now_nanos);
        }
        let tat = shard.map.get(&bits).copied().map(Duration::from_nanos);
        let tat = gcra::step(rule, tat, now)?;
        shard.map.insert(bits, nanos(tat));
        if shard.map.len() > shard.next_purge_len {
            shard.purge(now_nanos);
            if shard.map.len() > self.max_per_shard {
                shard.evict(self.max_per_shard - self.max_per_shard / 8);
            }
            shard.next_purge_len = (shard.map.len() * 2)
                .max(MIN_PURGE_LEN)
                .min(self.max_per_shard);
        }
        Ok(())
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for MemoryStore {
    type Error = Infallible;

    fn check(
        &self,
        key: Key,
        rule: Rule,
        now: Duration,
    ) -> impl Future<Output = Result<Result<(), Exceeded>, Infallible>> + Send {
        ready(Ok(self.check_sync(key, &rule, now)))
    }
}

impl Shard {
    fn purge(&mut self, now: u64) {
        self.map.retain(|_, tat| *tat > now);
        if self.map.capacity() > 4 * self.map.len().max(MIN_PURGE_LEN) {
            self.map.shrink_to(2 * self.map.len());
        }
        self.last_purge = now;
    }

    /// Keeps the `keep` keys with the latest TATs and forgets the rest.
    fn evict(&mut self, keep: usize) {
        let len = self.map.len();
        if len <= keep {
            return;
        }
        let drop = len - keep;
        let mut tats: Vec<u64> = self.map.values().copied().collect();
        let cut = *tats.select_nth_unstable(drop).1;
        // Fewer than `drop` TATs are below the cut; drop that many of those
        // equal to it as well, so ties (one burst of keys) still shrink.
        let mut equal_to_drop = drop - tats.iter().filter(|&&tat| tat < cut).count();
        self.map.retain(|_, tat| {
            if *tat < cut {
                false
            } else if *tat == cut && equal_to_drop > 0 {
                equal_to_drop -= 1;
                false
            } else {
                true
            }
        });
    }
}

fn lock(shard: &Mutex<Shard>) -> MutexGuard<'_, Shard> {
    // The map is valid after any panic (each change is a single call).
    shard.lock().unwrap_or_else(PoisonError::into_inner)
}
