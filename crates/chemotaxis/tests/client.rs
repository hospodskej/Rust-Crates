//! The client against a mock openrouteservice answering with recorded JSON.

use std::future::{Future, ready};
use std::time::Duration;

use chemotaxis::{
    Address, Cache, Client, Error, Estimate, LatLon, MemoryCache, PLACE_LAYERS, SuggestKind,
    Suggestion,
};
use serde_json::json;
use wiremock::matchers::{header, headers, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SAWMILL: LatLon = LatLon {
    lat: 48.80958,
    lon: 16.12111,
};
const KEY: &str = "test-key";

const STRUCTURED: &str = include_str!("fixtures/geocode_structured_brno.json");
const SEARCH: &str = include_str!("fixtures/geocode_search_znojmo.json");
const EMPTY: &str = include_str!("fixtures/geocode_empty.json");
const DIRECTIONS: &str = include_str!("fixtures/directions_driving_car.json");
const UNROUTABLE: &str = include_str!("fixtures/directions_unroutable.json");
const QUOTA: &str = include_str!("fixtures/quota_exceeded.json");

fn json_body(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.to_owned(), "application/json")
}

fn route_of(meters: f64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "type": "FeatureCollection",
        "features": [{"type": "Feature", "properties": {"summary": {"distance": meters, "duration": 1.0}}}]
    }))
}

fn client(server: &MockServer) -> Client<MemoryCache> {
    Client::builder(KEY, SAWMILL)
        .base_url(server.uri())
        .countries(["cz", "AT"])
        .cache(MemoryCache::new())
        .build()
        .unwrap()
}

fn brno<'a>() -> Address<'a> {
    Address {
        street: "Hlavní 1",
        city: "Brno",
        zip: "602 00",
        country: "cz",
    }
}

async fn mount_geocode(server: &MockServer, endpoint: &str, answer: ResponseTemplate, times: u64) {
    Mock::given(method("GET"))
        .and(path(endpoint))
        .respond_with(answer)
        .expect(times)
        .mount(server)
        .await;
}

async fn mount_route(server: &MockServer, answer: ResponseTemplate, times: u64) {
    Mock::given(method("GET"))
        .and(path("/v2/directions/driving-car"))
        .respond_with(answer)
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn geocodes_the_address_and_routes_from_the_origin() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/geocode/search/structured"))
        .and(query_param("address", "Hlavní 1"))
        .and(query_param("locality", "Brno"))
        .and(query_param("postalcode", "602 00"))
        .and(query_param("country", "CZ"))
        .and(query_param("size", "1"))
        .and(query_param_is_missing("api_key"))
        .and(header("authorization", KEY))
        .and(headers(
            "accept",
            vec!["application/json", "application/geo+json"],
        ))
        .respond_with(json_body(200, STRUCTURED))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/directions/driving-car"))
        .and(query_param("start", "16.12111,48.80958"))
        .and(query_param("end", "16.6,49.2"))
        .and(header("authorization", KEY))
        .respond_with(json_body(200, DIRECTIONS))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(client(&server).one_way_km(&brno()).await.unwrap(), 23);
}

#[tokio::test]
async fn kilometres_round_half_away_from_zero() {
    for (meters, km) in [(49_620.0, 50), (49_500.0, 50), (49_499.0, 49), (400.0, 0)] {
        let server = MockServer::start().await;
        mount_geocode(
            &server,
            "/geocode/search/structured",
            json_body(200, STRUCTURED),
            1,
        )
        .await;
        mount_route(&server, route_of(meters), 1).await;
        assert_eq!(
            client(&server).one_way_km(&brno()).await.unwrap(),
            km,
            "{meters} m"
        );
    }
}

#[tokio::test]
async fn an_austrian_address_is_looked_up_in_austria_without_an_empty_postcode() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/geocode/search/structured"))
        .and(query_param("country", "AT"))
        .and(query_param("locality", "Retz"))
        .and(query_param_is_missing("postalcode"))
        .respond_with(json_body(200, STRUCTURED))
        .expect(1)
        .mount(&server)
        .await;
    mount_route(&server, json_body(200, DIRECTIONS), 1).await;

    let address = Address {
        street: "Hauptplatz 1",
        city: "Retz",
        zip: " ",
        country: "AT",
    };
    assert_eq!(client(&server).one_way_km(&address).await.unwrap(), 23);
}

