//! Road distance to a place, from openrouteservice.
//!
//! Chemotaxis is an organism moving towards a goal along a gradient. This
//! crate finds how far a delivery has to drive: it geocodes an address or a
//! town with [openrouteservice](https://openrouteservice.org) and routes to
//! it by car from a fixed origin, such as a shop or a sawmill.
//!
//! - [`Client::one_way_km`]: a full address (street, town, postcode,
//!   country) to one-way kilometres, for the checkout.
//! - [`Client::estimate`]: a town name or postcode (places only, in the
//!   chosen country) to the place as found, its postcode and one-way
//!   kilometres, for an estimate before checkout.
//! - [`Client::suggest`]: suggestions while typing an address or a town,
//!   nearest to the origin first, as JSON a browser autocomplete reads.
//! - [`Client::geocode`], [`Client::geocode_place`] and
//!   [`Client::driving_meters`] for the steps on their own, uncached.
//!
//! Kilometres are whole, rounded half away from zero from the metres the
//! route answers. Found distances are cached for 30 days per address or
//! place in a [`Cache`] of your choice ([`MemoryCache`], [`NoCache`] or your
//! own, e.g. on SQLite). "Not found" and failures are not cached.
//!
//! Every failure is an [`Error`]; [`Error::is_not_found`] tells "the service
//! has no route there" from "the service failed". A shop typically falls
//! back to "delivery price calculated individually" on either.
//!
//! ```no_run
//! use chemotaxis::{Address, Client, Error, LatLon, MemoryCache};
//!
//! # async fn run(api_key: String) -> Result<(), Error> {
//! let sawmill = LatLon { lat: 48.80958, lon: 16.12111 };
//! let client = Client::builder(api_key, sawmill)
//!     .countries(["cz", "at"])
//!     .cache(MemoryCache::new())
//!     .build()?;
//!
//! let address = Address { street: "Hlavní 1", city: "Brno", zip: "602 00", country: "cz" };
//! match client.one_way_km(&address).await {
//!     Ok(km) => println!("{} km there and back", 2 * km),
//!     Err(error) if error.is_not_found() => println!("We'll price the delivery individually."),
//!     Err(error) => println!("Lookup failed ({error}); we'll price it individually."),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # API key and quota
//!
//! Get a key at <https://openrouteservice.org> and pass it to
//! [`Client::new`]; the crate never reads environment variables. The key is
//! sent in the `Authorization` header, never in a URL, and the client's
//! `Debug` prints `[redacted]` in its place. Keep it out of your own logs
//! too. Errors carry no URLs, so they don't log the customer's address.
//!
//! The free plan allows 1,000 geocoding and 2,000 route requests a day. A
//! lookup that isn't cached costs one of each, and an uncached suggestion
//! one geocoding request; the crate doesn't count them.
//! Over the quota, openrouteservice answers 429 ([`Error::Http`]).
//!
//! # Timeouts
//!
//! 4 seconds to connect and 4 seconds for each read of the answer, as
//! [`ClientBuilder::connect_timeout`] and [`ClientBuilder::read_timeout`]
//! can change. A shop page shouldn't wait longer for an estimate.

#![forbid(unsafe_code)]

mod cache;
mod client;
mod coords;
mod error;
mod ors;

pub use cache::{Cache, MemoryCache, NoCache};
pub use client::{
    ADDRESS_LAYERS, Address, Client, ClientBuilder, DEFAULT_BASE_URL, DEFAULT_CACHE_TTL,
    DEFAULT_SUGGEST_TTL, DEFAULT_TIMEOUT, Estimate, MAX_QUERY_CHARS, MAX_SUGGESTIONS,
    MIN_SUGGEST_CHARS, PLACE_LAYERS, Place, SuggestKind, Suggestion,
};
pub use coords::LatLon;
pub use error::Error;

/// Compiles the README's example.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
