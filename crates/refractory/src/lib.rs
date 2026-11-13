//! Rate limiting for Rust web apps: how often something may happen per
//! key (an IP address, an e-mail, ...) within a window.
//!
//! A neuron's refractory period is the moment after it fires when it can't
//! fire again; this crate gives a login form, a registration or an API one.
//!
//! There are two ways to use it:
//!
//! - **In handlers**, with [`Limiter::check`]: the handler decides what to
//!   answer when a limit is hit (a redirect back with a message, a 429, a
//!   form with an error, ...).
//! - **As a Tower layer** on routes, with [`RateLimitLayer`] (feature
//!   `axum`): it answers `429 Too Many Requests` with `Retry-After` by
//!   itself, counting per client IP ([`ClientIp`]) or any other
//!   [`KeyExtractor`].
//!
//! ```
//! use std::time::Duration;
//! use refractory::{Limiter, Rule, normalize_email};
//!
//! // 10 attempts per 3 minutes from one address, and 20 per hour against
//! // one account from any address.
//! const LOGIN: Rule = Rule::new("login", 10, Duration::from_secs(3 * 60));
//! const LOGIN_PER_ACCOUNT: Rule = Rule::new("login-account", 20, Duration::from_secs(60 * 60));
//!
//! # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//! let limiter = Limiter::new();
//! let ip = "203.0.113.7";
//! let email = normalize_email("  Jana@Example.cz ");
//!
//! let allowed = limiter.check(&LOGIN, ip).await
//!     .and(limiter.check(&LOGIN_PER_ACCOUNT, &email).await);
//! match allowed {
//!     Ok(()) => { /* check the password */ }
//!     Err(exceeded) => println!("too many attempts, try again in {} s", exceeded.retry_after_secs()),
//! }
//! # });
//! ```
//!
//! With Axum, a handler that sends the visitor back with a message:
//!
//! ```
//! # #[cfg(feature = "axum")] {
//! use std::time::Duration;
//! use axum::{Extension, Router, extract::State, response::Redirect, routing::post};
//! use refractory::{ClientIp, Limiter, RemoteIp, Rule};
//!
//! const INQUIRY: Rule = Rule::new("inquiry", 5, Duration::from_secs(10 * 60));
//!
//! async fn send_inquiry(State(limiter): State<Limiter>, ip: RemoteIp) -> Redirect {
//!     if let Err(exceeded) = limiter.check(&INQUIRY, ip.key()).await {
//!         let minutes = exceeded.retry_after_secs().div_ceil(60);
//!         // Put the message in the flash/session as the app does it.
//!         return Redirect::to(&format!("/contact?error=too-many&minutes={minutes}"));
//!     }
//!     // ... send the inquiry ...
//!     Redirect::to("/contact?sent")
//! }
//!
//! let app: Router = Router::new()
//!     .route("/contact", post(send_inquiry))
//!     .layer(Extension(ClientIp::behind_proxy()))
//!     .with_state(Limiter::new());
//! # }
//! ```
//!
//! # The algorithm
//!
//! Limits are counted with the Generic Cell Rate Algorithm (GCRA, a form
//! of leaky bucket; see [`gcra`]). Per rule and key it stores a single
//! number, the time at which the key would have used up nothing, and each
//! check is a few arithmetic operations under one lock, with no timers.
//!
//! A rule of `limit` per `window` allows a burst of exactly `limit` events
//! (one more is refused), and then one more every `window / limit`: after
//! 10 logins in a row under "10 per 3 minutes", the 11th is allowed 18
//! seconds later, and the full 10 again after 3 quiet minutes.
//! [`Exceeded::retry_after`] is exactly the time to the next allowed event.
//! Over any long stretch, the average stays within `limit` per `window`.
//!
//! The Rails `rate_limit` this replaces counts in fixed windows, where a
//! client can send `limit` events at the end of one window and `limit`
//! more at the start of the next, `2 × limit` back to back. GCRA has no
//! such edge: after a burst, the next events come no faster than the
//! sustained rate. (In the worst case a window of time sees `2 × limit − 1`
//! events, the burst and then the steady refill, spread evenly.)
//!
//! # Keys and privacy
//!
//! Keys are not stored. [`Limiter::key`] turns the rule's name and the key
//! into a 128-bit keyed hash (SipHash-1-3), and only that reaches the
//! [`Store`]. With an e-mail as the key, the store, its debug output and a
//! dump of it list no addresses. The secret is random for each limiter by
//! default, so the hashes mean nothing outside the process. This is not
//! encryption: someone who has the process's memory, secret included, can
//! still test whether a given address was counted. Set a fixed secret with
//! [`Builder::secret`] only for a store shared by several processes, and
//! keep it out of that store. 128 bits make collisions between keys
//! practically impossible.
//!
//! # Memory
//!
//! [`MemoryStore`] forgets keys that have their full allowance back, and
//! caps the number of keys it keeps, so a flood of distinct keys can't
//! grow it without bound. See its documentation.
//!
//! # Features
//!
//! - `axum` (default): [`RateLimitLayer`] (a Tower layer, generic over the
//!   body types), the [`ClientIp`] key extractor and the [`RemoteIp`] Axum
//!   extractor. It turns on Axum's `tokio` feature, which has
//!   `ConnectInfo` (and which an app that serves with `ConnectInfo` has
//!   on anyway). Without this feature, the crate needs neither Axum nor
//!   Tokio: the check API returns plain futures, and
//!   [`Limiter::check_sync`] works without any runtime.

#![forbid(unsafe_code)]

mod clock;
mod exceeded;
pub mod gcra;
#[cfg(feature = "axum")]
mod layer;
mod limiter;
mod memory;
mod rule;
mod store;

pub use clock::{Clock, ManualClock, MonotonicClock, SystemClock};
pub use exceeded::Exceeded;
#[cfg(feature = "axum")]
pub use layer::{ClientIp, KeyExtractor, MissingRemoteIp, RateLimit, RateLimitLayer, RemoteIp};
pub use limiter::{Builder, Limiter};
pub use memory::MemoryStore;
pub use rule::Rule;
pub use store::{Key, Store};

/// An e-mail address as a rate-limit key: trimmed and lowercased, so
/// `" Jana@Example.cz"` and `"jana@example.cz"` count as one account (as
/// Rails' `params[:email_address].to_s.strip.downcase`).
///
/// ```
/// assert_eq!(refractory::normalize_email("  Jana@Example.CZ\n"), "jana@example.cz");
/// ```
#[must_use]
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}
