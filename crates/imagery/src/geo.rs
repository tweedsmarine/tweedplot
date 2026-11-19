//! Map projections for the images: geographic, web mercator, ellipsoidal mercator and
//! transverse mercator (UTM, NZTM, MGA), all on WGS84 (GRS80 differs from WGS84 by less
//! than a millimetre here, so NZGD2000 and GDA are treated as WGS84).

use std::f64::consts::PI;

const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_223_563;
/// The sphere radius web mercator uses.
pub const R: f64 = 6_378_137.0;

/// A coordinate reference system an image can be in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Crs {
    /// Longitude and latitude in degrees.
    Geographic,
    /// EPSG:3857 (spherical mercator), metres.
    WebMercator,
    /// EPSG:3395 (mercator on the ellipsoid), metres.
    Mercator,
    /// Transverse mercator, metres.
    Tm(Tm),
}

/// Transverse mercator parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tm {
    /// Central meridian, degrees.
    pub lon0: f64,
    pub k0: f64,
    pub false_e: f64,
    pub false_n: f64,
}

impl Tm {
    pub fn utm(zone: u32, south: bool) -> Tm {
        Tm { lon0: zone as f64 * 6.0 - 183.0, k0: 0.9996, false_e: 500_000.0, false_n: if south { 10_000_000.0 } else { 0.0 } }
    }
}

impl Crs {
    /// The CRS of an EPSG code, for the codes images on a chart plotter come in.
    pub fn from_epsg(code: u32) -> Option<Crs> {
        Some(match code {
            4326 | 4167 | 4269 | 4283 | 7844 | 4258 | 4322 => Crs::Geographic,
            3857 | 900913 | 3785 | 102100 => Crs::WebMercator,
            3395 => Crs::Mercator,
            // WGS84 UTM north and south
            32601..=32660 => Crs::Tm(Tm::utm(code - 32600, false)),
            32701..=32760 => Crs::Tm(Tm::utm(code - 32700, true)),
            // NZTM2000
            2193 => Crs::Tm(Tm { lon0: 173.0, k0: 0.9996, false_e: 1_600_000.0, false_n: 10_000_000.0 }),
            // GDA94 and GDA2020 MGA zones (UTM south)
            28348..=28358 => Crs::Tm(Tm::utm(code - 28300, true)),
            7846..=7859 => Crs::Tm(Tm::utm(code - 7800, true)),
            // ETRS89 UTM zones 28-38 N
            25828..=25838 => Crs::Tm(Tm::utm(code - 25800, false)),
            // NAD83 UTM zones 1-23 N
            26901..=26923 => Crs::Tm(Tm::utm(code - 26900, false)),
            _ => return None,
        })
    }

    /// Longitude and latitude (degrees, WGS84) to this CRS.
    pub fn forward(&self, lon: f64, lat: f64) -> (f64, f64) {
        match self {
            Crs::Geographic => (lon, lat),
            Crs::WebMercator => web_mercator(lon, lat),
            Crs::Mercator => {
                let e = (F * (2.0 - F)).sqrt();
                let p = lat.clamp(-89.5, 89.5).to_radians();
                let s = e * p.sin();
                let y = A * ((PI / 4.0 + p / 2.0).tan() * ((1.0 - s) / (1.0 + s)).powf(e / 2.0)).ln();
                (A * lon.to_radians(), y)
            }
            Crs::Tm(t) => tm_forward(t, lon, lat),
        }
    }

    /// This CRS back to longitude and latitude (degrees, WGS84).
    pub fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Crs::Geographic => (x, y),
            Crs::WebMercator => web_mercator_inv(x, y),
            Crs::Mercator => {
                let e = (F * (2.0 - F)).sqrt();
                let t = (-y / A).exp();
                let mut p = PI / 2.0 - 2.0 * t.atan();
                for _ in 0..8 {
                    let s = e * p.sin();
                    p = PI / 2.0 - 2.0 * (t * ((1.0 - s) / (1.0 + s)).powf(e / 2.0)).atan();
                }
                ((x / A).to_degrees(), p.to_degrees())
            }
            Crs::Tm(t) => tm_inverse(t, x, y),
        }
    }
}

