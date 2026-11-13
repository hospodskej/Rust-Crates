//! The Tower layer and the client-IP extractors (feature `axum`).

use core::fmt;
use core::future::Future;
use core::hash::Hash;
use core::net::{IpAddr, Ipv6Addr, SocketAddr};
use core::pin::Pin;
use core::task::{Context, Poll};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::response::{IntoResponse, Response as AxumResponse};
use http::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use http::request::Parts;
use http::{Extensions, Request, Response, StatusCode};
use tower_layer::Layer;
use tower_service::Service;

use crate::{Clock, Limiter, Rule, Store};

/// What a [`RateLimitLayer`] counts requests per.
///
/// [`ClientIp`] is the usual choice. Implement this for other keys, such as
/// a session's user ID:
///
/// ```
/// use refractory::KeyExtractor;
///
/// /// Counts per API token, from the `Authorization` header.
/// #[derive(Clone)]
/// struct ApiToken;
///
/// impl KeyExtractor for ApiToken {
///     type Key = String;
///
///     fn extract<B>(&self, request: &http::Request<B>) -> Option<String> {
///         let value = request.headers().get(http::header::AUTHORIZATION)?;
///         Some(value.to_str().ok()?.to_owned())
///     }
/// }
/// ```
pub trait KeyExtractor: Clone + Send + Sync + 'static {
    /// The key requests are counted per.
    type Key: Hash;

    /// The request's key. `None` makes the layer answer
    /// `500 Internal Server Error` without calling the route, because a
    /// missing key is a mistake in the set-up (no `ConnectInfo`, say), and
    /// letting such requests through unlimited would hide it.
    fn extract<B>(&self, request: &Request<B>) -> Option<Self::Key>;
}

/// Finds the client's IP address, and counts requests per address.
///
/// - [`ClientIp::direct`] (the default): the address of the TCP peer, from
///   Axum's [`ConnectInfo<SocketAddr>`]. Serve the app with
///   `into_make_service_with_connect_info::<SocketAddr>()`.
/// - [`ClientIp::behind_proxy`]: for an app that only a reverse proxy
///   (kamal-proxy, nginx, ...) can reach. The proxy appends the address it
///   saw to `X-Forwarded-For`, so the client's address is the **last**
///   entry; the entries before it are whatever the client sent and are
///   ignored. Without that header, `X-Real-IP`, then the peer address.
///   [`ClientIp::behind_proxies`] is for a chain of `n` proxies.
///
/// Never trust the headers when clients can reach the app directly: anyone
/// could then choose their own address, and with it a fresh allowance.
///
/// IPv6 addresses are counted per `/64` network by default, since one
/// household or device usually gets a whole `/64` and could otherwise use
/// a new address for every request; see [`ClientIp::ipv6_prefix`].
/// IPv4-mapped IPv6 addresses (`::ffff:203.0.113.7`) count as IPv4.
///
/// ```
/// use std::net::IpAddr;
/// use http::{HeaderMap, Extensions};
/// use refractory::ClientIp;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("x-forwarded-for", "10.0.0.1, 203.0.113.7".parse().unwrap());
/// let ip = ClientIp::behind_proxy().ip(&headers, &Extensions::new());
/// assert_eq!(ip, Some("203.0.113.7".parse::<IpAddr>().unwrap()));
///
/// let key = ClientIp::direct().key_of("2001:db8:1:2:3:4:5:6".parse().unwrap());
/// assert_eq!(key, "2001:db8:1:2::".parse::<IpAddr>().unwrap());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp {
    proxies: u8,
    ipv6_prefix: u8,
}

impl ClientIp {
    /// The TCP peer's address; no headers are trusted.
    #[must_use]
    pub const fn direct() -> Self {
        Self {
            proxies: 0,
            ipv6_prefix: 64,
        }
    }

    /// Behind one trusted reverse proxy, such as kamal-proxy.
    #[must_use]
    pub const fn behind_proxy() -> Self {
        Self::behind_proxies(1)
    }

    /// Behind a chain of `proxies` trusted reverse proxies, each of which
    /// appends to `X-Forwarded-For`: the client's address is the entry
    /// `proxies` places from the end. `0` is [`ClientIp::direct`].
    #[must_use]
    pub const fn behind_proxies(proxies: u8) -> Self {
        Self {
            proxies,
            ipv6_prefix: 64,
        }
    }

