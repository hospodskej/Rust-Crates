//! The builder, the layer and the service.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Request, Response};
use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;

use crate::{
    CrossOriginOpenerPolicy, Csp, CspNonce, Error, Hsts, PermissionsPolicy, ReferrerPolicy,
    XFrameOptions,
};

/// Chooses the security headers; turn it into a [`CuticleLayer`] with
/// [`Cuticle::into_layer`].
///
/// [`Cuticle::new`] starts from secure defaults:
///
/// | Header | Value |
/// |---|---|
/// | `Content-Security-Policy` | [`Csp::leptos`] (strict, with the nonce and `'wasm-unsafe-eval'`) |
/// | `Permissions-Policy` | [`PermissionsPolicy::strict`] |
/// | `X-Content-Type-Options` | `nosniff` |
/// | `Referrer-Policy` | `strict-origin-when-cross-origin` |
/// | `X-Frame-Options` | `DENY` |
/// | `Cross-Origin-Opener-Policy` | `same-origin` |
/// | `Strict-Transport-Security` | not sent; turn on with [`Cuticle::hsts`] in production |
///
/// Every setter takes the value or `None`, which removes the header.
///
/// ```
/// use std::time::Duration;
/// use axum::{Router, routing::get};
/// use cuticle::{Csp, Cuticle, Hsts, PermissionsPolicy, Source, XFrameOptions};
///
/// let production = true;
/// let cuticle = Cuticle::new()
///     .csp(Csp::leptos().extend("img-src", [Source::host("https://images.example.com")]))
///     .permissions_policy(PermissionsPolicy::strict().deny("display-capture"))
///     .x_frame_options(XFrameOptions::SameOrigin)
///     .hsts(production.then(|| Hsts::new(Duration::from_secs(63_072_000)).include_subdomains()));
///
/// let app: Router = Router::new()
///     .route("/", get(|| async { "hello" }))
///     .layer(cuticle.into_layer());
/// ```
///
/// By default a header the handler (or an inner layer) already set is left
/// alone, so a route can send its own policy; see [`Cuticle::overwrite`].
#[derive(Clone, Debug)]
#[must_use]
pub struct Cuticle {
    csp: Option<Csp>,
    csp_report_only: Option<Csp>,
    permissions_policy: Option<PermissionsPolicy>,
    hsts: Option<Hsts>,
    x_frame_options: Option<XFrameOptions>,
    referrer_policy: Option<ReferrerPolicy>,
    cross_origin_opener_policy: Option<CrossOriginOpenerPolicy>,
    nosniff: bool,
    extra: Vec<(HeaderName, HeaderValue)>,
    overwrite: bool,
}

impl Default for Cuticle {
    fn default() -> Self {
        Self::new()
    }
}

impl Cuticle {
    /// The secure defaults listed above.
    pub fn new() -> Self {
        Self {
            csp: Some(Csp::leptos()),
            csp_report_only: None,
            permissions_policy: Some(PermissionsPolicy::strict()),
            hsts: None,
            x_frame_options: Some(XFrameOptions::Deny),
            referrer_policy: Some(ReferrerPolicy::StrictOriginWhenCrossOrigin),
            cross_origin_opener_policy: Some(CrossOriginOpenerPolicy::SameOrigin),
            nosniff: true,
            extra: Vec::new(),
            overwrite: false,
        }
    }

    /// No headers at all, to add only the ones you want. The nonce is still
    /// made and put in the request's extensions.
    pub fn empty() -> Self {
        Self {
            csp: None,
            csp_report_only: None,
            permissions_policy: None,
            hsts: None,
            x_frame_options: None,
            referrer_policy: None,
            cross_origin_opener_policy: None,
            nosniff: false,
            extra: Vec::new(),
            overwrite: false,
        }
    }

    /// The enforced `Content-Security-Policy`, or `None` for none.
    pub fn csp(mut self, csp: impl Into<Option<Csp>>) -> Self {
        self.csp = csp.into();
        self
    }

    /// A `Content-Security-Policy-Report-Only` policy: violations are
    /// reported (see [`Csp::report_uri`] and [`Csp::report_to`]) and logged
    /// in the browser console, but not blocked. It can be sent alongside the
    /// enforced policy, for example to try a stricter one; to only report,
    /// also call `.csp(None)`. Its nonce is the same as the enforced policy's.
    ///
    /// ```
    /// use cuticle::{Csp, Cuticle};
    ///
    /// // Try the policy out before enforcing it.
    /// let cuticle = Cuticle::new()
    ///     .csp(None)
    ///     .csp_report_only(Csp::leptos().report_uri("/csp-reports"));
    /// ```
    pub fn csp_report_only(mut self, csp: impl Into<Option<Csp>>) -> Self {
        self.csp_report_only = csp.into();
        self
    }