/// Longitude and latitude to web mercator metres.
pub fn web_mercator(lon: f64, lat: f64) -> (f64, f64) {
    let p = lat.clamp(-85.06, 85.06).to_radians();
    (R * lon.to_radians(), R * (PI / 4.0 + p / 2.0).tan().ln())
}

/// Web mercator metres to longitude and latitude.
pub fn web_mercator_inv(x: f64, y: f64) -> (f64, f64) {
    ((x / R).to_degrees(), (2.0 * (y / R).exp().atan() - PI / 2.0).to_degrees())
}

/// Transverse mercator by Krüger's series (Karney 2011), to the third order in n: well under
/// a millimetre within a UTM zone and still centimetres several zones out.
pub fn tm_forward(t: &Tm, lon: f64, lat: f64) -> (f64, f64) {
    let n = F / (2.0 - F);
    let (n2, n3) = (n * n, n * n * n);
    let big_a = A / (1.0 + n) * (1.0 + n2 / 4.0 + n2 * n2 / 64.0);
    let alpha = [n / 2.0 - 2.0 / 3.0 * n2 + 5.0 / 16.0 * n3, 13.0 / 48.0 * n2 - 3.0 / 5.0 * n3, 61.0 / 240.0 * n3];
    let phi = lat.to_radians();
    let mut dl = lon - t.lon0;
    while dl > 180.0 {
        dl -= 360.0;
    }
    while dl < -180.0 {
        dl += 360.0;
    }
    let lam = dl.to_radians();
    let c = 2.0 * n.sqrt() / (1.0 + n);
    let tt = (phi.sin().atanh() - c * (c * phi.sin()).atanh()).sinh();
    let xi_p = tt.atan2(lam.cos());
    let eta_p = (lam.sin() / (1.0 + tt * tt).sqrt()).atanh();
    let mut xi = xi_p;
    let mut eta = eta_p;
    for (j, a) in alpha.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi += a * (k * xi_p).sin() * (k * eta_p).cosh();
        eta += a * (k * xi_p).cos() * (k * eta_p).sinh();
    }
    (t.false_e + t.k0 * big_a * eta, t.false_n + t.k0 * big_a * xi)
}

/// Transverse mercator back to longitude and latitude (Krüger's series, third order).
pub fn tm_inverse(t: &Tm, x: f64, y: f64) -> (f64, f64) {
    let n = F / (2.0 - F);
    let (n2, n3) = (n * n, n * n * n);
    let big_a = A / (1.0 + n) * (1.0 + n2 / 4.0 + n2 * n2 / 64.0);
    let beta = [n / 2.0 - 2.0 / 3.0 * n2 + 37.0 / 96.0 * n3, n2 / 48.0 + n3 / 15.0, 17.0 / 480.0 * n3];
    let delta = [2.0 * n - 2.0 / 3.0 * n2 - 2.0 * n3, 7.0 / 3.0 * n2 - 8.0 / 5.0 * n3, 56.0 / 15.0 * n3];
    let xi = (y - t.false_n) / (t.k0 * big_a);
    let eta = (x - t.false_e) / (t.k0 * big_a);
    let mut xi_p = xi;
    let mut eta_p = eta;
    for (j, b) in beta.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi_p -= b * (k * xi).sin() * (k * eta).cosh();
        eta_p -= b * (k * xi).cos() * (k * eta).sinh();
    }
    let chi = (xi_p.sin() / eta_p.cosh()).asin();
    let lam = eta_p.sinh().atan2(xi_p.cos());
    let mut phi = chi;
    for (j, d) in delta.iter().enumerate() {
        phi += d * (2.0 * (j as f64 + 1.0) * chi).sin();
    }
    let mut lon = t.lon0 + lam.to_degrees();
    if lon > 180.0 {
        lon -= 360.0;
    } else if lon < -180.0 {
        lon += 360.0;
    }
    (lon, phi.to_degrees())
}

/// Great-circle distance in metres.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (p1, p2) = (a.1.to_radians(), b.1.to_radians());
    let dp = p2 - p1;
    let dl = (b.0 - a.0).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().asin()
}

