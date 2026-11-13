//! The Tower layer and the client-IP extraction, through an Axum router.
#![cfg(feature = "axum")]

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use refractory::{ClientIp, Limiter, ManualClock, MemoryStore, RateLimitLayer, RemoteIp, Rule};
use tower::ServiceExt;

const SEARCH: Rule = Rule::new("search", 3, Duration::from_secs(60));
const INQUIRY: Rule = Rule::new("inquiry", 2, Duration::from_secs(600));

type TestLimiter = Limiter<MemoryStore, ManualClock>;

fn limiter() -> (TestLimiter, ManualClock) {
    let clock = ManualClock::new();
    (Limiter::builder().clock(clock.clone()).build(), clock)
}

fn layered(limiter: &TestLimiter, client_ip: ClientIp) -> Router {
    Router::new()
        .route("/search", get(|| async { "results" }))
        .layer(RateLimitLayer::new(limiter.clone(), SEARCH, client_ip))
}

fn request(peer: &str, forwarded_for: Option<&str>) -> Request<Body> {
    let mut builder = Request::get("/search");
    if let Some(value) = forwarded_for {
        builder = builder.header("x-forwarded-for", value);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    let peer: SocketAddr = peer.parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(peer));
    request
}

async fn send(app: &Router, request: Request<Body>) -> Response {
    app.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn answers_429_with_retry_after_past_the_limit() {
    let (limiter, clock) = limiter();
    let app = layered(&limiter, ClientIp::direct());

    for _ in 0..3 {
        let response = send(&app, request("203.0.113.7:5000", None)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::RETRY_AFTER).is_none());
    }
    let response = send(&app, request("203.0.113.7:5001", None)).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[header::RETRY_AFTER], "20");

    // Part of a second still rounds up.
    clock.advance(Duration::from_millis(500));
    let response = send(&app, request("203.0.113.7:5002", None)).await;
    assert_eq!(response.headers()[header::RETRY_AFTER], "20");
    clock.advance(Duration::from_secs(19));
    let response = send(&app, request("203.0.113.7:5002", None)).await;
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");

    // Another address is not affected.
    let response = send(&app, request("203.0.113.8:5000", None)).await;
    assert_eq!(response.status(), StatusCode::OK);

    clock.advance(Duration::from_millis(500));
    let response = send(&app, request("203.0.113.7:5003", None)).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn routes_without_the_layer_are_not_limited() {
    let (limiter, _) = limiter();
    let app = Router::new()
        .route("/", get(|| async { "home" }))
        .merge(layered(&limiter, ClientIp::direct()));
    for _ in 0..3 {
        send(&app, request("203.0.113.7:1", None)).await;
    }
    let mut home = request("203.0.113.7:1", None);
    *home.uri_mut() = "/".parse().unwrap();
    assert_eq!(send(&app, home).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn direct_ignores_forwarded_headers() {
    let (limiter, _) = limiter();
    let app = layered(&limiter, ClientIp::direct());
    // A client making up a new X-Forwarded-For each time gets nothing.
    for i in 0..3 {
        let response = send(&app, request("203.0.113.7:1", Some(&format!("10.0.0.{i}")))).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = send(&app, request("203.0.113.7:1", Some("10.0.0.99"))).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn behind_a_proxy_counts_the_address_the_proxy_appended() {
    let (limiter, _) = limiter();
    let app = layered(&limiter, ClientIp::behind_proxy());
    let proxy = "172.18.0.2:40000";

    // The client sends its own made-up entries first; the proxy appends
    // the real address last.
    for fake in ["1.1.1.1", "2.2.2.2", "3.3.3.3"] {
        let value = format!("{fake}, 203.0.113.7");
        let response = send(&app, request(proxy, Some(&value))).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = send(&app, request(proxy, Some("4.4.4.4, 203.0.113.7"))).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // Everyone else behind the same proxy is counted on their own.
    let response = send(&app, request(proxy, Some("198.51.100.1"))).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn client_ip_extraction() {
    use axum::http::{Extensions, HeaderMap, HeaderValue};

    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    let mut extensions = Extensions::new();
    extensions.insert(ConnectInfo(
        "172.18.0.2:40000".parse::<SocketAddr>().unwrap(),
    ));

    let mut headers = HeaderMap::new();
    assert_eq!(
        ClientIp::behind_proxy().ip(&headers, &extensions),
        Some(ip("172.18.0.2"))
    );
    assert_eq!(
        ClientIp::behind_proxy().ip(&headers, &Extensions::new()),
        None
    );

    headers.insert("x-real-ip", HeaderValue::from_static("192.0.2.9"));
    assert_eq!(
        ClientIp::behind_proxy().ip(&headers, &extensions),
        Some(ip("192.0.2.9"))
    );
    assert_eq!(
        ClientIp::direct().ip(&headers, &extensions),
        Some(ip("172.18.0.2"))
    );

    // X-Forwarded-For wins over X-Real-IP, also across several headers.
    headers.append(
        "x-forwarded-for",
        HeaderValue::from_static("1.1.1.1, 10.0.0.1"),
    );
    headers.append("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
    assert_eq!(
        ClientIp::behind_proxy().ip(&headers, &extensions),
        Some(ip("203.0.113.7"))
    );
    assert_eq!(
        ClientIp::behind_proxies(2).ip(&headers, &extensions),
        Some(ip("10.0.0.1"))
    );
    assert_eq!(
        ClientIp::behind_proxies(3).ip(&headers, &extensions),
        Some(ip("1.1.1.1"))
    );
    // Fewer entries than proxies: not from the chain, use X-Real-IP.
    assert_eq!(
        ClientIp::behind_proxies(4).ip(&headers, &extensions),
        Some(ip("192.0.2.9"))
    );

    // Ports, IPv6 and IPv4-mapped addresses.
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("[2001:db8::1]:443"),
    );
    assert_eq!(
        ClientIp::behind_proxy().ip(&headers, &extensions),
        Some(ip("2001:db8::1"))
    );
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static(" ::ffff:203.0.113.7 "),
    );
    assert_eq!(
        ClientIp::behind_proxy().ip(&headers, &extensions),
        Some(ip("203.0.113.7"))
    );
    // Garbage from the proxy's slot: fall back to the peer.
    headers.insert("x-forwarded-for", HeaderValue::from_static("unknown"));
    assert_eq!(
        ClientIp::behind_proxy().ip(&headers, &extensions),
        Some(ip("172.18.0.2"))
    );
}

#[test]
fn ipv6_is_counted_per_network() {
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    let v6 = ip("2001:db8:aaaa:bbbb:1:2:3:4");
    assert_eq!(ClientIp::direct().key_of(v6), ip("2001:db8:aaaa:bbbb::"));
    assert_eq!(
        ClientIp::direct().ipv6_prefix(48).key_of(v6),
        ip("2001:db8:aaaa::")
    );
    assert_eq!(ClientIp::direct().ipv6_prefix(128).key_of(v6), v6);
    assert_eq!(ClientIp::direct().ipv6_prefix(0).key_of(v6), ip("::"));
    assert_eq!(
        ClientIp::direct().key_of(ip("203.0.113.7")),
        ip("203.0.113.7")
    );
}

#[tokio::test]
async fn ipv6_neighbours_share_a_limit() {
    let (limiter, _) = limiter();
    let app = layered(&limiter, ClientIp::direct());
    for i in 1..=3 {
        let peer = format!("[2001:db8:1:2::{i}]:443");
        assert_eq!(
            send(&app, request(&peer, None)).await.status(),
            StatusCode::OK
        );
    }
    let response = send(&app, request("[2001:db8:1:2:ffff::]:443", None)).await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let response = send(&app, request("[2001:db8:1:3::1]:443", None)).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_missing_address_is_a_server_error_not_a_free_pass() {
    let (limiter, _) = limiter();
    let app = layered(&limiter, ClientIp::direct());
    let request = Request::get("/search").body(Body::empty()).unwrap();
    let response = send(&app, request).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

/// A handler that checks by itself and redirects back with a message, as
/// the Rails app did.
async fn inquiry(State(limiter): State<TestLimiter>, ip: RemoteIp) -> Response {
    match limiter.check(&INQUIRY, ip.key()).await {
        Ok(()) => Redirect::to("/thanks").into_response(),
        Err(exceeded) => Redirect::to(&format!(
            "/contact?alert=too-many-attempts&retry={}",
            exceeded.retry_after_secs()
        ))
        .into_response(),
    }
}

#[tokio::test]
async fn handlers_check_and_redirect_with_a_message() {
    let (limiter, _) = limiter();
    let app = Router::new()
        .route("/contact", post(inquiry))
        .layer(Extension(ClientIp::behind_proxy()))
        .with_state(limiter);

    let post = |forwarded_for: &str| {
        let mut request = Request::post("/contact")
            .header("x-forwarded-for", forwarded_for)
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "172.18.0.2:40000".parse::<SocketAddr>().unwrap(),
        ));
        request
    };
    let location = |response: &Response| {
        response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_owned()
    };

    for _ in 0..2 {
        let response = send(&app, post("203.0.113.7")).await;
        assert_eq!(location(&response), "/thanks");
    }
    let response = send(&app, post("203.0.113.7")).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        location(&response),
        "/contact?alert=too-many-attempts&retry=300"
    );

    let response = send(&app, post("198.51.100.1")).await;
    assert_eq!(location(&response), "/thanks");
}

#[tokio::test]
async fn remote_ip_without_an_address_is_rejected() {
    let app: Router = Router::new().route("/", get(|ip: RemoteIp| async move { ip.to_string() }));
    let response = send(&app, Request::get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let mut request = Request::get("/").body(Body::empty()).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "[::ffff:203.0.113.7]:1".parse::<SocketAddr>().unwrap(),
    ));
    let response = send(&app, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(&body[..], b"203.0.113.7");
}

#[tokio::test]
async fn many_concurrent_requests_through_the_layer() {
    let (limiter, _) = limiter();
    let app = layered(&limiter, ClientIp::direct());
    let mut tasks = Vec::new();
    for i in 0..200 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            let peer = format!("203.0.113.{}:1", i % 5);
            app.oneshot(request(&peer, None)).await.unwrap().status()
        }));
    }
    let mut ok = 0;
    for task in tasks {
        if task.await.unwrap() == StatusCode::OK {
            ok += 1;
        }
    }
    assert_eq!(ok, 5 * 3);
}
