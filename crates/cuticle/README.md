# cuticle

A [Tower](https://docs.rs/tower) layer for [Axum](https://docs.rs/axum) 0.8 that adds a strict set of security headers, with a Content-Security-Policy that carries a fresh nonce for every request. Handlers and templates (Leptos 0.8 included) read the nonce and put it on their inline `<script nonce>` tags.

A cuticle is a plant's protective outer layer; this is the protective outer layer of an HTTP response.

```rust
use axum::{Router, response::Html, routing::get};
use cuticle::{Csp, CspNonce, Cuticle, NoStore, Source};

async fn page(nonce: CspNonce) -> Html<String> {
    Html(format!(r#"<script nonce="{nonce}">console.log("allowed")</script>"#))
}

let account: Router = Router::new()
    .route("/account", get(|| async { "private" }))
    .layer(NoStore); // Cache-Control: no-store

let app: Router = Router::new()
    .route("/", get(page))
    .merge(account)
    .layer(
        Cuticle::new()
            .csp(Csp::strict().extend("script-src", [Source::host("https://www.googletagmanager.com")]))
            .into_layer(),
    );
```

## Headers

`Cuticle::new()` sends:

| Header | Default | Configure with |
|---|---|---|
| `Content-Security-Policy` | `Csp::leptos()`: strict, nonce in `script-src` and `style-src`, `'wasm-unsafe-eval'` | `.csp(…)` |
| `Content-Security-Policy-Report-Only` | not sent | `.csp_report_only(…)` |
| `Permissions-Policy` | `PermissionsPolicy::strict()`: camera, microphone, geolocation, … off | `.permissions_policy(…)` |
| `X-Content-Type-Options` | `nosniff` | `.nosniff(bool)` |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | `.referrer_policy(…)` |
| `X-Frame-Options` | `DENY` | `.x_frame_options(…)` |
| `Cross-Origin-Opener-Policy` | `same-origin` | `.cross_origin_opener_policy(…)` |
| `Strict-Transport-Security` | not sent (turn on in production, over HTTPS) | `.hsts(Hsts::new(max_age).include_subdomains())` |

Every setter takes `None` to remove its header, `.header(name, value)` adds any other one, and `Cuticle::empty()` starts with none. Headers the handler already set are kept, so a route can send its own policy; `.overwrite(true)` replaces them instead.

The policy is typed: `Source::SelfOrigin` renders as `'self'`, `Source::Nonce` as `'nonce-…'` with the request's nonce, `Source::Data` as `data:`, `Source::host("https://www.google.com")` as given. Directives have methods (`default_src`, `script_src`, `style_src`, `img_src`, `connect_src`, `frame_src`, `font_src`, `object_src`, `base_uri`, `form_action`, `frame_ancestors`, `upgrade_insecure_requests`, `report_uri`, `report_to`, …), and `directive(name, sources)` sets any other. Values that would break the header (spaces, `;`, `,`) are refused when the layer is built.

## The nonce

For every request the layer makes a `CspNonce`: 128 bits from the operating system's random number generator ([`getrandom`](https://docs.rs/getrandom)), base64-encoded. It goes into the request's extensions, where the Axum extractor (`CspNonce` or `Option<CspNonce>`) finds it, and into the policy of the response. The header is rendered once when the layer is built; per request only the nonce is written in.

## Leptos

Leptos loads its client code as a WebAssembly module, which a Content-Security-Policy blocks unless `script-src` has `'wasm-unsafe-eval'`. That keyword allows WebAssembly only; JavaScript `eval()` stays blocked. `Csp::leptos()`, the default, includes it.

Leptos 0.8 puts the nonce from its context on its hydration scripts and on `leptos_meta`'s `<Script>` and `<Style>`. With the `leptos` feature, `cuticle::leptos::provide_nonce` makes that the request's nonce; pass it as `leptos_axum`'s `additional_context`:

```rust
let app = Router::new()
    .leptos_routes_with_context(&leptos_options, routes, cuticle::leptos::provide_nonce, {
        let leptos_options = leptos_options.clone();
        move || shell(leptos_options.clone())
    })
    .with_state(leptos_options)
    .layer(cuticle::Cuticle::new().into_layer());
```

In components, `leptos::nonce::use_nonce()` then returns it: `<script nonce=use_nonce()>…</script>`. During development, allow the live-reload WebSocket: `Csp::leptos().extend("connect-src", [Source::host("ws://127.0.0.1:3001")])`.

## Features

- `axum` (default): `CspNonce` is an Axum extractor.
- `leptos`: `cuticle::leptos::provide_nonce` and `From<CspNonce> for leptos::nonce::Nonce`. Needs leptos 0.8.20 or later and Rust 1.88.

## Example

[`examples/rails_policy.rs`](examples/rails_policy.rs) ports the Content-Security-Policy, Permissions-Policy and HSTS of a Rails app; `tests/rails_policy.rs` checks the headers match what Rails sent.

```sh
cargo run --example rails_policy
```

## Licence

MIT or Apache-2.0, at your option.
