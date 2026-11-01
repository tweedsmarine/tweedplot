//! Core navigation types: positions, angles and earth geometry.
//!
//! Conventions used across the whole project:
//! - Latitude/longitude in decimal degrees, WGS84, south and west negative.
//! - Bearings in degrees true, 0..360.
//! - Distances in nautical miles, speeds in knots.
//! - The earth is a sphere of radius 3440.065 NM (good to ~0.5 % for routing;
//!   use an ellipsoid later where it matters, e.g. start lines).

pub const EARTH_RADIUS_NM: f64 = 3440.065;

/// A position on the earth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon: wrap_lon(lon) }
    }

    /// Great-circle distance in NM (haversine).
    pub fn distance_to(&self, other: &LatLon) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), other.lat.to_radians());
        let dp = p2 - p1;
        let dl = (other.lon - self.lon).to_radians();
        let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_NM * a.sqrt().atan2((1.0 - a).sqrt())
    }

    /// Initial great-circle bearing in degrees true.
    pub fn bearing_to(&self, other: &LatLon) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), other.lat.to_radians());
        let dl = (other.lon - self.lon).to_radians();
        let y = dl.sin() * p2.cos();
        let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
        norm360(y.atan2(x).to_degrees())
    }

    /// Point reached after `dist_nm` along great-circle initial bearing `brg`.
    pub fn destination(&self, brg: f64, dist_nm: f64) -> LatLon {
        let d = dist_nm / EARTH_RADIUS_NM;
        let t = brg.to_radians();
        let p1 = self.lat.to_radians();
        let l1 = self.lon.to_radians();
        let p2 = (p1.sin() * d.cos() + p1.cos() * d.sin() * t.cos()).asin();
        let l2 = l1 + (t.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * p2.sin());
        LatLon::new(p2.to_degrees(), l2.to_degrees())
    }

    /// Rhumb-line distance (NM) and bearing (deg true).
    pub fn rhumb_to(&self, other: &LatLon) -> (f64, f64) {
        let p1 = self.lat.to_radians();
        let p2 = other.lat.to_radians();
        let dp = p2 - p1;
        let mut dl = (other.lon - self.lon).to_radians();
        if dl.abs() > std::f64::consts::PI {
            dl -= dl.signum() * 2.0 * std::f64::consts::PI;
        }
        let dpsi = ((std::f64::consts::FRAC_PI_4 + p2 / 2.0).tan() / (std::f64::consts::FRAC_PI_4 + p1 / 2.0).tan()).ln();
        let q = if dpsi.abs() > 1e-12 { dp / dpsi } else { p1.cos() };
        let dist = (dp * dp + q * q * dl * dl).sqrt() * EARTH_RADIUS_NM;
        let brg = norm360(dl.atan2(dpsi).to_degrees());
        (dist, brg)
    }
}

/// Normalise an angle to 0..360.
pub fn norm360(a: f64) -> f64 {
    let r = a % 360.0;
    if r < 0.0 {
        r + 360.0
    } else {
        r
    }
}

/// Normalise an angle to -180..180.
pub fn norm180(a: f64) -> f64 {
    let r = norm360(a);
    if r > 180.0 {
        r - 360.0
    } else {
        r
    }
}

/// Wrap longitude into -180..180.
pub fn wrap_lon(lon: f64) -> f64 {
    norm180(lon)
}

/// True wind angle (0..180) for a heading and a wind-from direction.
pub fn twa(heading: f64, twd: f64) -> f64 {
    norm180(twd - heading).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn nelson_to_wellington() {
        // Nelson Haven entrance to Wellington Harbour entrance, roughly 75 NM.
        let nelson = LatLon::new(-41.257, 173.268);
        let wgtn = LatLon::new(-41.36, 174.83);
        let d = nelson.distance_to(&wgtn);
        assert!(close(d, 70.8, 1.0), "distance {d}");
        let b = nelson.bearing_to(&wgtn);
        assert!(b > 90.0 && b < 100.0, "bearing {b}");
    }

    #[test]
    fn destination_round_trip() {
        let a = LatLon::new(-36.84, 174.80);
        let b = a.destination(347.0, 100.0);
        assert!(close(a.distance_to(&b), 100.0, 1e-6));
        assert!(close(a.bearing_to(&b), 347.0, 1e-6));
    }

    #[test]
    fn rhumb_due_east_on_equator() {
        let a = LatLon::new(0.0, 0.0);
        let b = LatLon::new(0.0, 1.0);
        let (d, brg) = a.rhumb_to(&b);
        assert!(close(d, 60.04, 0.05));
        assert!(close(brg, 90.0, 1e-9));
    }

    #[test]
    fn dateline_wrap() {
        let a = LatLon::new(-40.0, 179.5);
        let b = a.destination(90.0, 60.0);
        assert!(b.lon < -179.0, "lon {}", b.lon);
    }

    #[test]
    fn true_wind_angle() {
        assert_eq!(twa(0.0, 45.0), 45.0);
        assert_eq!(twa(350.0, 10.0), 20.0);
        assert_eq!(twa(90.0, 270.0), 180.0);
    }
}
