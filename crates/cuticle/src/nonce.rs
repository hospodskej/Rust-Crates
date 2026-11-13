//! The per-request nonce.

use std::fmt;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// The nonce of the current request.
///
/// [`CuticleLayer`](crate::CuticleLayer) makes a new one for every request,
/// puts it in the request's extensions and writes it into the
/// Content-Security-Policy of the response wherever the policy has
/// [`Source::Nonce`](crate::Source::Nonce). Put it on inline scripts:
/// `<script nonce="{nonce}">…</script>`.
///
/// It is 128 bits from the operating system's random number generator,
/// base64-encoded (24 characters). Cloning is cheap.
///
/// With the `axum` feature, it is an extractor:
///
/// ```
/// use axum::response::Html;
/// use cuticle::CspNonce;
///
/// async fn page(nonce: CspNonce) -> Html<String> {
///     Html(format!(r#"<script nonce="{nonce}">console.log("hi")</script>"#))
/// }
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CspNonce(Arc<str>);

impl CspNonce {
    /// Makes a new random nonce.
    ///
    /// # Panics
    ///
    /// If the operating system's random number generator fails, which it
    /// does not do once the system has booted.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("the system random number generator failed");
        Self(STANDARD.encode(bytes).into())
    }

    /// The nonce as text, as it goes into a `nonce` attribute.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(feature = "leptos")]
    pub(crate) fn as_arc(&self) -> &Arc<str> {
        &self.0
    }
}

impl fmt::Display for CspNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for CspNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CspNonce").field(&&*self.0).finish()
    }
}

impl AsRef<str> for CspNonce {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The rejection when [`CspNonce`] is extracted but the request did not pass
/// through [`CuticleLayer`](crate::CuticleLayer). Responds with
/// `500 Internal Server Error`, since this is a mistake in setting up the
/// router. Extract `Option<CspNonce>` to handle it yourself.
#[cfg(feature = "axum")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MissingCspNonce;

#[cfg(feature = "axum")]
impl fmt::Display for MissingCspNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no CSP nonce in the request; is the cuticle layer on this route?")
    }
}

#[cfg(feature = "axum")]
impl std::error::Error for MissingCspNonce {}

#[cfg(feature = "axum")]
mod extract {
    use std::convert::Infallible;

    use axum_core::extract::{FromRequestParts, OptionalFromRequestParts};
    use axum_core::response::{IntoResponse, Response};
    use http::StatusCode;
    use http::request::Parts;

    use super::{CspNonce, MissingCspNonce};

    impl IntoResponse for MissingCspNonce {
        fn into_response(self) -> Response {
            (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
        }
    }

    impl<S: Send + Sync> FromRequestParts<S> for CspNonce {
        type Rejection = MissingCspNonce;

        async fn from_request_parts(
            parts: &mut Parts,
            _state: &S,
        ) -> Result<Self, Self::Rejection> {
            parts
                .extensions
                .get::<CspNonce>()
                .cloned()
                .ok_or(MissingCspNonce)
        }
    }

    impl<S: Send + Sync> OptionalFromRequestParts<S> for CspNonce {
        type Rejection = Infallible;

        async fn from_request_parts(
            parts: &mut Parts,
            _state: &S,
        ) -> Result<Option<Self>, Self::Rejection> {
            Ok(parts.extensions.get::<CspNonce>().cloned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format() {
        let nonce = CspNonce::generate();
        assert_eq!(nonce.as_str().len(), 24);
        assert!(nonce.as_str().ends_with("=="));
        let bytes = STANDARD.decode(nonce.as_str()).unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(nonce.to_string(), nonce.as_str());
    }

    #[test]
    fn unique() {
        let nonces: std::collections::HashSet<_> =
            (0..10_000).map(|_| CspNonce::generate()).collect();
        assert_eq!(nonces.len(), 10_000);
    }
}
