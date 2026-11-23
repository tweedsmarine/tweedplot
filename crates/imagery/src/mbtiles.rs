//! MBTiles: a SQLite file of ready-made web-mercator map tiles (PNG, JPEG or WebP), as NOAA's
//! Chart Display Service, OpenSeaMap and cruisers' satellite collections publish them. The
//! tiles are passed to the map as they are; above the file's deepest zoom the map enlarges them.
//! Vector-tile MBTiles (pbf) are not drawn.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::{err, Error, Result};

pub struct MbTiles {
    conn: Mutex<Connection>,
    pub name: String,
    pub description: String,
    pub attribution: String,
    /// "png", "jpg" or "webp".
    pub format: String,
    /// [west, south, east, north]
    pub bounds: [f64; 4],
    pub min_zoom: u8,
    pub max_zoom: u8,
}

fn e(x: rusqlite::Error) -> Error {
    Error(format!("MBTiles: {x}"))
}

/// Whether the file starts like a SQLite database.
pub fn is_sqlite(head: &[u8]) -> bool {
    head.starts_with(b"SQLite format 3\0")
}

impl MbTiles {
    pub fn open(path: &Path) -> Result<MbTiles> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(e)?;
        let meta = |k: &str| -> Option<String> {
            conn.query_row("SELECT value FROM metadata WHERE name = ?1", [k], |r| r.get::<_, String>(0)).optional().ok().flatten()
        };
        let has_tiles: bool =
            conn.query_row("SELECT count(*) FROM sqlite_master WHERE name = 'tiles' AND type IN ('table', 'view')", [], |r| r.get::<_, i64>(0)).map_err(e)? > 0;
        if !has_tiles {
            return err("this SQLite file has no map tiles (it is not an MBTiles file)");
        }
        // the format: from the metadata, else from the first tile's bytes
        let first: Option<Vec<u8>> = conn.query_row("SELECT tile_data FROM tiles LIMIT 1", [], |r| r.get(0)).optional().map_err(e)?;
        let Some(first) = first else { return err("this MBTiles file has no tiles in it") };
        let sniffed = if first.starts_with(&[0x89, b'P', b'N', b'G']) {
            "png"
        } else if first.starts_with(&[0xff, 0xd8]) {
            "jpg"
        } else if first.len() > 12 && &first[0..4] == b"RIFF" && &first[8..12] == b"WEBP" {
            "webp"
        } else {
            "pbf"
        };
        let format = meta("format").map(|f| f.to_ascii_lowercase()).filter(|f| !f.is_empty()).unwrap_or_else(|| sniffed.to_string());
        let format = if format == "jpeg" { "jpg".to_string() } else { format };
        if format == "pbf" || format == "mvt" || sniffed == "pbf" {
            return err("this MBTiles file holds vector tiles, which TweedPlot doesn't draw yet: only picture tiles (PNG, JPEG, WebP)");
        }
        let (zmin, zmax): (Option<i64>, Option<i64>) =
            conn.query_row("SELECT min(zoom_level), max(zoom_level) FROM tiles", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?;
        let min_zoom = meta("minzoom").and_then(|v| v.trim().parse::<f64>().ok()).map(|v| v as i64).or(zmin).unwrap_or(0).clamp(0, 24) as u8;
        let max_zoom = meta("maxzoom").and_then(|v| v.trim().parse::<f64>().ok()).map(|v| v as i64).or(zmax).unwrap_or(18).clamp(0, 24) as u8;
        let bounds = match meta("bounds").map(|b| b.split(',').filter_map(|v| v.trim().parse::<f64>().ok()).collect::<Vec<_>>()) {
            Some(v) if v.len() == 4 && v[0] < v[2] && v[1] < v[3] => [v[0].max(-180.0), v[1].max(-85.06), v[2].min(180.0), v[3].min(85.06)],
            _ => bounds_from_tiles(&conn, zmax.unwrap_or(max_zoom as i64) as u8)?,
        };
        let name = meta("name").unwrap_or_default();
        let description = meta("description").unwrap_or_default();
        let attribution = meta("attribution").map(|a| strip_tags(&a)).unwrap_or_default();
        Ok(MbTiles { conn: Mutex::new(conn), name, description, attribution, format, bounds, min_zoom: min_zoom.min(max_zoom), max_zoom })
    }