/// An affine map from image pixels (column, row; 0,0 the top-left corner of the first
/// pixel) to CRS coordinates: x = a + b·col + c·row, y = d + e·col + f·row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub fn apply(&self, col: f64, row: f64) -> (f64, f64) {
        (self.a + self.b * col + self.c * row, self.d + self.e * col + self.f * row)
    }

    /// CRS coordinates back to pixels (None for a degenerate transform).
    pub fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let det = self.b * self.f - self.c * self.e;
        if det.abs() < 1e-300 {
            return None;
        }
        let (dx, dy) = (x - self.a, y - self.d);
        Some(((self.f * dx - self.c * dy) / det, (-self.e * dx + self.b * dy) / det))
    }
}

/// Least squares: solve the normal equations for `cols` unknowns (Gaussian elimination with
/// partial pivoting). None when the points don't determine them.
pub fn least_squares(rows: &[Vec<f64>], rhs: &[f64]) -> Option<Vec<f64>> {
    let n = rows.first()?.len();
    let mut m = vec![vec![0.0; n + 1]; n];
    for (r, &y) in rows.iter().zip(rhs) {
        for i in 0..n {
            for j in 0..n {
                m[i][j] += r[i] * r[j];
            }
            m[i][n] += r[i] * y;
        }
    }
    for col in 0..n {
        let piv = (col..n).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[piv][col].abs() < 1e-12 * (1.0 + m[col][col].abs()) {
            return None;
        }
        m.swap(col, piv);
        for r in 0..n {
            if r != col {
                let k = m[r][col] / m[col][col];
                if k != 0.0 {
                    let pivot = m[col].clone();
                    for (v, p) in m[r][col..].iter_mut().zip(&pivot[col..]) {
                        *v -= k * p;
                    }
                }
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}

/// Metres per degree of latitude (roughly), for turning ground resolution into zoom levels.
pub const M_PER_DEG: f64 = 111_320.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utm_matches_known_points() {
        // Nelson, NZ: 173.2840 E, 41.2706 S → UTM 59S (pyproj): 691 306.82 E, 5 428 687.19 N
        let (e, n) = Crs::from_epsg(32759).unwrap().forward(173.2840, -41.2706);
        assert!((e - 691_306.82).abs() < 0.05, "{e}");
        assert!((n - 5_428_687.19).abs() < 0.05, "{n}");
        // NZTM2000 for the same point (pyproj): 1 623 786.90 E, 5 431 164.18 N
        let (e, n) = Crs::from_epsg(2193).unwrap().forward(173.2840, -41.2706);
        assert!((e - 1_623_786.90).abs() < 0.05, "{e}");
        assert!((n - 5_431_164.18).abs() < 0.05, "{n}");
    }

    #[test]
    fn inverses_round_trip() {
        for code in [32759, 2193, 32633, 3395, 3857, 4326] {
            let crs = Crs::from_epsg(code).unwrap();
            for (lon, lat) in [(173.28, -41.27), (171.5, -45.9), (174.9, -36.8)] {
                let (lon, lat) = if code == 32633 { (lon - 160.0, -lat) } else { (lon, lat) };
                let (x, y) = crs.forward(lon, lat);
                let (lo, la) = crs.inverse(x, y);
                assert!((lo - lon).abs() < 1e-8 && (la - lat).abs() < 1e-8, "{code}: {lo},{la} vs {lon},{lat}");
            }
        }
    }

    #[test]
    fn affine_round_trip() {
        let t = Affine { a: 100.0, b: 10.0, c: 0.5, d: 900.0, e: -0.25, f: -10.0 };
        let (x, y) = t.apply(12.0, 34.0);
        let (c, r) = t.invert(x, y).unwrap();
        assert!((c - 12.0).abs() < 1e-9 && (r - 34.0).abs() < 1e-9);
    }

    #[test]
    fn web_mercator_round_trip() {
        let (x, y) = web_mercator(173.25, -41.27);
        let (lon, lat) = web_mercator_inv(x, y);
        assert!((lon - 173.25).abs() < 1e-9 && (lat + 41.27).abs() < 1e-9);
    }
}
