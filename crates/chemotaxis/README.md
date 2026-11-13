# chemotaxis

A typed async client for [openrouteservice](https://openrouteservice.org) that finds the road distance to a place: it geocodes an address or a town and routes to it by car from a fixed origin, such as a shop or a sawmill. Made for delivery prices.

Chemotaxis is an organism moving towards a goal along a gradient.

```rust
use chemotaxis::{Address, Client, LatLon, MemoryCache};

async fn run(api_key: String) -> Result<(), chemotaxis::Error> {
    let sawmill = LatLon { lat: 48.80958, lon: 16.12111 };
    let client = Client::builder(api_key, sawmill)
        .countries(["cz", "at"])
        .cache(MemoryCache::new())
        .build()?;

    // Before checkout: a town or postcode, places only, in that country.
    let estimate = client.estimate("Znojmo", "cz").await?;
    assert_eq!(estimate.place, "Znojmo"); // and estimate.zip, estimate.km

    // At checkout: the full address.
    let address = Address { street: "Hlavní 1", city: "Brno", zip: "602 00", country: "cz" };
    println!("{} km one way", client.one_way_km(&address).await?);
    Ok(())
}
```

## What it does

- **Address** (`one_way_km`): structured geocoding of street, town, postcode and country, then the driving distance from the origin in whole kilometres.
- **Place** (`estimate`): a town name or postcode (PSČ), searched only among towns, parts of towns and postcodes in the given country, so "Znojmo" doesn't find a street of that name. Returns the place as found, its postcode and the kilometres.
- **Kilometres** are the route's metres / 1000, rounded half away from zero (49.5 km is 50).
- **Caching:** found distances are kept for 30 days per address or place, ignoring spaces and case. "Not found" and failures are not kept. Bring your own store by implementing the small async `Cache` trait (e.g. on SQLite), or use `MemoryCache` or `NoCache` (the default).
- **Errors** are an `Error` enum: `MissingApiKey`, `InvalidInput`, `NotFound`, `NoRoute`, `Http` (with the service's message), `Timeout`, `Request`. `is_not_found()` separates "no route there" from "the service failed"; a shop typically shows "delivery price calculated individually" for both.
- **Timeouts:** 4 seconds to connect and 4 seconds per read, configurable.
- **Base URL** is configurable, for tests against a mock server or a self-hosted openrouteservice.

`LatLon { lat, lon }` has named fields, so latitude and longitude can't be swapped; the client writes `lon,lat` for openrouteservice itself.

## API key and quota

The key comes from you; the crate reads no environment variables. It is sent in the `Authorization` header, never in a URL, and `Debug` on the client prints `[redacted]` in its place. Keep it out of your logs too. Errors carry no URLs, so logging them doesn't log the customer's address.

The free plan allows 1,000 geocoding and 2,000 route requests a day. An uncached lookup costs one of each; the crate doesn't count them.

## Licence

Dual-licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
