//! The layer in an Axum router.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use cuticle::{
    Csp, CspNonce, Cuticle, Hsts, NoStore, PermissionsPolicy, ReferrerPolicy, Source, XFrameOptions,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn page(nonce: CspNonce) -> Html<String> {
    Html(format!(r#"<script nonce="{nonce}">1</script>"#))
}

async fn get_response(app: Router, path: &str) -> axum::response::Response {
    app.oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn body(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn header<'a>(response: &'a axum::response::Response, name: &str) -> Option<&'a str> {
    response.headers().get(name).map(|v| v.to_str().unwrap())
}

fn app(cuticle: Cuticle) -> Router {
    Router::new()
        .route("/", get(page))
        .layer(cuticle.into_layer())
}

#[tokio::test]
async fn default_headers() {
    let response = get_response(app(Cuticle::new()), "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    let csp = header(&response, "content-security-policy").unwrap();
    assert!(csp.starts_with("default-src 'self'; script-src 'self' 'nonce-"));
    assert!(csp.contains("'wasm-unsafe-eval'"));
    assert_eq!(
        header(&response, "permissions-policy"),
        Some(PermissionsPolicy::strict().to_string().as_str())
    );
    assert_eq!(header(&response, "x-content-type-options"), Some("nosniff"));
    assert_eq!(
        header(&response, "referrer-policy"),
        Some("strict-origin-when-cross-origin")
    );
    assert_eq!(header(&response, "x-frame-options"), Some("DENY"));
    assert_eq!(
        header(&response, "cross-origin-opener-policy"),
        Some("same-origin")
    );
    assert_eq!(header(&response, "strict-transport-security"), None);
    assert_eq!(
        header(&response, "content-security-policy-report-only"),
        None
    );
}

#[tokio::test]
async fn nonce_in_extensions_matches_header() {
    let response = get_response(app(Cuticle::new()), "/").await;
    let csp = header(&response, "content-security-policy")
        .unwrap()
        .to_owned();
    let html = body(response).await;
    let nonce = html
        .strip_prefix(r#"<script nonce=""#)
        .and_then(|rest| rest.split('"').next())
        .unwrap();
    assert_eq!(nonce.len(), 24);
    // Both script-src and style-src carry the request's nonce.
    assert_eq!(csp.matches(&format!("'nonce-{nonce}'")).count(), 2);
    assert_eq!(csp.matches("'nonce-").count(), 2);
}

#[tokio::test]
async fn nonce_differs_per_request() {
    let app = app(Cuticle::new());
    let mut seen = std::collections::HashSet::new();
    for _ in 0..50 {
        let response = get_response(app.clone(), "/").await;
        let csp = header(&response, "content-security-policy")
            .unwrap()
            .to_owned();
        assert!(seen.insert(csp));
    }
}

#[tokio::test]
async fn handler_headers_are_kept() {
    let router = |cuticle: Cuticle| {
        Router::new()
            .route(
                "/",
                get(|| async {
                    (
                        [
                            (header::CONTENT_SECURITY_POLICY, "default-src *"),
                            (header::X_FRAME_OPTIONS, "SAMEORIGIN"),
                        ],
                        "embed me",
                    )
                        .into_response()
                }),
            )
            .layer(cuticle.into_layer())
    };

    let kept = get_response(router(Cuticle::new()), "/").await;
    assert_eq!(
        header(&kept, "content-security-policy"),
        Some("default-src *")
    );
    assert_eq!(header(&kept, "x-frame-options"), Some("SAMEORIGIN"));
    assert_eq!(header(&kept, "x-content-type-options"), Some("nosniff"));
    assert_eq!(kept.headers().get_all("x-frame-options").iter().count(), 1);

    let replaced = get_response(router(Cuticle::new().overwrite(true)), "/").await;
    assert!(
        header(&replaced, "content-security-policy")
            .unwrap()
            .starts_with("default-src 'self'")
    );
    assert_eq!(header(&replaced, "x-frame-options"), Some("DENY"));
}

#[tokio::test]
async fn headers_can_be_configured_and_removed() {
    let cuticle = Cuticle::new()
        .csp(None)
        .csp_report_only(Csp::strict().report_uri("/csp-reports"))
        .permissions_policy(None)
        .x_frame_options(None)
        .referrer_policy(ReferrerPolicy::NoReferrer)
        .cross_origin_opener_policy(None)
        .nosniff(false)
        .hsts(Hsts::new(Duration::from_secs(60)).include_subdomains())
        .header(
            header::HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        );
    let response = get_response(app(cuticle), "/").await;
    let headers = response.headers();
    assert!(headers.get("content-security-policy").is_none());
    assert!(
        header(&response, "content-security-policy-report-only")
            .unwrap()
            .ends_with("frame-ancestors 'none'; report-uri /csp-reports")
    );
    assert!(headers.get("permissions-policy").is_none());
    assert!(headers.get("x-frame-options").is_none());
    assert!(headers.get("cross-origin-opener-policy").is_none());
    assert!(headers.get("x-content-type-options").is_none());
    assert_eq!(header(&response, "referrer-policy"), Some("no-referrer"));
    assert_eq!(
        header(&response, "strict-transport-security"),
        Some("max-age=60; includeSubDomains")
    );
    assert_eq!(
        header(&response, "cross-origin-resource-policy"),
        Some("same-origin")
    );
}

#[tokio::test]
async fn empty_sends_nothing_but_still_makes_a_nonce() {
    let response = get_response(app(Cuticle::empty()), "/").await;
    let mut names: Vec<_> = response
        .headers()
        .keys()
        .map(header::HeaderName::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["content-length", "content-type"]);
    assert!(body(response).await.starts_with(r#"<script nonce=""#));
}

#[tokio::test]
async fn missing_layer_is_a_server_error() {
    let app = Router::new().route("/", get(page));
    let response = get_response(app, "/").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let app = Router::new().route(
        "/",
        get(|nonce: Option<CspNonce>| async move { nonce.is_none().to_string() }),
    );
    assert_eq!(body(get_response(app, "/").await).await, "true");
}

#[tokio::test]
async fn csp_without_nonce_is_static() {
    let cuticle = Cuticle::new().csp(Csp::new().default_src([Source::SelfOrigin]));
    let response = get_response(app(cuticle), "/").await;
    assert_eq!(
        header(&response, "content-security-policy"),
        Some("default-src 'self'")
    );
}

#[tokio::test]
async fn no_store_on_some_routes() {
    let private = Router::new()
        .route(
            "/account",
            get(|| async { ([(header::CACHE_CONTROL, "public, max-age=60")], "mine") }),
        )
        .layer(NoStore);
    let app = Router::new().route("/", get(page)).merge(private).layer(
        Cuticle::new()
            .x_frame_options(XFrameOptions::SameOrigin)
            .into_layer(),
    );

    let account = get_response(app.clone(), "/account").await;
    assert_eq!(header(&account, "cache-control"), Some("no-store"));
    assert_eq!(header(&account, "x-frame-options"), Some("SAMEORIGIN"));

    let home = get_response(app, "/").await;
    assert_eq!(header(&home, "cache-control"), None);
}

#[tokio::test]
async fn nested_layers_share_the_nonce() {
    let app = Router::new()
        .route("/", get(page))
        .layer(
            Cuticle::empty()
                .csp(Csp::new().script_src([Source::Nonce]))
                .into_layer(),
        )
        .layer(
            Cuticle::empty()
                .csp_report_only(Csp::new().script_src([Source::Nonce]))
                .into_layer(),
        );
    let response = get_response(app, "/").await;
    let enforced = header(&response, "content-security-policy")
        .unwrap()
        .to_owned();
    let report = header(&response, "content-security-policy-report-only").unwrap();
    assert_eq!(enforced, report);
    let html = body(response).await;
    assert!(enforced.contains(html.split('"').nth(1).unwrap()));
}

#[test]
fn invalid_configuration_is_an_error() {
    let err = Cuticle::new()
        .csp(Csp::strict().img_src([Source::host("https://a.com; script-src *")]))
        .try_into_layer()
        .unwrap_err();
    assert_eq!(err.value(), "https://a.com; script-src *");
    assert!(
        Cuticle::new()
            .permissions_policy(PermissionsPolicy::new().deny("camera=*"))
            .try_into_layer()
            .is_err()
    );
}