    /// The tile's bytes as stored (the map's y counts down from the north; MBTiles' up from the south).
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
        if z > 24 || x >= (1u32 << z) || y >= (1u32 << z) {
            return None;
        }
        let row = (1u32 << z) - 1 - y;
        let c = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        c.query_row("SELECT tile_data FROM tiles WHERE zoom_level = ?1 AND tile_column = ?2 AND tile_row = ?3", [z as u32, x, row], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn mime(&self) -> &'static str {
        match self.format.as_str() {
            "jpg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        }
    }

    /// Ground metres per pixel at the deepest zoom, at the middle of the area.
    pub fn resolution_m(&self) -> f64 {
        let lat = ((self.bounds[1] + self.bounds[3]) / 2.0).to_radians();
        2.0 * std::f64::consts::PI * crate::geo::R * lat.cos() / (256.0 * (1u64 << self.max_zoom) as f64)
    }
}

/// The area the deepest zoom's tiles cover, when the metadata doesn't say.
fn bounds_from_tiles(conn: &Connection, z: u8) -> Result<[f64; 4]> {
    let (x0, x1, r0, r1): (i64, i64, i64, i64) = conn
        .query_row("SELECT min(tile_column), max(tile_column), min(tile_row), max(tile_row) FROM tiles WHERE zoom_level = ?1", [z as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(e)?;
    let n = (1i64 << z) as f64;
    let lon = |x: f64| x / n * 360.0 - 180.0;
    let lat = |y: f64| (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    // MBTiles rows count up from the south: the map's y = n − 1 − row
    let (ytop, ybot) = (n - 1.0 - r1 as f64, n - 1.0 - r0 as f64 + 1.0);
    Ok([lon(x0 as f64), lat(ybot), lon(x1 as f64 + 1.0), lat(ytop)])
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&copy;", "©").replace("&amp;", "&").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(path: &Path, format: Option<&str>, tile: &[u8]) {
        let c = Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE metadata (name TEXT, value TEXT); CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB);",
        )
        .unwrap();
        if let Some(f) = format {
            c.execute("INSERT INTO metadata VALUES ('format', ?1)", [f]).unwrap();
        }
        c.execute("INSERT INTO metadata VALUES ('name', 'Test harbour')", []).unwrap();
        c.execute("INSERT INTO metadata VALUES ('attribution', '<a href=\"x\">&copy; Someone</a>')", []).unwrap();
        // zoom 10, the tile over Nelson: x 1004, y (map) 641 → row 1023 − 641 = 382
        c.execute("INSERT INTO tiles VALUES (10, 1004, 382, ?1)", [tile]).unwrap();
        c.execute("INSERT INTO tiles VALUES (9, 502, 191, ?1)", [tile]).unwrap();
    }

    #[test]
    fn reads_tiles_and_metadata() {
        let d = std::env::temp_dir().join(format!("tp-mbt-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("t.mbtiles");
        let _ = std::fs::remove_file(&p);
        make(&p, None, &[0x89, b'P', b'N', b'G', 1, 2, 3]);
        let m = MbTiles::open(&p).unwrap();
        assert_eq!(m.format, "png");
        assert_eq!((m.min_zoom, m.max_zoom), (9, 10));
        assert_eq!(m.name, "Test harbour");
        assert_eq!(m.attribution, "© Someone");
        assert!(m.tile(10, 1004, 641).is_some());
        assert!(m.tile(10, 1004, 382).is_none());
        // the area from the tiles: around 173°E, 41°S
        assert!(m.bounds[0] > 172.0 && m.bounds[2] < 174.0 && m.bounds[1] > -42.0 && m.bounds[3] < -40.0, "{:?}", m.bounds);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn refuses_vector_tiles() {
        let d = std::env::temp_dir().join(format!("tp-mbv-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("v.mbtiles");
        let _ = std::fs::remove_file(&p);
        make(&p, Some("pbf"), &[0x1f, 0x8b, 0, 0]);
        let r = MbTiles::open(&p);
        assert!(r.is_err_and(|e| e.0.contains("vector")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
