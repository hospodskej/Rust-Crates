//! The policy of a Rails app (`config/initializers/content_security_policy.rb`,
//! its `Permissions-Policy` and `force_ssl`), ported, as a fixture: the
//! headers must match what Rails sent, nonce aside.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use cuticle::{Csp, CspNonce, Cuticle, Hsts, PermissionsPolicy, Source, XFrameOptions};
use tower::ServiceExt;

const GOOGLE_ANALYTICS: [&str; 4] = [
    "https://*.google-analytics.com",
    "https://*.analytics.google.com",
    "https://*.googletagmanager.com",
    "https://www.google.com",
];

fn csp() -> Csp {
    let ga = || GOOGLE_ANALYTICS.map(Source::host);
    Csp::new()
        .default_src([Source::SelfOrigin])
        .script_src([
            Source::SelfOrigin,
            Source::host("https://www.googletagmanager.com"),
            Source::Nonce,
        ])
        .connect_src([Source::SelfOrigin].into_iter().chain(ga()))
        .img_src([Source::SelfOrigin, Source::Data].into_iter().chain(ga()))
        .style_src([Source::SelfOrigin, Source::UnsafeInline])
        .font_src([Source::SelfOrigin, Source::Data])
        .frame_src([Source::host("https://www.google.com")])
        .object_src([Source::None])
        .base_uri([Source::SelfOrigin])
        .form_action([Source::SelfOrigin])
        .frame_ancestors([Source::SelfOrigin])
}

fn cuticle() -> Cuticle {
    Cuticle::new()
        .csp(csp())
        .permissions_policy(
            [
                "camera",
                "microphone",
                "geolocation",
                "usb",
                "payment",
                "gyroscope",
                "magnetometer",
            ]
            .into_iter()
            .fold(PermissionsPolicy::new(), PermissionsPolicy::deny),
        )
        .x_frame_options(XFrameOptions::SameOrigin)
        .hsts(Hsts::new(Duration::from_secs(63_072_000)).include_subdomains())
}

/// What Rails sent, with `{nonce}` for the nonce.
const RAILS_CSP: &str = "default-src 'self'; \
    script-src 'self' https://www.googletagmanager.com 'nonce-{nonce}'; \
    connect-src 'self' https://*.google-analytics.com https://*.analytics.google.com https://*.googletagmanager.com https://www.google.com; \
    img-src 'self' data: https://*.google-analytics.com https://*.analytics.google.com https://*.googletagmanager.com https://www.google.com; \
    style-src 'self' 'unsafe-inline'; \
    font-src 'self' data:; \
    frame-src https://www.google.com; \
    object-src 'none'; \
    base-uri 'self'; \
    form-action 'self'; \
    frame-ancestors 'self'";

const RAILS_PERMISSIONS_POLICY: &str =
    "camera=(), microphone=(), geolocation=(), usb=(), payment=(), gyroscope=(), magnetometer=()";

#[test]
fn renders_like_rails() {
    assert_eq!(csp().to_string(), RAILS_CSP);
}

#[tokio::test]
async fn response_headers_match_rails() {
    let app = Router::new()
        .route("/", get(|nonce: CspNonce| async move { nonce.to_string() }))
        .layer(cuticle().into_layer());
    let response = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let headers = response.headers().clone();
    let get = |name: &str| headers.get(name).unwrap().to_str().unwrap().to_owned();

    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let nonce = std::str::from_utf8(&bytes).unwrap();

    assert_eq!(
        get("content-security-policy"),
        RAILS_CSP.replace("{nonce}", nonce)
    );
    assert_eq!(get("permissions-policy"), RAILS_PERMISSIONS_POLICY);
    assert_eq!(
        get("strict-transport-security"),
        "max-age=63072000; includeSubDomains"
    );
    assert_eq!(get("x-frame-options"), "SAMEORIGIN");
    assert_eq!(get("x-content-type-options"), "nosniff");
    assert_eq!(get("referrer-policy"), "strict-origin-when-cross-origin");
}
