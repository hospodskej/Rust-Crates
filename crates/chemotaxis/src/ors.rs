//! The parts of openrouteservice's answers that the client reads. Every
//! other field is ignored.

use serde::Deserialize;

use crate::LatLon;

/// A `GeoJSON` `FeatureCollection`, as both geocoding and directions answer.
#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "P: Deserialize<'de>"))]
pub(crate) struct FeatureCollection<P> {
    #[serde(default = "Vec::new")]
    pub features: Vec<Feature<P>>,
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "P: Deserialize<'de>"))]
pub(crate) struct Feature<P> {
    #[serde(default)]
    pub geometry: Option<Geometry>,
    #[serde(default)]
    pub properties: Option<P>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Geometry {
    /// Kept loose: only a `[lon, lat]` pair counts as a found point.
    #[serde(default)]
    pub coordinates: serde_json::Value,
}

impl Geometry {
    pub fn point(&self) -> Option<LatLon> {
        let position = self.coordinates.as_array()?;
        let numbers: Option<Vec<f64>> = position.iter().map(serde_json::Value::as_f64).collect();
        LatLon::from_geojson(&numbers?)
    }
}

/// Properties of a geocoding result (Pelias).
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Place {
    #[serde(default)]
    pub locality: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub postalcode: Option<String>,
}

/// Properties of a directions result.
#[derive(Debug, Deserialize)]
pub(crate) struct Route {
    #[serde(default)]
    pub summary: Option<Summary>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Summary {
    /// Metres. Missing when start and end are the same point.
    #[serde(default)]
    pub distance: Option<f64>,
}

/// The message from an error answer, from whichever shape it has:
/// `{"error": {"message": ...}}` (directions), `{"error": "..."}` (the API
/// gateway) or `{"geocoding": {"errors": ["..."]}}` (geocoding).
pub(crate) fn error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let message = value
        .pointer("/error/message")
        .or_else(|| value.get("error"))
        .or_else(|| value.pointer("/geocoding/errors/0"))?
        .as_str()?
        .trim();
    if message.is_empty() {
        return None;
    }
    Some(message.chars().take(300).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages() {
        assert_eq!(
            error_message(r#"{"error":{"code":2010,"message":"Could not find routable point"}}"#)
                .as_deref(),
            Some("Could not find routable point")
        );
        assert_eq!(
            error_message(r#"{"error":"Access to this API has been disallowed"}"#).as_deref(),
            Some("Access to this API has been disallowed")
        );
        assert_eq!(
            error_message(r#"{"geocoding":{"errors":["invalid param 'boundary.country'"]}}"#)
                .as_deref(),
            Some("invalid param 'boundary.country'")
        );
        assert_eq!(error_message("<html>Bad Gateway</html>"), None);
        assert_eq!(error_message(r#"{"error":""}"#), None);
    }

    #[test]
    fn only_a_pair_is_a_point() {
        let geometry = |json: &str| Geometry {
            coordinates: serde_json::from_str(json).unwrap(),
        };
        assert_eq!(
            geometry("[16.6, 49.2]").point(),
            Some(LatLon {
                lat: 49.2,
                lon: 16.6
            })
        );
        assert_eq!(geometry("[[16.6, 49.2]]").point(), None);
        assert_eq!(geometry("null").point(), None);
    }
}
