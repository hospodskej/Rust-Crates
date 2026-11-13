# refractory

Rate limiting for Rust web apps: how often something may happen per key (an IP address, an e-mail, ...) within a window. A check API for handlers that decide their own answer, and an optional Tower/[Axum](https://docs.rs/axum) layer that answers `429 Too Many Requests` with `Retry-After`.

A neuron's refractory period is the moment after it fires when it can't fire again; this crate gives a login form one.

## In a handler

The handler decides what to answer when a limit is hit, for example a redirect back with a message:

```rust
use std::time::Duration;
use axum::{Extension, Form, Router, extract::State, response::Redirect, routing::post};
use refractory::{ClientIp, Limiter, RemoteIp, Rule, normalize_email};

// 10 attempts per 3 minutes from one address, and 20 per hour against one
// account from any address.
const LOGIN: Rule = Rule::new("login", 10, Duration::from_secs(3 * 60));
const LOGIN_PER_ACCOUNT: Rule = Rule::new("login-per-account", 20, Duration::from_secs(60 * 60));

#[derive(serde::Deserialize)]
struct LoginForm { email: String, password: String }

async fn login(State(limiter): State<Limiter>, ip: RemoteIp, Form(form): Form<LoginForm>) -> Redirect {
    let email = normalize_email(&form.email); // trimmed, lowercase
    let allowed = match limiter.check(&LOGIN, ip.key()).await {
        Ok(()) => limiter.check(&LOGIN_PER_ACCOUNT, &email).await,
        refused => refused,
    };
    if let Err(exceeded) = allowed {
        let minutes = exceeded.retry_after_secs().div_ceil(60);
        return Redirect::to(&format!("/login?alert=too-many-attempts&minutes={minutes}"));
    }
    // ... check the password ...
    Redirect::to("/account")
}

let app: Router = Router::new()
    .route("/login", post(login))
    .layer(Extension(ClientIp::behind_proxy())) // where RemoteIp reads the address
    .with_state(Limiter::new());
```

Serve the app with `into_make_service_with_connect_info::<SocketAddr>()`, so the client's address is known. [`examples/shop.rs`](examples/shop.rs) has every limit of a small shop.

Without Axum, `limiter.check(&rule, key).await` works with any key that implements `Hash`, and `limiter.check_sync(&rule, key)` needs no async runtime at all.

## As a layer

```rust
use refractory::{ClientIp, Limiter, RateLimitLayer, Rule};

const SUGGESTIONS: Rule = Rule::new("address-suggestions", 120, std::time::Duration::from_secs(60));

let app: axum::Router = axum::Router::new()
    .route("/address-suggestions", axum::routing::get(|| async { "[]" }))
    .layer(RateLimitLayer::new(Limiter::new(), SUGGESTIONS, ClientIp::behind_proxy()));
```

Past the limit the route is not called; the answer is `429` with `Retry-After` in whole seconds. The layer counts per key from a `KeyExtractor`; `ClientIp` is the one for addresses:

- `ClientIp::direct()` (the default): the TCP peer's address from `ConnectInfo<SocketAddr>`; no header is trusted.
- `ClientIp::behind_proxy()`: behind one reverse proxy such as kamal-proxy, the **last** `X-Forwarded-For` entry (the one the proxy appended; the client can only add entries before it), then `X-Real-IP`, then the peer. `behind_proxies(n)` for a chain. Use it only when nothing but the proxy can reach the app.
- IPv6 addresses count per `/64` network (`.ipv6_prefix(bits)` to change it).

## How it counts

The Generic Cell Rate Algorithm (GCRA): per rule and key, one stored timestamp and a few arithmetic operations per check. A rule of `limit` per `window` allows a burst of exactly `limit` (one more is refused), then one more every `window / limit`, and the whole burst again after a quiet `window`. `Exceeded::retry_after` is the exact time to the next allowed event. Unlike fixed windows (Rails' `rate_limit`), there is no window edge where `2 × limit` can come back to back.

## State, keys and privacy

- `MemoryStore` (the default) keeps the state in the process, in a sharded map. It forgets keys that have their full allowance back, as it is used and on `limiter.purge()`, and keeps at most 100,000 keys (`MemoryStore::new().max_keys(n)`), so a flood of made-up keys can't fill the memory.
- The `Store` trait returns futures, so a store shared by several processes (SQLite, Redis) can be added; `refractory::gcra::step` does the arithmetic. If a store fails, events are allowed, unless the limiter is built with `.fail_closed(true)`.
- Keys are never stored as given: the store gets a 128-bit SipHash of the rule's name and the key, with a secret that is random per limiter. E-mail addresses used as keys don't show up in the store or its debug output. It is not encryption: whoever has the process's memory, secret included, can check a guessed address.
- `ManualClock` lets tests move time without sleeping.

## Features

| Feature | Default | What it adds |
|---|---|---|
| `axum` | yes | `RateLimitLayer`, `KeyExtractor`, `ClientIp`, the `RemoteIp` extractor |

Without `axum`, the only dependency is `siphasher`; no Axum, no Tokio.

## Licence

Dual-licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