#[tokio::test]
async fn the_distance_is_cached_per_address_whatever_the_spaces_and_case() {
    let server = MockServer::start().await;
    mount_geocode(
        &server,
        "/geocode/search/structured",
        json_body(200, STRUCTURED),
        1,
    )
    .await;
    mount_route(&server, json_body(200, DIRECTIONS), 1).await;
    let client = client(&server);

    assert_eq!(client.one_way_km(&brno()).await.unwrap(), 23);
    let respelled = Address {
        street: " hlavní   1 ",
        city: "BRNO",
        ..brno()
    };
    assert_eq!(client.one_way_km(&respelled).await.unwrap(), 23);
    assert_eq!(client.cache().len(), 1);
}

#[tokio::test]
async fn the_cache_keeps_the_value_for_thirty_days() {
    struct Recorder(std::sync::Mutex<Vec<(String, String, Duration)>>);
    impl Cache for Recorder {
        fn get(&self, _key: &str) -> impl Future<Output = Option<String>> + Send {
            ready(None)
        }
        fn put(&self, key: &str, value: String, ttl: Duration) -> impl Future<Output = ()> + Send {
            self.0.lock().unwrap().push((key.to_owned(), value, ttl));
            ready(())
        }
    }

    let server = MockServer::start().await;
    mount_geocode(&server, "/geocode/search", json_body(200, SEARCH), 1).await;
    mount_route(&server, json_body(200, DIRECTIONS), 1).await;
    let client = Client::builder(KEY, SAWMILL)
        .base_url(server.uri())
        .cache(Recorder(std::sync::Mutex::default()))
        .build()
        .unwrap();

    client.estimate("Znojmo", "cz").await.unwrap();
    let stored = client.cache().0.lock().unwrap().clone();
    assert_eq!(
        stored,
        [(
            r#"["chemotaxis","estimate","16.12111,48.80958","cz","znojmo"]"#.to_owned(),
            r#"{"km":23,"place":"Znojmo","zip":"669 02"}"#.to_owned(),
            Duration::from_secs(30 * 24 * 60 * 60),
        )]
    );
}

