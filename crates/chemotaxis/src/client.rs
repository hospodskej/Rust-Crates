//! The openrouteservice client.

use std::fmt;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::cache::{Cache, NoCache};
use crate::ors;
use crate::{Error, LatLon};

/// Where openrouteservice answers unless the builder says otherwise.
pub const DEFAULT_BASE_URL: &str = "https://api.openrouteservice.org";
/// The default limit for connecting, and separately for reading the answer.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);
/// How long found distances are cached by default.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// The geocoding layers a place lookup searches: towns, parts of towns and
/// postcodes, so "Znojmo" doesn't find a street or a shop of that name.
pub const PLACE_LAYERS: &str = "locality,localadmin,borough,neighbourhood,postalcode";
/// The longest place query, in characters.
pub const MAX_QUERY_CHARS: usize = 80;

/// A delivery address, as the customer typed it.
///
/// Spaces at the ends and runs of spaces inside don't matter, nor does
/// letter case: they give the same cached answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address<'a> {
    /// Street and house number, e.g. `"Hlavní 1"`. Required.
    pub street: &'a str,
    /// Town, e.g. `"Brno"`. Required.
    pub city: &'a str,
    /// Postcode, e.g. `"602 00"`. May be empty.
    pub zip: &'a str,
    /// ISO 3166-1 alpha-2 country code, either case, e.g. `"cz"`.
    pub country: &'a str,
}

/// A town or postcode as geocoding found it.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// Where it is.
    pub point: LatLon,
    /// The town as found, e.g. `"Znojmo"` for the query `"669 02"`. The
    /// query itself when the answer names nothing.
    pub name: String,
    /// The postcode, when the answer has one.
    pub zip: Option<String>,
}

/// The distance to a town or postcode, for an estimate before the customer
/// gives a full address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estimate {
    /// One-way driving distance from the origin in whole kilometres.
    pub km: u32,
    /// The town as found, e.g. `"Znojmo"`.
    pub place: String,
    /// The postcode, when the answer has one, e.g. `"669 02"`.
    pub zip: Option<String>,
}

/// Configures a [`Client`]. Made by [`Client::builder`].
#[must_use]
pub struct ClientBuilder<C = NoCache> {
    api_key: String,
    origin: LatLon,
    base_url: String,
    connect_timeout: Duration,
    read_timeout: Duration,
    cache: C,
    cache_ttl: Duration,
    countries: Vec<String>,
}