    /// `Permissions-Policy`, or `None` for none.
    pub fn permissions_policy(mut self, policy: impl Into<Option<PermissionsPolicy>>) -> Self {
        self.permissions_policy = policy.into();
        self
    }

    /// `Strict-Transport-Security`, off by default. Turn it on only where the
    /// site is served over HTTPS.
    pub fn hsts(mut self, hsts: impl Into<Option<Hsts>>) -> Self {
        self.hsts = hsts.into();
        self
    }

    /// `X-Frame-Options`, or `None` to rely on the CSP's `frame-ancestors`
    /// (which every current browser supports).
    pub fn x_frame_options(mut self, value: impl Into<Option<XFrameOptions>>) -> Self {
        self.x_frame_options = value.into();
        self
    }

    /// `Referrer-Policy`, or `None` for the browser's default.
    pub fn referrer_policy(mut self, value: impl Into<Option<ReferrerPolicy>>) -> Self {
        self.referrer_policy = value.into();
        self
    }

    /// `Cross-Origin-Opener-Policy`, or `None` for none.
    pub fn cross_origin_opener_policy(
        mut self,
        value: impl Into<Option<CrossOriginOpenerPolicy>>,
    ) -> Self {
        self.cross_origin_opener_policy = value.into();
        self
    }

    /// Whether to send `X-Content-Type-Options: nosniff`, which stops the
    /// browser from guessing a response is a script or stylesheet when its
    /// `Content-Type` says otherwise.
    pub fn nosniff(mut self, on: bool) -> Self {
        self.nosniff = on;
        self
    }

    /// Any other header, sent like the rest (for example
    /// `Cross-Origin-Resource-Policy` or `X-Permitted-Cross-Domain-Policies`).
    ///
    /// ```
    /// use cuticle::Cuticle;
    /// use http::{HeaderName, HeaderValue};
    ///
    /// let cuticle = Cuticle::new().header(
    ///     HeaderName::from_static("x-permitted-cross-domain-policies"),
    ///     HeaderValue::from_static("none"),
    /// );
    /// ```
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.extra.retain(|(n, _)| *n != name);
        self.extra.push((name, value));
        self
    }

    /// Whether to replace headers the handler already set. Off by default,
    /// so a handler can send its own policy for one route.
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Checks the configuration and builds the layer.
    ///
    /// # Errors
    ///
    /// If a host, origin, directive or feature name contains characters that
    /// would break the header, such as spaces, `;` or `,`.
    pub fn try_into_layer(self) -> Result<CuticleLayer, Error> {
        fn text(value: &impl std::fmt::Display) -> Result<HeaderValue, Error> {
            let value = value.to_string();
            HeaderValue::try_from(value.as_str()).map_err(|_| Error::new("header value", &value))
        }

        let mut headers = Vec::new();
        for (name, csp) in [
            (header::CONTENT_SECURITY_POLICY, &self.csp),
            (
                header::CONTENT_SECURITY_POLICY_REPORT_ONLY,
                &self.csp_report_only,
            ),
        ] {
            if let Some(csp) = csp {
                csp.validate()?;
                headers.push((name, Value::from_parts(csp.render_parts(None))?));
            }
        }
        if let Some(policy) = &self.permissions_policy {
            policy.validate()?;
            headers.push((
                HeaderName::from_static("permissions-policy"),
                Value::Static(text(policy)?),
            ));
        }
        let fixed = [
            (
                header::STRICT_TRANSPORT_SECURITY,
                self.hsts.map(|hsts| text(&hsts)).transpose()?,
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                self.nosniff.then(|| HeaderValue::from_static("nosniff")),
            ),
            (
                header::REFERRER_POLICY,
                self.referrer_policy
                    .map(|v| HeaderValue::from_static(v.as_str())),
            ),
            (
                header::X_FRAME_OPTIONS,
                self.x_frame_options
                    .map(|v| HeaderValue::from_static(v.as_str())),
            ),
            (
                HeaderName::from_static("cross-origin-opener-policy"),
                self.cross_origin_opener_policy
                    .map(|v| HeaderValue::from_static(v.as_str())),
            ),
        ];
        for (name, value) in fixed {
            if let Some(value) = value {
                headers.push((name, Value::Static(value)));
            }
        }
        for (name, value) in self.extra {
            headers.retain(|(n, _)| *n != name);
            headers.push((name, Value::Static(value)));
        }

        Ok(CuticleLayer {
            config: Arc::new(Config {
                headers,
                overwrite: self.overwrite,
            }),
        })
    }

    /// Builds the layer.
    ///
    /// # Panics
    ///
    /// If the configuration is invalid; see [`Cuticle::try_into_layer`].
    #[must_use]
    pub fn into_layer(self) -> CuticleLayer {
        match self.try_into_layer() {
            Ok(layer) => layer,
            Err(err) => panic!("{err}"),
        }
    }
}