#[tokio::test]
async fn no_request_without_a_key_an_address_or_a_served_country() {
    let server = MockServer::start().await;
    let client = client(&server);

    let no_street = Address {
        street: " ",
        ..brno()
    };
    assert!(matches!(
        client.one_way_km(&no_street).await,
        Err(Error::InvalidInput(_))
    ));
    let no_city = Address { city: "", ..brno() };
    assert!(matches!(
        client.one_way_km(&no_city).await,
        Err(Error::InvalidInput(_))
    ));
    let slovakia = Address {
        country: "sk",
        ..brno()
    };
    assert!(matches!(
        client.one_way_km(&slovakia).await,
        Err(Error::InvalidInput(_))
    ));
    let nonsense = Address {
        country: "Czechia",
        ..brno()
    };
    assert!(matches!(
        client.one_way_km(&nonsense).await,
        Err(Error::InvalidInput(_))
    ));

    let keyless = Client::builder("  ", SAWMILL)
        .base_url(server.uri())
        .build()
        .unwrap();
    assert!(matches!(
        keyless.one_way_km(&brno()).await,
        Err(Error::MissingApiKey)
    ));
    assert!(matches!(
        keyless.estimate("Znojmo", "cz").await,
        Err(Error::MissingApiKey)
    ));

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_address_not_found_is_not_found_and_not_cached() {
    let server = MockServer::start().await;
    mount_geocode(
        &server,
        "/geocode/search/structured",
        json_body(200, EMPTY),
        2,
    )
    .await;
    mount_route(&server, json_body(200, DIRECTIONS), 0).await;
    let client = client(&server);

    for _ in 0..2 {
        let error = client.one_way_km(&brno()).await.unwrap_err();
        assert!(matches!(error, Error::NotFound));
        assert!(error.is_not_found());
    }
    assert!(client.cache().is_empty());
}

#[tokio::test]
async fn a_failure_is_not_cached() {
    let server = MockServer::start().await;
    let client = client(&server);
    Mock::given(path("/geocode/search/structured"))
        .respond_with(
            ResponseTemplate::new(503).set_body_string("<html>Service Unavailable</html>"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let error = client.one_way_km(&brno()).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::Http {
                status: 503,
                message: None
            }
        ),
        "{error:?}"
    );
    assert!(!error.is_not_found());

    mount_geocode(
        &server,
        "/geocode/search/structured",
        json_body(200, STRUCTURED),
        1,
    )
    .await;
    mount_route(&server, json_body(200, DIRECTIONS), 1).await;
    assert_eq!(client.one_way_km(&brno()).await.unwrap(), 23);
}

#[tokio::test]
async fn an_unroutable_point_carries_the_service_message() {
    let server = MockServer::start().await;
    mount_geocode(
        &server,
        "/geocode/search/structured",
        json_body(200, STRUCTURED),
        1,
    )
    .await;
    mount_route(&server, json_body(404, UNROUTABLE), 1).await;

    let error = client(&server).one_way_km(&brno()).await.unwrap_err();
    let Error::Http {
        status: 404,
        message: Some(message),
    } = &error
    else {
        panic!("{error:?}")
    };
    assert!(message.starts_with("Could not find routable point"));
}

#[tokio::test]
async fn a_route_without_a_distance_is_no_route() {
    let server = MockServer::start().await;
    mount_geocode(
        &server,
        "/geocode/search/structured",
        json_body(200, STRUCTURED),
        1,
    )
    .await;
    let same_point = json!({"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {"summary": {}}}]});
    mount_route(
        &server,
        ResponseTemplate::new(200).set_body_json(same_point),
        1,
    )
    .await;

    let error = client(&server).one_way_km(&brno()).await.unwrap_err();
    assert!(matches!(error, Error::NoRoute));
    assert!(error.is_not_found());
}

#[tokio::test]
async fn over_the_quota_is_a_429() {
    let server = MockServer::start().await;
    mount_geocode(&server, "/geocode/search", json_body(429, QUOTA), 1).await;

    let error = client(&server).estimate("Znojmo", "cz").await.unwrap_err();
    assert!(
        matches!(&error, Error::Http { status: 429, message: Some(m) } if m == "Rate limit exceeded"),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_slow_answer_times_out() {
    let server = MockServer::start().await;
    mount_geocode(
        &server,
        "/geocode/search",
        json_body(200, SEARCH).set_delay(Duration::from_secs(2)),
        1,
    )
    .await;
    let client = Client::builder(KEY, SAWMILL)
        .base_url(server.uri())
        .read_timeout(Duration::from_millis(100))
        .build()
        .unwrap();

    assert!(matches!(
        client.estimate("Znojmo", "cz").await,
        Err(Error::Timeout)
    ));
}

#[tokio::test]
async fn an_unreachable_service_is_a_request_error_without_the_address() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client = Client::builder(KEY, SAWMILL)
        .base_url(format!("http://127.0.0.1:{port}/"))
        .build()
        .unwrap();

    let error = client.one_way_km(&brno()).await.unwrap_err();
    assert!(matches!(error, Error::Request(_)), "{error:?}");
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("Brno") && !shown.contains(KEY), "{shown}");
}

#[tokio::test]
async fn invalid_json_is_a_request_error() {
    let server = MockServer::start().await;
    mount_geocode(&server, "/geocode/search", json_body(200, "{not json"), 1).await;

    assert!(matches!(
        client(&server).estimate("Znojmo", "cz").await,
        Err(Error::Request(_))
    ));
}

#[tokio::test]
async fn estimates_the_distance_to_a_place_in_that_country() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/geocode/search"))
        .and(query_param("text", "Znojmo"))
        .and(query_param("boundary.country", "CZ"))
        .and(query_param("layers", PLACE_LAYERS))
        .and(query_param("size", "1"))
        .and(header("authorization", KEY))
        .respond_with(json_body(200, SEARCH))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/directions/driving-car"))
        .and(query_param("start", "16.12111,48.80958"))
        .and(query_param("end", "16.04882,48.85555"))
        .respond_with(json_body(200, DIRECTIONS))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(
        client(&server).estimate("  Znojmo ", "cz").await.unwrap(),
        Estimate {
            km: 23,
            place: "Znojmo".into(),
            zip: Some("669 02".into())
        }
    );
}

#[tokio::test]
async fn an_estimate_is_cached_per_place_whatever_the_spaces_and_case() {
    let server = MockServer::start().await;
    mount_geocode(&server, "/geocode/search", json_body(200, SEARCH), 1).await;
    mount_route(&server, json_body(200, DIRECTIONS), 1).await;
    let client = client(&server);

    let first = client.estimate("Znojmo", "cz").await.unwrap();
    assert_eq!(client.estimate(" znojmo", "CZ").await.unwrap(), first);
}

#[tokio::test]
async fn no_estimate_without_a_place_outside_the_countries_or_when_not_found() {
    let server = MockServer::start().await;
    let client = client(&server);
    assert!(matches!(
        client.estimate(" ", "cz").await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        client.estimate(&"x".repeat(81), "cz").await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        client.estimate("Bratislava", "sk").await,
        Err(Error::InvalidInput(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());

    mount_geocode(&server, "/geocode/search", json_body(200, EMPTY), 1).await;
    assert!(matches!(
        client.estimate("Neexistuje", "cz").await,
        Err(Error::NotFound)
    ));
    assert!(client.cache().is_empty());
}

#[tokio::test]
async fn an_80_character_query_is_still_looked_up() {
    let server = MockServer::start().await;
    mount_geocode(&server, "/geocode/search", json_body(200, EMPTY), 1).await;
    let query = "č".repeat(80);
    assert!(matches!(
        client(&server).estimate(&query, "cz").await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_place_is_named_by_locality_then_name_then_the_query() {
    let cases = [
        (
            json!({"locality": "Znojmo", "name": "669 02", "postalcode": "669 02"}),
            "Znojmo",
            Some("669 02"),
        ),
        (
            json!({"locality": " ", "name": "Hatě", "postalcode": ""}),
            "Hatě",
            None,
        ),
        (json!({}), "669 02", None),
    ];
    for (properties, name, zip) in cases {
        let server = MockServer::start().await;
        let answer = json!({"type": "FeatureCollection", "features": [{
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [16.04882, 48.85555]},
            "properties": properties
        }]});
        mount_geocode(
            &server,
            "/geocode/search",
            ResponseTemplate::new(200).set_body_json(answer),
            1,
        )
        .await;

        let place = client(&server)
            .geocode_place("669  02", "cz")
            .await
            .unwrap();
        assert_eq!(
            place.point,
            LatLon {
                lat: 48.85555,
                lon: 16.04882
            }
        );
        assert_eq!(place.name, name);
        assert_eq!(place.zip.as_deref(), zip);
    }
}

#[tokio::test]
async fn a_feature_without_a_point_is_not_found() {
    let server = MockServer::start().await;
    let answer = json!({"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {"locality": "Znojmo"}}]});
    mount_geocode(
        &server,
        "/geocode/search",
        ResponseTemplate::new(200).set_body_json(answer),
        1,
    )
    .await;

    assert!(matches!(
        client(&server).estimate("Znojmo", "cz").await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_steps_work_on_their_own() {
    let server = MockServer::start().await;
    mount_geocode(
        &server,
        "/geocode/search/structured",
        json_body(200, STRUCTURED),
        1,
    )
    .await;
    mount_route(&server, json_body(200, DIRECTIONS), 1).await;
    let client = client(&server);

    let point = client.geocode(&brno()).await.unwrap();
    assert_eq!(
        point,
        LatLon {
            lat: 49.2,
            lon: 16.6
        }
    );
    assert!((client.driving_meters(point).await.unwrap() - 23_412.6).abs() < 1e-9);
    assert!(client.cache().is_empty());
}

const NONE: [Suggestion; 0] = [];
const AUTOCOMPLETE: &str = include_str!("fixtures/autocomplete_hlavni.json");

fn autocomplete_of(properties: &[serde_json::Value]) -> ResponseTemplate {
    let features: Vec<_> = properties
        .iter()
        .map(|properties| json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [16.6, 49.2]}, "properties": properties}))
        .collect();
    ResponseTemplate::new(200)
        .set_body_json(json!({"type": "FeatureCollection", "features": features}))
}

#[tokio::test]
async fn suggests_addresses_near_the_origin_with_street_town_and_postcode() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/geocode/autocomplete"))
        .and(query_param("text", "Hlavní"))
        .and(query_param("boundary.country", "CZ"))
        .and(query_param("layers", "address,street"))
        .and(query_param("focus.point.lon", "16.12111"))
        .and(query_param("focus.point.lat", "48.80958"))
        .and(query_param_is_missing("api_key"))
        .and(header("authorization", KEY))
        .respond_with(json_body(200, AUTOCOMPLETE))
        .expect(1)
        .mount(&server)
        .await;

    let found = client(&server)
        .suggest(" Hlavní ", "CZ", SuggestKind::Address)
        .await
        .unwrap();

    // The second "Hlavní, Brno" is a duplicate label and the last feature
    // names no town; both are left out.
    assert_eq!(
        found,
        [
            Suggestion {
                label: "Hlavní 12, Znojmo, Czechia".into(),
                street: Some("Hlavní 12".into()),
                city: "Znojmo".into(),
                zip: Some("669 02".into()),
                country: "cz".into(),
            },
            Suggestion {
                label: "Hlavní, Brno, Czechia".into(),
                street: Some("Hlavní".into()),
                city: "Brno".into(),
                zip: None,
                country: "cz".into(),
            },
            Suggestion {
                label: "Hlavní, Pohořelice, Czechia".into(),
                street: Some("Hlavní".into()),
                city: "Pohořelice".into(),
                zip: None,
                country: "cz".into(),
            },
        ]
    );
    assert_eq!(
        serde_json::to_value(&found[1]).unwrap(),
        json!({"label": "Hlavní, Brno, Czechia", "street": "Hlavní", "city": "Brno", "zip": null, "country": "cz"})
    );
}

#[tokio::test]
async fn town_suggestions_look_for_places_only_and_are_cached_for_a_day() {
    struct Recorder(MemoryCache, std::sync::Mutex<Vec<Duration>>);
    impl Cache for Recorder {
        fn get(&self, key: &str) -> impl Future<Output = Option<String>> + Send {
            self.0.get(key)
        }
        fn put(&self, key: &str, value: String, ttl: Duration) -> impl Future<Output = ()> + Send {
            self.1.lock().unwrap().push(ttl);
            self.0.put(key, value, ttl)
        }
    }

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/geocode/autocomplete"))
        .and(query_param("layers", PLACE_LAYERS))
        .respond_with(autocomplete_of(&[
            json!({"label": "Brno, Czechia", "locality": "Brno"}),
        ]))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::builder(KEY, SAWMILL)
        .base_url(server.uri())
        .cache(Recorder(MemoryCache::new(), std::sync::Mutex::default()))
        .build()
        .unwrap();

    for text in ["Brn", " brn  "] {
        let found = client
            .suggest(text, "cz", SuggestKind::Place)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].city, "Brno");
        assert_eq!(found[0].street, None);
    }
    assert_eq!(
        *client.cache().1.lock().unwrap(),
        [Duration::from_secs(24 * 60 * 60)]
    );
}

#[tokio::test]
async fn address_and_town_suggestions_are_cached_apart() {
    let server = MockServer::start().await;
    mount_geocode(
        &server,
        "/geocode/autocomplete",
        autocomplete_of(&[json!({"label": "Brno", "locality": "Brno"})]),
        2,
    )
    .await;
    let client = client(&server);

    client
        .suggest("Brno", "cz", SuggestKind::Place)
        .await
        .unwrap();
    client
        .suggest("Brno", "cz", SuggestKind::Address)
        .await
        .unwrap();
    client
        .suggest("Brno", "cz", SuggestKind::Address)
        .await
        .unwrap();
    assert_eq!(client.cache().len(), 2);
}

#[tokio::test]
async fn takes_the_first_eight_features() {
    let server = MockServer::start().await;
    let features: Vec<_> = (1..=10)
        .map(|n| json!({"label": format!("Ulice {n}, Brno"), "locality": "Brno"}))
        .collect();
    mount_geocode(
        &server,
        "/geocode/autocomplete",
        autocomplete_of(&features),
        1,
    )
    .await;

    let found = client(&server)
        .suggest("Ulice", "cz", SuggestKind::Address)
        .await
        .unwrap();
    let labels: Vec<_> = found.iter().map(|found| found.label.as_str()).collect();
    assert_eq!(
        labels,
        (1..=8)
            .map(|n| format!("Ulice {n}, Brno"))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn the_city_falls_back_to_localadmin_then_name_and_the_label_to_the_city() {
    let server = MockServer::start().await;
    let features = [
        json!({"localadmin": "Hrušovany", "name": "Nádražní", "street": "Nádražní", "housenumber": " "}),
        json!({"locality": "", "name": "Hatě", "label": " ", "postalcode": "669 02"}),
        json!({"label": "Somewhere"}),
    ];
    mount_geocode(
        &server,
        "/geocode/autocomplete",
        autocomplete_of(&features),
        1,
    )
    .await;

    let found = client(&server)
        .suggest("Nád", "at", SuggestKind::Address)
        .await
        .unwrap();
    assert_eq!(
        found,
        [
            Suggestion {
                label: "Hrušovany".into(),
                street: Some("Nádražní".into()),
                city: "Hrušovany".into(),
                zip: None,
                country: "at".into(),
            },
            Suggestion {
                label: "Hatě".into(),
                street: None,
                city: "Hatě".into(),
                zip: Some("669 02".into()),
                country: "at".into()
            },
        ]
    );
}

#[tokio::test]
async fn no_suggestions_and_no_request_for_short_text_no_key_or_another_country() {
    let server = MockServer::start().await;
    let client = client(&server);

    assert_eq!(
        client
            .suggest("Br", "cz", SuggestKind::Place)
            .await
            .unwrap(),
        NONE
    );
    assert_eq!(
        client
            .suggest("  Br   ", "cz", SuggestKind::Place)
            .await
            .unwrap(),
        NONE
    );
    assert_eq!(
        client
            .suggest(&"x".repeat(81), "cz", SuggestKind::Place)
            .await
            .unwrap(),
        NONE
    );
    assert_eq!(
        client
            .suggest("Bratislava", "sk", SuggestKind::Place)
            .await
            .unwrap(),
        NONE
    );
    assert_eq!(
        client
            .suggest("Brno", "Czechia", SuggestKind::Place)
            .await
            .unwrap(),
        NONE
    );
    let keyless = Client::builder("", SAWMILL)
        .base_url(server.uri())
        .build()
        .unwrap();
    assert_eq!(
        keyless
            .suggest("Brno", "cz", SuggestKind::Place)
            .await
            .unwrap(),
        NONE
    );

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_empty_answer_is_cached_but_a_failure_is_not() {
    let server = MockServer::start().await;
    let client = client(&server);
    Mock::given(path("/geocode/autocomplete"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let error = client
        .suggest("Brno", "cz", SuggestKind::Place)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Http { status: 503, .. }),
        "{error:?}"
    );
    assert!(client.cache().is_empty());

    mount_geocode(&server, "/geocode/autocomplete", json_body(200, EMPTY), 1).await;
    for _ in 0..2 {
        assert_eq!(
            client
                .suggest("Brno", "cz", SuggestKind::Place)
                .await
                .unwrap(),
            NONE
        );
    }
    assert_eq!(client.cache().len(), 1);
}
