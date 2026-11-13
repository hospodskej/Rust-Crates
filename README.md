# Rust Crates

Small, focused crates for Rust web development, each in its own folder under `crates/`.
They are named after parts of living things and electronics.

| Crate | What it does |
|---|---|
| [refractory](crates/refractory) | Rate limiting: a check API for handlers (redirect with a message, a 429, ...) and an optional Tower/Axum layer that answers 429 with `Retry-After`, per client IP behind a trusted proxy or any other key |

## Licence

Every crate is dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