impl From<Cuticle> for CuticleLayer {
    /// Same as [`Cuticle::into_layer`], including the panic.
    fn from(cuticle: Cuticle) -> Self {
        cuticle.into_layer()
    }
}

#[derive(Debug)]
struct Config {
    headers: Vec<(HeaderName, Value)>,
    overwrite: bool,
}

#[derive(Debug)]
enum Value {
    Static(HeaderValue),
    /// The text between nonces.
    Template(Box<[Box<str>]>),
}

impl Value {
    fn from_parts(parts: Vec<String>) -> Result<Self, Error> {
        if let [only] = parts.as_slice() {
            return HeaderValue::try_from(only.as_str())
                .map(Self::Static)
                .map_err(|_| Error::new("header value", only));
        }
        // Check the whole value once with a sample nonce, so filling it in
        // per request cannot fail.
        let sample = parts.join("AAAAAAAAAAAAAAAAAAAAAA==");
        HeaderValue::try_from(sample.as_str()).map_err(|_| Error::new("header value", &sample))?;
        Ok(Self::Template(
            parts.into_iter().map(String::into_boxed_str).collect(),
        ))
    }

    fn to_header_value(&self, nonce: &CspNonce) -> Option<HeaderValue> {
        match self {
            Self::Static(value) => Some(value.clone()),
            Self::Template(parts) => {
                let nonce = nonce.as_str();
                let len =
                    parts.iter().map(|p| p.len()).sum::<usize>() + nonce.len() * (parts.len() - 1);
                let mut out = String::with_capacity(len);
                for (i, part) in parts.iter().enumerate() {
                    if i > 0 {
                        out.push_str(nonce);
                    }
                    out.push_str(part);
                }
                HeaderValue::try_from(out).ok()
            }
        }
    }
}

impl Config {
    fn apply(&self, headers: &mut HeaderMap, nonce: &CspNonce) {
        for (name, value) in &self.headers {
            if !self.overwrite && headers.contains_key(name) {
                continue;
            }
            if let Some(value) = value.to_header_value(nonce) {
                headers.insert(name.clone(), value);
            }
        }
    }
}

/// The [`Layer`] that adds the security headers; made by [`Cuticle::into_layer`].
///
/// Cloning is cheap: the rendered headers are shared.
#[derive(Clone, Debug)]
pub struct CuticleLayer {
    config: Arc<Config>,
}

impl Default for CuticleLayer {
    /// The layer for [`Cuticle::new`].
    fn default() -> Self {
        Cuticle::new().into_layer()
    }
}

impl<S> Layer<S> for CuticleLayer {
    type Service = CuticleService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CuticleService {
            inner,
            config: Arc::clone(&self.config),
        }
    }
}

/// The [`Service`] made by [`CuticleLayer`].
///
/// For each request it makes a [`CspNonce`] (or keeps one already in the
/// request's extensions, so nested layers agree), puts it in the request's
/// extensions, and adds the headers to the response.
#[derive(Clone, Debug)]
pub struct CuticleService<S> {
    inner: S,
    config: Arc<Config>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for CuticleService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let nonce = if let Some(nonce) = req.extensions().get::<CspNonce>() {
            nonce.clone()
        } else {
            let nonce = CspNonce::generate();
            req.extensions_mut().insert(nonce.clone());
            nonce
        };
        ResponseFuture {
            inner: self.inner.call(req),
            config: Arc::clone(&self.config),
            nonce,
        }
    }
}

pin_project! {
    /// The response future of [`CuticleService`].
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        config: Arc<Config>,
        nonce: CspNonce,
    }
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let mut response = ready!(this.inner.poll(cx))?;
        this.config.apply(response.headers_mut(), this.nonce);
        Poll::Ready(Ok(response))
    }
}

impl<F> std::fmt::Debug for ResponseFuture<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResponseFuture")
            .field("nonce", &self.nonce)
            .finish_non_exhaustive()
    }
}
