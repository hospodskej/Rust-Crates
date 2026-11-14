//! Giving the nonce to Leptos (the `leptos` feature).
//!
//! Leptos 0.8 puts the nonce from its context (`leptos::nonce::use_nonce`)
//! on the scripts it writes: the hydration scripts, `<HydrationScripts/>`'s
//! module preload, and `leptos_meta`'s `<Script>` and `<Style>`. By default
//! `leptos_axum` provides a nonce of its own, which the header from
//! [`CuticleLayer`](crate::CuticleLayer) would not match. Call
//! [`provide_nonce`] in the `additional_context` closure to replace it with
//! the request's [`CspNonce`]:
//!
//! ```ignore
//! use axum::Router;
//! use leptos::prelude::*;
//! use leptos_axum::{LeptosRoutes, generate_route_list};
//!
//! let leptos_options = get_configuration(None)?.leptos_options;
//! let routes = generate_route_list(App);
//!
//! let app = Router::new()
//!     .leptos_routes_with_context(
//!         &leptos_options,
//!         routes,
//!         cuticle::leptos::provide_nonce,
//!         {
//!             let leptos_options = leptos_options.clone();
//!             move || shell(leptos_options.clone())
//!         },
//!     )
//!     .fallback(leptos_axum::file_and_error_handler(shell))
//!     .with_state(leptos_options)
//!     // Outermost, so the nonce is in the request before Leptos renders.
//!     .layer(cuticle::Cuticle::new().into_layer());
//! ```
//!
//! Turn on Leptos's own `nonce` feature (this feature does it for you) and
//! use [`Csp::leptos`](crate::Csp::leptos), which allows the WebAssembly
//! module. In a component, read the nonce with `use_nonce()` and put it on
//! inline scripts: `<script nonce=use_nonce()>…</script>`.
//!
//! Without this helper the wiring is two lines in `additional_context`:
//!
//! ```ignore
//! let parts = use_context::<http::request::Parts>().unwrap();
//! if let Some(nonce) = parts.extensions.get::<cuticle::CspNonce>() {
//!     provide_context(leptos::nonce::Nonce::from_value(nonce.as_str()));
//! }
//! ```

use leptos::context::provide_context;
use leptos::nonce::Nonce;
use leptos::reactive::owner::with_context;

use crate::CspNonce;

/// Provides the request's [`CspNonce`] as Leptos's [`Nonce`], for
/// `use_nonce()` and the hydration scripts.
///
/// Pass it as (or call it in) `leptos_axum`'s `additional_context`, where
/// `leptos_axum` has already provided the request's `http::request::Parts`.
/// It provides nothing when there are no request parts in the context or
/// the request did not pass through [`CuticleLayer`](crate::CuticleLayer);
/// Leptos then keeps the nonce it made itself, which the header does not
/// allow, so the scripts are blocked and the mistake shows up at once.
pub fn provide_nonce() {
    if let Some(nonce) = request_nonce() {
        provide(&nonce);
    }
}

/// The request's [`CspNonce`], read from the `http::request::Parts` that
/// `leptos_axum` provides as context during server rendering.
#[must_use]
pub fn request_nonce() -> Option<CspNonce> {
    with_context::<http::request::Parts, _>(|parts| parts.extensions.get::<CspNonce>().cloned())
        .flatten()
}

/// Provides the given nonce as Leptos's [`Nonce`], for when you have it at
/// hand, for example from the [`CspNonce`] extractor.
pub fn provide(nonce: &CspNonce) {
    provide_context(Nonce::from(nonce));
}

impl From<&CspNonce> for Nonce {
    fn from(nonce: &CspNonce) -> Self {
        Nonce::from_value(std::sync::Arc::clone(nonce.as_arc()))
    }
}

impl From<CspNonce> for Nonce {
    fn from(nonce: CspNonce) -> Self {
        Nonce::from(&nonce)
    }
}

#[cfg(test)]
mod tests {
    use leptos::nonce::use_nonce;
    use leptos::reactive::owner::Owner;

    use super::*;

    #[test]
    fn provides_the_request_nonce() {
        let nonce = CspNonce::generate();
        let mut request = http::Request::new(());
        request.extensions_mut().insert(nonce.clone());
        let (parts, ()) = request.into_parts();

        let owner = Owner::new();
        owner.with(|| {
            provide_context(parts);
            assert_eq!(request_nonce(), Some(nonce.clone()));
            provide_nonce();
            assert_eq!(use_nonce().as_deref(), Some(nonce.as_str()));
        });
    }

    #[test]
    fn nothing_without_parts() {
        Owner::new().with(|| {
            provide_nonce();
            assert_eq!(use_nonce(), None);
        });
    }
}