impl<C> ClientBuilder<C> {
    /// Sends requests here instead of [`DEFAULT_BASE_URL`], e.g. to a mock
    /// server in tests or a self-hosted openrouteservice. A trailing slash
    /// is ignored.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        url.trim_end_matches('/').clone_into(&mut self.base_url);
        self
    }

    /// The limit for opening the connection. 4 seconds by default.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// The limit for each read of the answer. 4 seconds by default.
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// Keeps found distances in `cache`. Without one nothing is kept.
    pub fn cache<D: Cache>(self, cache: D) -> ClientBuilder<D> {
        ClientBuilder {
            api_key: self.api_key,
            origin: self.origin,
            base_url: self.base_url,
            connect_timeout: self.connect_timeout,
            read_timeout: self.read_timeout,
            cache,
            cache_ttl: self.cache_ttl,
            countries: self.countries,
        }
    }

    /// How long found distances are kept. 30 days by default.
    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    /// Only looks up addresses and places in these countries (ISO 3166-1
    /// alpha-2 codes, either case); others are
    /// [`Error::InvalidInput`] without a request. By default any country.
    pub fn countries<I, S>(mut self, countries: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.countries = countries
            .into_iter()
            .map(|code| code.as_ref().trim().to_ascii_lowercase())
            .collect();
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidBaseUrl`] for a base URL that isn't an `http` or
    /// `https` URL, [`Error::InvalidInput`] for an origin outside -90..90
    /// latitude or -180..180 longitude, and [`Error::Request`] if the HTTP
    /// client can't start (no TLS roots).
    pub fn build(self) -> Result<Client<C>, Error> {
        match reqwest::Url::parse(&self.base_url) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => {}
            _ => return Err(Error::InvalidBaseUrl(self.base_url)),
        }
        if !self.origin.is_valid() {
            return Err(Error::InvalidInput("origin coordinates out of range"));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(self.connect_timeout)
            .read_timeout(self.read_timeout)
            .user_agent(concat!("chemotaxis/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| Error::Request(error.without_url()))?;
        Ok(Client {
            http,
            api_key: self.api_key,
            origin: self.origin,
            base_url: self.base_url,
            cache: self.cache,
            cache_ttl: self.cache_ttl,
            countries: self.countries,
        })
    }
}

impl<C> fmt::Debug for ClientBuilder<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("api_key", &Redacted)
            .field("origin", &self.origin)
            .field("base_url", &self.base_url)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_timeout", &self.read_timeout)
            .field("cache_ttl", &self.cache_ttl)
            .field("countries", &self.countries)
            .finish_non_exhaustive()
    }
}

/// An openrouteservice client that finds the driving distance from a fixed
/// origin (a shop, a sawmill) to an address or a town.
///
/// Cheap to clone when the cache is (the HTTP connection pool is shared).
/// `Debug` prints `[redacted]` in place of the API key.
///
/// ```no_run
/// use chemotaxis::{Address, Client, LatLon, MemoryCache};
///
/// # async fn run(api_key: String) -> Result<(), chemotaxis::Error> {
/// let sawmill = LatLon { lat: 48.80958, lon: 16.12111 };
/// let client = Client::builder(api_key, sawmill)
///     .countries(["cz", "at"])
///     .cache(MemoryCache::new())
///     .build()?;
///
/// let estimate = client.estimate("Znojmo", "cz").await?;
/// println!("{} ({:?}): {} km", estimate.place, estimate.zip, estimate.km);
///
/// let km = client
///     .one_way_km(&Address { street: "Hlavní 1", city: "Brno", zip: "602 00", country: "cz" })
///     .await?;
/// println!("{km} km one way");
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Client<C = NoCache> {
    http: reqwest::Client,
    api_key: String,
    origin: LatLon,
    base_url: String,
    cache: C,
    cache_ttl: Duration,
    countries: Vec<String>,
}

impl Client {
    /// A client with the defaults: [`DEFAULT_BASE_URL`], 4-second
    /// timeouts, any country and no cache.
    ///
    /// The API key is the caller's to load (from the environment, a secret
    /// store); this crate reads no environment variables. It is sent in the
    /// `Authorization` header, never in the URL, and keep it out of logs on
    /// your side too.
    ///
    /// An empty key still builds, so a site without one runs; every lookup
    /// then answers [`Error::MissingApiKey`] without a request.
    ///
    /// # Errors
    ///
    /// As [`ClientBuilder::build`].
    pub fn new(api_key: impl Into<String>, origin: LatLon) -> Result<Self, Error> {
        Self::builder(api_key, origin).build()
    }

    /// A builder for a client with another base URL, timeouts, cache or
    /// allowed countries. See [`Client::new`] about the key.
    pub fn builder(api_key: impl Into<String>, origin: LatLon) -> ClientBuilder {
        ClientBuilder {
            api_key: api_key.into().trim().to_owned(),
            origin,
            base_url: DEFAULT_BASE_URL.to_owned(),
            connect_timeout: DEFAULT_TIMEOUT,
            read_timeout: DEFAULT_TIMEOUT,
            cache: NoCache,
            cache_ttl: DEFAULT_CACHE_TTL,
            countries: Vec::new(),
        }
    }
}

impl<C: Cache> Client<C> {
    /// Where routes start.
    #[must_use]
    pub fn origin(&self) -> LatLon {
        self.origin
    }

    /// The cache the client keeps distances in.
    pub fn cache(&self) -> &C {
        &self.cache
    }

    /// One-way driving distance from the origin to `address`, in whole
    /// kilometres (metres / 1000, rounded half away from zero: 49.5 km is
    /// 50). Cached per address.
    ///
    /// Costs one geocode and one route request when not cached.
    ///
    /// # Errors
    ///
    /// [`Error::MissingApiKey`]; [`Error::InvalidInput`] for an empty street
    /// or town or a country not allowed; [`Error::NotFound`] when geocoding
    /// finds nothing; [`Error::NoRoute`]; and the request failures
    /// ([`Error::Http`], [`Error::Timeout`], [`Error::Request`]). None of
    /// them are cached.
    pub async fn one_way_km(&self, address: &Address<'_>) -> Result<u32, Error> {
        self.require_key()?;
        let country = self.country(address.country)?;
        if is_blank(address.street) || is_blank(address.city) {
            return Err(Error::InvalidInput("street and town are required"));
        }
        let key = self.cache_key(
            "km",
            &[
                &country,
                &squish(address.zip),
                &squish(address.city),
                &squish(address.street),
            ],
        );
        if let Some(km) = self.cached(&key).await {
            return Ok(km);
        }
        let point = self.geocode(address).await?;
        let km = round_km(self.driving_meters(point).await?)?;
        self.store(&key, &km).await;
        Ok(km)
    }

    /// The estimate before checkout: the distance to a town or postcode
    /// (`query`, e.g. `"Znojmo"` or `"669 02"`), looking only for places in
    /// `country`. Cached per place, whatever the spaces and case.
    ///
    /// Costs one geocode and one route request when not cached.
    ///
    /// # Errors
    ///
    /// As [`Client::one_way_km`]; [`Error::InvalidInput`] also for an empty
    /// query or one over [`MAX_QUERY_CHARS`].
    pub async fn estimate(&self, query: &str, country: &str) -> Result<Estimate, Error> {
        self.require_key()?;
        let country = self.country(country)?;
        let query = squish(query);
        check_query(&query)?;
        let key = self.cache_key("estimate", &[&country, &query.to_lowercase()]);
        if let Some(estimate) = self.cached(&key).await {
            return Ok(estimate);
        }
        let place = self.find_place(&query, &country).await?;
        let km = round_km(self.driving_meters(place.point).await?)?;
        let estimate = Estimate {
            km,
            place: place.name,
            zip: place.zip,
        };
        self.store(&key, &estimate).await;
        Ok(estimate)
    }

    /// Geocodes a full address (structured search, best match only). Not
    /// cached.
    ///
    /// # Errors
    ///
    /// As [`Client::one_way_km`], without [`Error::NoRoute`].
    pub async fn geocode(&self, address: &Address<'_>) -> Result<LatLon, Error> {
        self.require_key()?;
        let country = self.country(address.country)?;
        let (street, city, zip) = (
            squish(address.street),
            squish(address.city),
            squish(address.zip),
        );
        if street.is_empty() || city.is_empty() {
            return Err(Error::InvalidInput("street and town are required"));
        }
        let mut query = vec![("address", street), ("locality", city)];
        if !zip.is_empty() {
            query.push(("postalcode", zip));
        }
        query.push(("country", country.to_ascii_uppercase()));
        query.push(("size", "1".to_owned()));
        let answer: ors::FeatureCollection<ors::Place> =
            self.get("/geocode/search/structured", &query).await?;
        answer
            .features
            .first()
            .and_then(|feature| feature.geometry.as_ref())
            .and_then(ors::Geometry::point)
            .ok_or(Error::NotFound)
    }

    /// Geocodes a town or postcode in `country`, searching only the
    /// [`PLACE_LAYERS`] (best match only). Not cached.
    ///
    /// # Errors
    ///
    /// As [`Client::estimate`], without [`Error::NoRoute`].
    pub async fn geocode_place(&self, query: &str, country: &str) -> Result<Place, Error> {
        self.require_key()?;
        let country = self.country(country)?;
        let query = squish(query);
        check_query(&query)?;
        self.find_place(&query, &country).await
    }

    /// Driving distance by car from the origin to `to`, in metres, as
    /// openrouteservice answers it. Not cached.
    ///
    /// # Errors
    ///
    /// [`Error::MissingApiKey`], [`Error::NoRoute`] when the answer has no
    /// distance, and the request failures.
    pub async fn driving_meters(&self, to: LatLon) -> Result<f64, Error> {
        self.require_key()?;
        let query = [("start", self.origin.to_ors()), ("end", to.to_ors())];
        let answer: ors::FeatureCollection<ors::Route> =
            self.get("/v2/directions/driving-car", &query).await?;
        answer
            .features
            .first()
            .and_then(|feature| feature.properties.as_ref())
            .and_then(|route| route.summary.as_ref())
            .and_then(|summary| summary.distance)
            .ok_or(Error::NoRoute)
    }

    async fn find_place(&self, query: &str, country: &str) -> Result<Place, Error> {
        let params = [
            ("text", query.to_owned()),
            ("boundary.country", country.to_ascii_uppercase()),
            ("layers", PLACE_LAYERS.to_owned()),
            ("size", "1".to_owned()),
        ];
        let answer: ors::FeatureCollection<ors::Place> =
            self.get("/geocode/search", &params).await?;
        let feature = answer.features.into_iter().next().ok_or(Error::NotFound)?;
        let point = feature
            .geometry
            .as_ref()
            .and_then(ors::Geometry::point)
            .ok_or(Error::NotFound)?;
        let properties = feature.properties.unwrap_or_default();
        let name = present(properties.locality)
            .or_else(|| present(properties.name))
            .unwrap_or_else(|| query.to_owned());
        Ok(Place {
            point,
            name,
            zip: present(properties.postalcode),
        })
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, Error> {
        let response = self
            .http
            .get(format!("{}{path}", self.base_url))
            .query(query)
            .header(AUTHORIZATION, &self.api_key)
            .header(ACCEPT, "application/json, application/geo+json")
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Http {
                status: status.as_u16(),
                message: ors::error_message(&body),
            });
        }
        response.json().await.map_err(request_error)
    }

    fn require_key(&self) -> Result<(), Error> {
        if self.api_key.is_empty() {
            Err(Error::MissingApiKey)
        } else {
            Ok(())
        }
    }

    fn country(&self, code: &str) -> Result<String, Error> {
        let code = code.trim().to_ascii_lowercase();
        if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_lowercase()) {
            return Err(Error::InvalidInput("country must be a two-letter code"));
        }
        if !self.countries.is_empty() && !self.countries.contains(&code) {
            return Err(Error::InvalidInput("country not served"));
        }
        Ok(code)
    }

    /// A JSON array, so no part can run into the next.
    fn cache_key(&self, kind: &str, parts: &[&str]) -> String {
        let origin = self.origin.to_ors();
        let mut key = vec!["chemotaxis", kind, &origin];
        key.extend(parts.iter().map(|part| part.trim()));
        let lowered: Vec<String> = key.iter().map(|part| part.to_lowercase()).collect();
        serde_json::to_string(&lowered).unwrap_or_default()
    }

    async fn cached<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_str(&self.cache.get(key).await?).ok()
    }

    async fn store<T: Serialize>(&self, key: &str, value: &T) {
        if let Ok(json) = serde_json::to_string(value) {
            self.cache.put(key, json, self.cache_ttl).await;
        }
    }
}

