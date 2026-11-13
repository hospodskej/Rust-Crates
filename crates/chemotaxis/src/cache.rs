//! Where found distances are kept between lookups.

use std::collections::HashMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// A store for found distances, kept for a time to live.
///
/// The client keeps every distance it finds for 30 days (see
/// [`ClientBuilder::cache_ttl`](crate::ClientBuilder::cache_ttl)), one entry
/// per address or place, so a customer who opens the checkout three times
/// costs one geocode and one route. "Not found" and failures are not kept:
/// the next lookup asks again.
///
/// Keys are short UTF-8 strings that already hold everything that makes an
/// answer differ (origin, country, the normalized address or place). Values
/// are small JSON documents. Treat both as opaque.
///
/// A cache never fails a lookup: a `get` that can't read answers `None`
/// (the client asks openrouteservice instead) and a `put` that can't write
/// drops the value. Log such errors in the implementation if you want them.
///
/// The methods return `Send` futures, so the client can be used from any
/// multi-threaded runtime; an implementation writes them as `async fn`.
///
/// ```
/// use std::time::Duration;
/// use chemotaxis::Cache;
///
/// /// Sketch of a database-backed cache.
/// struct Db;
///
/// impl Cache for Db {
///     async fn get(&self, key: &str) -> Option<String> {
///         // SELECT value FROM ors_cache WHERE key = ? AND expires_at > now()
///         # let _ = key;
///         None
///     }
///
///     async fn put(&self, key: &str, value: String, ttl: Duration) {
///         // INSERT OR REPLACE INTO ors_cache (key, value, expires_at)
///         // VALUES (?, ?, now() + ttl)
///         # let _ = (key, value, ttl);
///     }
/// }
/// ```
pub trait Cache: Send + Sync {
    /// The value stored under `key`, unless there is none or it expired.
    fn get(&self, key: &str) -> impl Future<Output = Option<String>> + Send;

    /// Stores `value` under `key` for `ttl`, replacing any earlier value.
    fn put(&self, key: &str, value: String, ttl: Duration) -> impl Future<Output = ()> + Send;
}

impl<C: Cache> Cache for Arc<C> {
    fn get(&self, key: &str) -> impl Future<Output = Option<String>> + Send {
        C::get(self, key)
    }

    fn put(&self, key: &str, value: String, ttl: Duration) -> impl Future<Output = ()> + Send {
        C::put(self, key, value, ttl)
    }
}

const CENTURY: Duration = Duration::from_secs(100 * 365 * 86_400);

/// A cache that keeps nothing: every lookup asks openrouteservice. The
/// client's default.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCache;

impl Cache for NoCache {
    fn get(&self, _key: &str) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn put(&self, _key: &str, _value: String, _ttl: Duration) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

/// A cache in the process's memory, lost on restart and not shared between
/// processes. Fine for one server process and for tests; use a database
/// for anything longer-lived.
///
/// Expired entries are dropped when they are read and whenever a new value
/// is stored. Clones share the same entries.
///
/// ```
/// use chemotaxis::{Client, LatLon, MemoryCache};
///
/// let cache = MemoryCache::new();
/// let client = Client::builder("my-key", LatLon { lat: 48.80958, lon: 16.12111 })
///     .cache(cache.clone())
///     .build()?;
/// assert!(cache.is_empty());
/// # Ok::<(), chemotaxis::Error>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct MemoryCache {
    entries: Arc<Mutex<HashMap<String, (String, Instant)>>>,
}

impl MemoryCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many entries are stored, expired ones not yet dropped included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Drops every entry.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (String, Instant)>> {
        // A panic elsewhere can't leave a half-written HashMap entry behind.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn get_now(&self, key: &str) -> Option<String> {
        let mut entries = self.lock();
        match entries.get(key) {
            Some((value, expires)) if *expires > Instant::now() => Some(value.clone()),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    fn put_now(&self, key: &str, value: String, ttl: Duration) {
        let now = Instant::now();
        let mut entries = self.lock();
        entries.retain(|_, (_, expires)| *expires > now);
        // A TTL too long for the clock is capped at a century.
        let expires = now
            .checked_add(ttl.min(CENTURY))
            .unwrap_or(now + Duration::from_secs(86_400));
        entries.insert(key.to_owned(), (value, expires));
    }
}

impl Cache for MemoryCache {
    fn get(&self, key: &str) -> impl Future<Output = Option<String>> + Send {
        ready(self.get_now(key))
    }

    fn put(&self, key: &str, value: String, ttl: Duration) -> impl Future<Output = ()> + Send {
        self.put_now(key, value, ttl);
        ready(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_values_until_they_expire() {
        let cache = MemoryCache::new();
        cache.put_now("a", "1".into(), Duration::from_secs(60));
        cache.put_now("b", "2".into(), Duration::ZERO);
        assert_eq!(cache.get_now("a").as_deref(), Some("1"));
        assert_eq!(cache.get_now("b"), None);
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn clones_share_entries() {
        let cache = MemoryCache::new();
        cache
            .clone()
            .put_now("a", "1".into(), Duration::from_secs(60));
        assert_eq!(cache.get_now("a").as_deref(), Some("1"));
    }

    #[test]
    fn an_endless_ttl_is_kept() {
        let cache = MemoryCache::new();
        cache.put_now("a", "1".into(), Duration::MAX);
        assert_eq!(cache.get_now("a").as_deref(), Some("1"));
    }
}
