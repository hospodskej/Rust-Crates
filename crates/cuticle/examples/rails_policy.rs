//! The headers of a Rails shop, ported: its `content_security_policy.rb`
//! initializer, its `Permissions-Policy` and the HSTS that `force_ssl` sends.
//!
//! Run with `cargo run --example rails_policy` to print the headers of one
//! response.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Request};
use axum::response::Html;
use axum::routing::get;
use cuticle::{
    Allow, Csp, CspNonce, Cuticle, Hsts, NoStore, PermissionsPolicy, Source, XFrameOptions,
};
use tower::ServiceExt;

/// The Content-Security-Policy: the browser runs only the site's own scripts
/// (and Google Analytics), so injected HTML can't run a script or send data
/// elsewhere.
fn csp() -> Csp {
    // Where Google Analytics sends its measurements (www.google.com/g/collect
    // with consent mode).
    let google_analytics = || {
        [
            "https://*.google-analytics.com",
            "https://*.analytics.google.com",
            "https://*.googletagmanager.com",
            "https://www.google.com",
        ]
        .map(Source::host)
    };

    Csp::new()
        .default_src([Source::SelfOrigin])
        // The few inline scripts (theme, Google Analytics, the purchase
        // event) carry the nonce.
        .script_src([
            Source::SelfOrigin,
            Source::host("https://www.googletagmanager.com"),
            Source::Nonce,
        ])
        .connect_src([Source::SelfOrigin].into_iter().chain(google_analytics()))
        .img_src(
            [Source::SelfOrigin, Source::Data]
                .into_iter()
                .chain(google_analytics()),
        )
        // style="" attributes are used throughout; styles can't run code.
        .style_src([Source::SelfOrigin, Source::UnsafeInline])
        .font_src([Source::SelfOrigin, Source::Data])
        .frame_src([Source::host("https://www.google.com")]) // the map in the footer
        .object_src([Source::None])
        .base_uri([Source::SelfOrigin])
        .form_action([Source::SelfOrigin])
        .frame_ancestors([Source::SelfOrigin])
}

/// Browser features the site doesn't use are off for every page (and for
/// anything injected into one).
fn permissions_policy() -> PermissionsPolicy {
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
    .fold(PermissionsPolicy::new(), PermissionsPolicy::deny)
}

fn cuticle(production: bool) -> Cuticle {
    // Rails' `config.force_ssl = true`: two years, with subdomains.
    let hsts = Hsts::new(Duration::from_secs(2 * 365 * 24 * 60 * 60)).include_subdomains();
    Cuticle::new()
        .csp(csp())
        .permissions_policy(permissions_policy())
        // Rails' default; frame-ancestors 'self' says the same.
        .x_frame_options(XFrameOptions::SameOrigin)
        .header(
            HeaderName::from_static("x-permitted-cross-domain-policies"),
            HeaderValue::from_static("none"),
        )
        .hsts(production.then_some(hsts))
}

async fn home(nonce: CspNonce) -> Html<String> {
    Html(format!(
        r#"<script nonce="{nonce}">document.documentElement.dataset.theme = "light"</script>"#
    ))
}

#[tokio::main]
async fn main() {
    // Unused here, but this is how a geolocation exception would look.
    let _ = PermissionsPolicy::new().allow("geolocation", [Allow::SelfOrigin]);

    let account = Router::new()
        .route("/account", get(|| async { "orders" }))
        .layer(NoStore);
    let app = Router::new()
        .route("/", get(home))
        .merge(account)
        .layer(cuticle(true).into_layer());

    let response = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    for (name, value) in response.headers() {
        println!("{name}: {}", value.to_str().unwrap());
    }
}
