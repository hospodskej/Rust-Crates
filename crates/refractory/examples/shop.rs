//! The rate limits of a small shop, as a Rails app had them with
//! `rate_limit`, ported to Axum.
//!
//! Form posts that hit a limit are sent back to their page with a message
//! (Rails' `redirect_back ... alert:`); here the message goes into the
//! query string, where a real app would use its flash. The address
//! suggestions, an API for a script, get a plain 429 from the layer.
//!
//! ```sh
//! cargo run -p refractory --example shop
//! for i in $(seq 11); do curl -si -X POST localhost:3000/login -d email=jana@example.cz | grep -i location; done
//! ```

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{Form, State};
use axum::http::HeaderMap;
use axum::http::header::REFERER;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use refractory::{ClientIp, Exceeded, Limiter, RateLimitLayer, RemoteIp, Rule, normalize_email};

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(60 * 60);

// Password guessing: from one address, and against one account from many.
const LOGIN: Rule = Rule::new("login", 10, Duration::from_secs(3 * 60));
const LOGIN_PER_ACCOUNT: Rule = Rule::new("login-per-account", 20, HOUR);
const REGISTRATION: Rule = Rule::new("registration", 5, HOUR);
const DISCOUNT_CODE: Rule = Rule::new("discount-code", 10, Duration::from_secs(10 * 60));
const DELIVERY_ESTIMATE: Rule = Rule::new("delivery-estimate", 20, Duration::from_secs(10 * 60));
const INQUIRY: Rule = Rule::new("inquiry", 5, Duration::from_secs(10 * 60));
const ORDER: Rule = Rule::new("order", 5, Duration::from_secs(10 * 60));
const PASSWORD_RESET: Rule = Rule::new("password-reset", 5, Duration::from_secs(10 * 60));
const ADDRESS_SUGGESTIONS: Rule = Rule::new("address-suggestions", 120, MINUTE);

#[derive(serde::Deserialize)]
struct LoginForm {
    email: String,
}

/// Back to the page the form was on, with a message. Only the path of the
/// `Referer` is used, so the redirect never leaves the site.
fn too_many_attempts(headers: &HeaderMap, exceeded: Exceeded) -> Redirect {
    let back = headers
        .get(REFERER)
        .and_then(|value| value.to_str().ok())
        .map(|referer| match referer.split_once("://") {
            Some((_, rest)) => rest.find('/').map_or("/", |i| &rest[i..]),
            None => referer,
        })
        .map(|path| path.split(['?', '#']).next().unwrap_or("/"))
        .filter(|path| path.starts_with('/') && !path.starts_with("//"))
        .unwrap_or("/");
    let minutes = exceeded.retry_after_secs().div_ceil(60);
    Redirect::to(&format!(
        "{back}?alert=Too+many+attempts.+Try+again+in+{minutes}+min."
    ))
}

async fn login(
    State(limiter): State<Limiter>,
    ip: RemoteIp,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Redirect {
    let email = normalize_email(&form.email);
    let allowed = match limiter.check(&LOGIN, ip.key()).await {
        Ok(()) => limiter.check(&LOGIN_PER_ACCOUNT, &email).await,
        Err(exceeded) => Err(exceeded),
    };
    if let Err(exceeded) = allowed {
        return too_many_attempts(&headers, exceeded);
    }
    // ... check the password ...
    Redirect::to("/account")
}

/// One handler for the forms limited per address only.
async fn limited_form(rule: Rule, limiter: Limiter, ip: RemoteIp, headers: HeaderMap) -> Redirect {
    match limiter.check(&rule, ip.key()).await {
        Ok(()) => Redirect::to("/thanks"),
        Err(exceeded) => too_many_attempts(&headers, exceeded),
    }
}

fn app(limiter: Limiter) -> Router {
    let form = |rule: Rule| {
        post(
            move |State(limiter): State<Limiter>, ip: RemoteIp, headers: HeaderMap| {
                limited_form(rule, limiter, ip, headers)
            },
        )
    };
    let suggestions = Router::new()
        .route(
            "/address-suggestions",
            get(|| async { Json(Vec::<String>::new()) }),
        )
        .layer(RateLimitLayer::new(
            limiter.clone(),
            ADDRESS_SUGGESTIONS,
            ClientIp::behind_proxy(),
        ));
    Router::new()
        .route("/login", post(login))
        .route("/registration", form(REGISTRATION))
        .route("/discount-code", form(DISCOUNT_CODE))
        .route("/delivery-estimate", form(DELIVERY_ESTIMATE))
        .route("/inquiry", form(INQUIRY))
        .route("/checkout/confirm", form(ORDER))
        .route("/password-reset", form(PASSWORD_RESET))
        .with_state(limiter)
        .merge(suggestions)
        // The shop runs behind kamal-proxy, the only way in.
        .layer(Extension(ClientIp::behind_proxy()))
}

#[tokio::main]
async fn main() {
    let limiter = Limiter::new();
    // Give memory back after a flood even when it is quiet.
    let purging = limiter.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(MINUTE).await;
            purging.purge();
        }
    });
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_owned());
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    axum::serve(
        listener,
        app(limiter).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