    /// Counts IPv6 addresses per network of this prefix length (default
    /// 64; 128 counts every address on its own).
    ///
    /// # Panics
    ///
    /// If `bits` is more than 128.
    #[must_use]
    pub const fn ipv6_prefix(mut self, bits: u8) -> Self {
        assert!(bits <= 128, "an IPv6 prefix has at most 128 bits");
        self.ipv6_prefix = bits;
        self
    }

    /// The client's address, from a request's headers and extensions.
    #[must_use]
    pub fn ip(&self, headers: &HeaderMap, extensions: &Extensions) -> Option<IpAddr> {
        let from_headers = || {
            forwarded_for(headers, usize::from(self.proxies))
                .or_else(|| headers.get("x-real-ip").and_then(parse_ip))
        };
        let ip = if self.proxies > 0 {
            from_headers()
        } else {
            None
        };
        ip.or_else(|| {
            extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(peer)| peer.ip())
        })
        .map(|ip| ip.to_canonical())
    }

    /// The rate-limit key for `ip`: the address itself for IPv4, its
    /// network for IPv6 (see [`ClientIp::ipv6_prefix`]).
    #[must_use]
    pub fn key_of(&self, ip: IpAddr) -> IpAddr {
        match ip.to_canonical() {
            IpAddr::V6(v6) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.ipv6_prefix))
                    .unwrap_or(0);
                IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & mask))
            }
            v4 @ IpAddr::V4(_) => v4,
        }
    }
}

impl Default for ClientIp {
    fn default() -> Self {
        Self::direct()
    }
}

impl KeyExtractor for ClientIp {
    type Key = IpAddr;

    fn extract<B>(&self, request: &Request<B>) -> Option<IpAddr> {
        self.ip(request.headers(), request.extensions())
            .map(|ip| self.key_of(ip))
    }
}

/// The `n`-th entry from the end of all `X-Forwarded-For` headers.
fn forwarded_for(headers: &HeaderMap, n: usize) -> Option<IpAddr> {
    let mut entries: Vec<&str> = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        entries.extend(value.to_str().ok()?.split(','));
    }
    let entry = entries.len().checked_sub(n).map(|i| entries[i])?;
    parse_ip_str(entry)
}

fn parse_ip(value: &HeaderValue) -> Option<IpAddr> {
    parse_ip_str(value.to_str().ok()?)
}

/// An address, also with a port (`203.0.113.7:4711`, `[2001:db8::1]:4711`).
fn parse_ip_str(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    s.parse::<IpAddr>()
        .ok()
        .or_else(|| s.parse::<SocketAddr>().ok().map(|addr| addr.ip()))
}

/// The client's IP address as an Axum extractor, for handlers that call
/// [`Limiter::check`] themselves.
///
/// It reads the [`ClientIp`] set-up from the request's extensions, so add
/// the same one the app uses as an `Extension` layer
/// (`.layer(Extension(ClientIp::behind_proxy()))`); without it,
/// [`ClientIp::direct`]. Rejects with `500 Internal Server Error` when no
/// address is found (see [`KeyExtractor::extract`]).
///
/// ```
/// use axum::{Extension, Router, routing::post};
/// use refractory::{ClientIp, RemoteIp};
///
/// async fn login(ip: RemoteIp) -> String {
///     format!("hello, {}", ip.ip())
/// }
///
/// let app: Router = Router::new()
///     .route("/login", post(login))
///     .layer(Extension(ClientIp::behind_proxy()));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteIp {
    ip: IpAddr,
    key: IpAddr,
}

impl RemoteIp {
    /// The client's address.
    #[must_use]
    pub const fn ip(&self) -> IpAddr {
        self.ip
    }

    /// The rate-limit key for it (see [`ClientIp::key_of`]).
    #[must_use]
    pub const fn key(&self) -> IpAddr {
        self.key
    }
}

impl fmt::Display for RemoteIp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.ip.fmt(f)
    }
}

impl<St: Send + Sync> FromRequestParts<St> for RemoteIp {
    type Rejection = MissingRemoteIp;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &St,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let config = parts
            .extensions
            .get::<ClientIp>()
            .copied()
            .unwrap_or_default();
        let found = config
            .ip(&parts.headers, &parts.extensions)
            .map(|ip| Self {
                ip,
                key: config.key_of(ip),
            })
            .ok_or(MissingRemoteIp);
        core::future::ready(found)
    }
}

/// The rejection of [`RemoteIp`]: the request had no address to read.
/// Answers `500 Internal Server Error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingRemoteIp;

