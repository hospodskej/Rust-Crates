//! Coordinates with named latitude and longitude.

use std::fmt;

/// A point on the map, in WGS 84 degrees.
///
/// openrouteservice writes points as `longitude,latitude`, the opposite of
/// how most people (and maps) say them. The fields are named, so a point is
/// always built as `LatLon { lat, lon }` and the order on the wire is this
/// crate's business, never the caller's.
///
/// ```
/// use chemotaxis::LatLon;
///
/// let sawmill = LatLon { lat: 48.80958, lon: 16.12111 };
/// assert_eq!(sawmill.to_string(), "48.80958, 16.12111");
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    /// Latitude, north positive (-90 to 90).
    pub lat: f64,
    /// Longitude, east positive (-180 to 180).
    pub lon: f64,
}

impl LatLon {
    /// The point as openrouteservice wants it in a URL: `lon,lat`.
    pub(crate) fn to_ors(self) -> String {
        format!("{},{}", self.lon, self.lat)
    }

    /// Reads a `GeoJSON` position (`[lon, lat]`), which must have exactly two
    /// numbers, as the original service checked.
    pub(crate) fn from_geojson(position: &[f64]) -> Option<Self> {
        match position {
            [lon, lat] => Some(Self {
                lat: *lat,
                lon: *lon,
            }),
            _ => None,
        }
    }

    pub(crate) fn is_valid(self) -> bool {
        (-90.0..=90.0).contains(&self.lat) && (-180.0..=180.0).contains(&self.lon)
    }
}

/// Written as `lat, lon`, the order people read.
impl fmt::Display for LatLon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.lat, self.lon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ors_order_is_lon_lat() {
        let point = LatLon {
            lat: 48.80958,
            lon: 16.12111,
        };
        assert_eq!(point.to_ors(), "16.12111,48.80958");
    }

    #[test]
    fn geojson_position_is_lon_lat() {
        assert_eq!(
            LatLon::from_geojson(&[16.6, 49.2]),
            Some(LatLon {
                lat: 49.2,
                lon: 16.6
            })
        );
        assert_eq!(LatLon::from_geojson(&[16.6]), None);
        assert_eq!(LatLon::from_geojson(&[16.6, 49.2, 200.0]), None);
    }

    #[test]
    fn range_check() {
        assert!(
            LatLon {
                lat: 48.8,
                lon: 16.1
            }
            .is_valid()
        );
        assert!(
            !LatLon {
                lat: 116.1,
                lon: 48.8
            }
            .is_valid()
        );
        assert!(
            !LatLon {
                lat: f64::NAN,
                lon: 0.0
            }
            .is_valid()
        );
    }
}
