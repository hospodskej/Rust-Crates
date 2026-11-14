//! A [Tower](https://docs.rs/tower) layer for [Axum](https://docs.rs/axum)
//! that adds a strict set of security headers, with a
//! Content-Security-Policy that carries a fresh nonce for every request.
//!
//! A cuticle is a plant's protective outer layer; this is the protective
//! outer layer of an HTTP response.
//!
//! ```
//! use axum::{Router, response::Html, routing::get};
//! use cuticle::{Csp, CspNonce, Cuticle, Source};
//!
//! async fn page(nonce: CspNonce) -> Html<String> {
//!     Html(format!(
//!         r#"<!doctype html><script nonce="{nonce}">document.documentElement.dataset.js = "yes"</script>"#
//!     ))
//! }
//!
//! let app: Router = Router::new()
//!     .route("/", get(page))
//!     .layer(
//!         Cuticle::new()
//!             .csp(Csp::strict().extend("script-src", [Source::host("https://www.googletagmanager.com")]))
//!             .into_layer(),
//!     );
//! ```
//!
//! Every response then carries (with a new nonce each time):
//!
//! ```text
//! content-security-policy: default-src 'self'; script-src 'self' 'nonce-0Hf4…==' https://www.googletagmanager.com; …
//! permissions-policy: accelerometer=(), camera=(), geolocation=(), …
//! x-content-type-options: nosniff
//! referrer-policy: strict-origin-when-cross-origin
//! x-frame-options: DENY
//! cross-origin-opener-policy: same-origin
//! ```
//!
//! # How it works
//!
//! [`Cuticle`] is the builder. [`Cuticle::into_layer`] renders every header
//! once, splitting the policy where the nonce goes, and returns a
//! [`CuticleLayer`]. For each request, its service makes a [`CspNonce`]
//! (128 bits from the operating system's random number generator, base64),
//! puts it in the request's extensions for handlers and templates, and adds
//! the headers to the response with the nonce written in. Headers the
//! handler set are kept unless [`Cuticle::overwrite`] is on.
//!
//! [`NoStore`] is a separate layer for routes whose responses must not be
//! cached, such as the admin, the account and the checkout.
//!
//! # Features
//!
//! - `axum` (default): [`CspNonce`] is an Axum extractor (also as
//!   `Option<CspNonce>`).
//! - `leptos`: `cuticle::leptos::provide_nonce` makes the request's nonce
//!   the one Leptos 0.8 puts on its hydration scripts. Needs Rust 1.88.
//!
//! # Leptos
//!
//! Leptos loads its client code as a WebAssembly module, which a
//! Content-Security-Policy blocks unless `script-src` has
//! `'wasm-unsafe-eval'`. [`Csp::leptos`], the default policy, allows it; the
//! `leptos` module shows how to wire the nonce into `leptos_axum`.
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod csp;
mod error;
mod headers;
mod layer;
mod no_store;
mod nonce;

#[cfg(feature = "leptos")]
#[cfg_attr(docsrs, doc(cfg(feature = "leptos")))]
pub mod leptos;

pub use csp::{Csp, Source};
pub use error::Error;
pub use headers::{
    Allow, CrossOriginOpenerPolicy, Hsts, PermissionsPolicy, ReferrerPolicy, XFrameOptions,
};
pub use layer::{Cuticle, CuticleLayer, CuticleService, ResponseFuture};
pub use no_store::{NoStore, NoStoreFuture, NoStoreService};
pub use nonce::CspNonce;
#[cfg(feature = "axum")]
#[cfg_attr(docsrs, doc(cfg(feature = "axum")))]
pub use nonce::MissingCspNonce;