impl<C> fmt::Debug for Client<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("api_key", &Redacted)
            .field("origin", &self.origin)
            .field("base_url", &self.base_url)
            .field("cache_ttl", &self.cache_ttl)
            .field("countries", &self.countries)
            .finish_non_exhaustive()
    }
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Timeouts apart, the URL is dropped from the error: it holds the
/// customer's address.
fn request_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout
    } else {
        Error::Request(error.without_url())
    }
}

fn check_query(query: &str) -> Result<(), Error> {
    if query.is_empty() {
        return Err(Error::InvalidInput("a town or postcode is required"));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::InvalidInput("the place query is too long"));
    }
    Ok(())
}

/// Metres to whole kilometres, as the Ruby original did with
/// `(meters / 1000.0).round`: half away from zero.
fn round_km(meters: f64) -> Result<u32, Error> {
    let km = (meters / 1000.0).round();
    if !(0.0..=f64::from(u32::MAX)).contains(&km) {
        return Err(Error::NoRoute);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "range checked above"
    )]
    Ok(km as u32)
}

/// Trims and collapses inner whitespace to one space (Rails' `squish`).
fn squish(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// The text unless it is missing or blank (Rails' `presence`).
fn present(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_half_away_from_zero() {
        assert_eq!(round_km(49_620.0).unwrap(), 50);
        assert_eq!(round_km(49_499.9).unwrap(), 49);
        assert_eq!(round_km(49_500.0).unwrap(), 50);
        assert_eq!(round_km(23_400.0).unwrap(), 23);
        assert_eq!(round_km(0.0).unwrap(), 0);
        assert!(round_km(-1_000.0).is_err());
        assert!(round_km(f64::NAN).is_err());
    }

    #[test]
    fn squish_like_rails() {
        assert_eq!(squish("  Hlavní \t 1 \n"), "Hlavní 1");
        assert_eq!(squish("   "), "");
    }

    #[test]
    fn cache_keys_ignore_spaces_and_case_but_not_separators() {
        let client = Client::new(
            "key",
            LatLon {
                lat: 48.80958,
                lon: 16.12111,
            },
        )
        .unwrap();
        assert_eq!(
            client.cache_key("km", &["cz", "60200", "Brno", "HLAVNÍ 1"]),
            r#"["chemotaxis","km","16.12111,48.80958","cz","60200","brno","hlavní 1"]"#
        );
        assert_ne!(
            client.cache_key("km", &["cz", "a|b", "c"]),
            client.cache_key("km", &["cz", "a", "b|c"])
        );
    }

    #[test]
    fn debug_redacts_the_key() {
        let builder = Client::builder(
            "secret-key-123",
            LatLon {
                lat: 48.8,
                lon: 16.1,
            },
        );
        assert!(!format!("{builder:?}").contains("secret-key-123"));
        let client = builder.build().unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("secret-key-123"));
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn build_checks_url_and_origin() {
        let origin = LatLon {
            lat: 48.8,
            lon: 16.1,
        };
        assert!(matches!(
            Client::builder("k", origin).base_url("not a url").build(),
            Err(Error::InvalidBaseUrl(_))
        ));
        assert!(matches!(
            Client::builder("k", origin).base_url("ftp://x").build(),
            Err(Error::InvalidBaseUrl(_))
        ));
        assert!(matches!(
            Client::new(
                "k",
                LatLon {
                    lat: 116.1,
                    lon: 48.8
                }
            ),
            Err(Error::InvalidInput(_))
        ));
    }
}