impl fmt::Display for MissingRemoteIp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "no client IP address: serve the app with into_make_service_with_connect_info::<SocketAddr>()",
        )
    }
}

impl std::error::Error for MissingRemoteIp {}

impl IntoResponse for MissingRemoteIp {
    fn into_response(self) -> AxumResponse {
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}

/// A Tower layer that counts requests per key under one [`Rule`], and
/// answers `429 Too Many Requests` with a `Retry-After` header (in
/// seconds) once a key has used up its limit, without calling the route.
///
/// Refused requests are not counted. For a friendlier answer (a redirect
/// with a message, say), call [`Limiter::check`] in the handler instead.
///
/// ```
/// use std::time::Duration;
/// use axum::{Router, routing::post};
/// use refractory::{ClientIp, Limiter, RateLimitLayer, Rule};
///
/// const DELIVERY_ESTIMATE: Rule = Rule::new("delivery-estimate", 20, Duration::from_secs(600));
///
/// let limiter = Limiter::new();
/// let app: Router = Router::new()
///     .route("/delivery-estimate", post(|| async { "3 days" }))
///     .layer(RateLimitLayer::new(limiter, DELIVERY_ESTIMATE, ClientIp::behind_proxy()));
/// ```
pub struct RateLimitLayer<S, C, K> {
    limiter: Limiter<S, C>,
    rule: Rule,
    extractor: K,
}

impl<S, C, K> RateLimitLayer<S, C, K> {
    /// A layer that counts requests under `rule` per key from `extractor`,
    /// in `limiter`.
    pub fn new(limiter: Limiter<S, C>, rule: Rule, extractor: K) -> Self {
        Self {
            limiter,
            rule,
            extractor,
        }
    }
}

impl<S, C, K: Clone> Clone for RateLimitLayer<S, C, K> {
    fn clone(&self) -> Self {
        Self {
            limiter: self.limiter.clone(),
            rule: self.rule,
            extractor: self.extractor.clone(),
        }
    }
}

impl<S, C, K: fmt::Debug> fmt::Debug for RateLimitLayer<S, C, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimitLayer")
            .field("rule", &self.rule)
            .field("extractor", &self.extractor)
            .finish_non_exhaustive()
    }
}

impl<Svc, S, C, K: Clone> Layer<Svc> for RateLimitLayer<S, C, K> {
    type Service = RateLimit<Svc, S, C, K>;

    fn layer(&self, inner: Svc) -> Self::Service {
        RateLimit {
            inner,
            layer: self.clone(),
        }
    }
}

/// The service made by [`RateLimitLayer`].
pub struct RateLimit<Svc, S, C, K> {
    inner: Svc,
    layer: RateLimitLayer<S, C, K>,
}

impl<Svc: Clone, S, C, K: Clone> Clone for RateLimit<Svc, S, C, K> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            layer: self.layer.clone(),
        }
    }
}

impl<Svc: fmt::Debug, S, C, K: fmt::Debug> fmt::Debug for RateLimit<Svc, S, C, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimit")
            .field("inner", &self.inner)
            .field("layer", &self.layer)
            .finish()
    }
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

impl<Svc, S, C, K, ReqBody, ResBody> Service<Request<ReqBody>> for RateLimit<Svc, S, C, K>
where
    Svc: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    Svc::Future: Send,
    Svc::Error: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Default + Send + 'static,
    S: Store,
    C: Clock,
    K: KeyExtractor,
{
    type Response = Response<ResBody>;
    type Error = Svc::Error;
    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        let Some(key) = self.layer.extractor.extract(&request) else {
            return Box::pin(core::future::ready(Ok(status(
                StatusCode::INTERNAL_SERVER_ERROR,
            ))));
        };
        let rule = self.layer.rule;
        let key = self.layer.limiter.key(&rule, key);
        let limiter = self.layer.limiter.clone();
        // The service that was polled ready is the one to call.
        let clone = self.inner.clone();
        let mut inner = core::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            match limiter.check_key(&rule, key).await {
                Ok(()) => inner.call(request).await,
                Err(exceeded) => {
                    let mut response = status(StatusCode::TOO_MANY_REQUESTS);
                    response
                        .headers_mut()
                        .insert(RETRY_AFTER, HeaderValue::from(exceeded.retry_after_secs()));
                    Ok(response)
                }
            }
        })
    }
}

fn status<B: Default>(status: StatusCode) -> Response<B> {
    let mut response = Response::new(B::default());
    *response.status_mut() = status;
    response
}
