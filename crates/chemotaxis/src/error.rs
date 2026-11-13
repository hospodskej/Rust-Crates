//! The error type.

/// Why a lookup gave no distance.
///
/// [`NotFound`](Error::NotFound) and [`NoRoute`](Error::NoRoute) mean the
/// service worked but the place can't be delivered to by road; the other
/// variants mean the lookup itself failed and may work later. Either way a
/// shop would fall back to "delivery price calculated individually"; use
/// [`Error::is_not_found`] when the two need different wording.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The client was built with an empty API key. No request is sent.
    #[error("no openrouteservice API key")]
    MissingApiKey,

    /// The address or place can't be looked up as given: an empty street,
    /// town or query, a query over 80 characters, or a country that isn't a
    /// two-letter code or isn't one the client allows. No request is sent.
    #[error("invalid lookup: {0}")]
    InvalidInput(&'static str),

    /// Geocoding found nothing for the address or place.
    #[error("address or place not found")]
    NotFound,

    /// The point was found, but the route answer had no distance.
    #[error("no driving route to the found point")]
    NoRoute,

    /// openrouteservice answered with an error status: 403 for a wrong key,
    /// 429 over the daily quota, 404 when no road is near a point, 5xx when
    /// it is down.
    #[error("openrouteservice answered {status}{}", message.as_deref().map(|m| format!(": {m}")).unwrap_or_default())]
    Http {
        /// The HTTP status code.
        status: u16,
        /// The error message from the response body, when it had one.
        message: Option<String>,
    },

    /// Connecting or reading the answer took longer than the timeout
    /// (4 seconds each by default).
    #[error("openrouteservice did not answer in time")]
    Timeout,

    /// The request could not be sent or its answer could not be read
    /// (DNS, TLS, connection reset, invalid JSON).
    #[error("openrouteservice request failed: {0}")]
    Request(#[source] reqwest::Error),

    /// The base URL given to the builder isn't a valid URL.
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(String),
}

impl Error {
    /// Whether the service worked but has no distance for this place
    /// ([`NotFound`](Error::NotFound) or [`NoRoute`](Error::NoRoute)), as
    /// opposed to the lookup failing.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound | Self::NoRoute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_error_shows_the_message() {
        let error = Error::Http {
            status: 404,
            message: Some("Could not find routable point".into()),
        };
        assert_eq!(
            error.to_string(),
            "openrouteservice answered 404: Could not find routable point"
        );
        let error = Error::Http {
            status: 503,
            message: None,
        };
        assert_eq!(error.to_string(), "openrouteservice answered 503");
    }

    #[test]
    fn not_found_is_not_a_failure() {
        assert!(Error::NotFound.is_not_found());
        assert!(Error::NoRoute.is_not_found());
        assert!(!Error::MissingApiKey.is_not_found());
        assert!(
            !Error::Http {
                status: 503,
                message: None
            }
            .is_not_found()
        );
    }
}
