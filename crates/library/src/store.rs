//! The library's database: one SQLite file, `library.gpkg`.
//!
//! It is a GeoPackage (OGC), so GIS software (QGIS and others) opens it and shows the
//! marks; TweedPlot's own tables sit beside the GeoPackage ones. WAL mode: a crash or
//! power cut never leaves it half written.
//!
//! Tables (all ids are text, as before):
//! - `marks`: a GeoPackage feature table (point `geom`), plus lat/lon and the mark's fields.
//! - `routes`, `route_marks` (the route's marks in order), `routings` (summary columns
//!   plus the request and result as JSON), `barriers` (points as JSON).
//! - `tracks`: one row per track with its totals (what lists show), and `track_data`:
//!   its fixes, delta-coded and compressed ([`encode_segments`]).
//! - `imports`: each import (file, when); objects remember theirs, so an import can be undone.
//! - `trips`: trips (later: set at the start of a trip, everything made in it goes into it).
//! - `meta`: key/value (schema version, the JSON conversion).
//!
//! Deleting sets `deleted_at` (the Trash; purged after 30 days); every list leaves those out.

use rusqlite::{params, Connection, OptionalExtension};

use crate::tracks::TrackPoint;
use crate::Result;

pub const FILE: &str = "library.gpkg";
pub const SCHEMA_VERSION: i64 = 8;
/// Days a deleted object stays in the Trash.
pub const TRASH_DAYS: i64 = 30;

pub fn open(path: &std::path::Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(std::time::Duration::from_secs(30))?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    create(&c)?;
    migrate(&c)?;
    Ok(c)
}

/// Bring an older database up to [`SCHEMA_VERSION`].
fn migrate(c: &Connection) -> Result<()> {
    let v: i64 = meta(c, "schema")?.and_then(|s| s.parse().ok()).unwrap_or(1);
    if v < 2 {
        // marks: the conditions when they were made (from the track, or the name) and tags;
        // a spatial index (what's in the chart view); TimeZero's symbols and what they mean
        c.execute_batch(
            r#"
            BEGIN;
            ALTER TABLE marks ADD COLUMN depth REAL;
            ALTER TABLE marks ADD COLUMN temp REAL;
            ALTER TABLE marks ADD COLUMN sog REAL;
            ALTER TABLE marks ADD COLUMN tags TEXT NOT NULL DEFAULT '';
            CREATE VIRTUAL TABLE marks_rtree USING rtree(fid, minx, maxx, miny, maxy);
            INSERT INTO marks_rtree SELECT fid, lon, lon, lat, lat FROM marks;
            CREATE TRIGGER marks_rtree_ins AFTER INSERT ON marks BEGIN
                INSERT INTO marks_rtree VALUES (new.fid, new.lon, new.lon, new.lat, new.lat); END;
            CREATE TRIGGER marks_rtree_upd AFTER UPDATE OF lat, lon ON marks BEGIN
                UPDATE marks_rtree SET minx = new.lon, maxx = new.lon, miny = new.lat, maxy = new.lat WHERE fid = new.fid; END;
            CREATE TRIGGER marks_rtree_del AFTER DELETE ON marks BEGIN
                DELETE FROM marks_rtree WHERE fid = old.fid; END;
            CREATE TABLE tz_symbols (symbol TEXT PRIMARY KEY, kind TEXT NOT NULL DEFAULT '', label TEXT NOT NULL DEFAULT '');
            UPDATE meta SET value = '2' WHERE key = 'schema';
            COMMIT;
            "#,
        )?;
    }
    if v < 3 {
        // trips: who was aboard
        c.execute_batch("BEGIN; ALTER TABLE trips ADD COLUMN crew TEXT NOT NULL DEFAULT ''; UPDATE meta SET value = '3' WHERE key = 'schema'; COMMIT;")?;
    }
    if v < 4 {
        // the fishing log (events.rs)
        c.execute_batch(
            "BEGIN;
             CREATE TABLE events (id TEXT PRIMARY KEY, kind TEXT NOT NULL DEFAULT 'note', name TEXT NOT NULL DEFAULT '', at INTEGER NOT NULL,
               lat REAL, lon REAL, depth REAL, temp REAL, sog REAL, cog REAL, notes TEXT NOT NULL DEFAULT '', pair TEXT NOT NULL DEFAULT '',
               import_id INTEGER, deleted_at INTEGER);
             CREATE INDEX events_at ON events(at);
             UPDATE meta SET value = '4' WHERE key = 'schema';
             COMMIT;",
        )?;
    }
    if v < 5 {
        // barriers can come from an import (TimeZero's lines, areas and circles)
        c.execute_batch("BEGIN; ALTER TABLE barriers ADD COLUMN import_id INTEGER; UPDATE meta SET value = '5' WHERE key = 'schema'; COMMIT;")?;
    }
    if v < 6 {
        // an event's own details (a line set's radio buoys, baskets and spacing), as JSON
        c.execute_batch("BEGIN; ALTER TABLE events ADD COLUMN data TEXT NOT NULL DEFAULT '{}'; UPDATE meta SET value = '6' WHERE key = 'schema'; COMMIT;")?;
    }
    if v < 7 {
        // what a track's depths are measured from (see Track::depth_ref); TimeZero's are
        // already reduced to chart datum
        c.execute_batch(
            "BEGIN;
             ALTER TABLE tracks ADD COLUMN depth_ref TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN depth_offset REAL;
             UPDATE tracks SET depth_ref = 'datum' WHERE source = 'timezero' AND has_depth = 1;
             UPDATE meta SET value = '7' WHERE key = 'schema';
             COMMIT;",
        )?;
    }
    if v < 8 {
        // a mark's own details as JSON (an anchor's swing radius, bottom type and holding)
        c.execute_batch("BEGIN; ALTER TABLE marks ADD COLUMN data TEXT NOT NULL DEFAULT '{}'; UPDATE meta SET value = '8' WHERE key = 'schema'; COMMIT;")?;
    }
    Ok(())
}

fn create(c: &Connection) -> Result<()> {
    let have: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE name = 'meta'", [], |r| r.get(0))?;
    if have > 0 {
        return Ok(());
    }
    // GeoPackage 1.3: application id "GPKG", version 10300, and its required tables
    c.execute_batch(
        r#"
        PRAGMA application_id = 1196444487;
        PRAGMA user_version = 10300;
        BEGIN;
        CREATE TABLE gpkg_spatial_ref_sys (
            srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
            organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
        INSERT INTO gpkg_spatial_ref_sys VALUES
            ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', 'undefined cartesian coordinate reference system'),
            ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', 'undefined geographic coordinate reference system'),
            ('WGS 84 geodetic', 4326, 'EPSG', 4326,
             'GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]]',
             'longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid');
        CREATE TABLE gpkg_contents (
            table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE,
            description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER,
            CONSTRAINT fk_gc_r_srs_id FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id));
        CREATE TABLE gpkg_geometry_columns (
            table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
            srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL,
            CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name),
            CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name),
            CONSTRAINT fk_gc_srs FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys (srs_id));

        CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
        CREATE TABLE imports (id INTEGER PRIMARY KEY, file TEXT NOT NULL, kind TEXT NOT NULL, at INTEGER NOT NULL, report TEXT);
        CREATE TABLE trips (id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT '', number TEXT NOT NULL DEFAULT '',
            fishery TEXT NOT NULL DEFAULT '', started INTEGER, ended INTEGER, notes TEXT NOT NULL DEFAULT '', deleted_at INTEGER);

        CREATE TABLE marks (
            fid INTEGER PRIMARY KEY AUTOINCREMENT, geom BLOB,
            id TEXT NOT NULL UNIQUE, name TEXT NOT NULL DEFAULT '', lat REAL NOT NULL, lon REAL NOT NULL,
            kind TEXT NOT NULL DEFAULT 'mark', notes TEXT NOT NULL DEFAULT '', grp TEXT NOT NULL DEFAULT '',
            barrier_nm REAL NOT NULL DEFAULT 0, modified INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL DEFAULT 0,
            date_unknown INTEGER NOT NULL DEFAULT 0, colour TEXT NOT NULL DEFAULT '', symbol TEXT NOT NULL DEFAULT '',
            import_id INTEGER, trip_id INTEGER, deleted_at INTEGER);
        CREATE INDEX marks_grp ON marks(grp);
        CREATE INDEX marks_lat ON marks(lat);
        CREATE INDEX marks_import ON marks(import_id);
        INSERT INTO gpkg_contents (table_name, data_type, identifier, description, srs_id)
            VALUES ('marks', 'features', 'TweedPlot marks', 'Marks and waypoints', 4326);
        INSERT INTO gpkg_geometry_columns VALUES ('marks', 'geom', 'POINT', 4326, 0, 0);

        CREATE TABLE routes (
            id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', notes TEXT NOT NULL DEFAULT '',
            start_lon REAL, start_lat REAL, start_mark TEXT, modified INTEGER NOT NULL DEFAULT 0,
            import_id INTEGER, trip_id INTEGER, deleted_at INTEGER);
        CREATE TABLE route_marks (
            route_id TEXT NOT NULL REFERENCES routes(id) ON DELETE CASCADE, seq INTEGER NOT NULL,
            mark_id TEXT NOT NULL, rounding TEXT NOT NULL DEFAULT '', radius_nm REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (route_id, seq));
        CREATE INDEX route_marks_mark ON route_marks(mark_id);

        CREATE TABLE routings (
            id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', created INTEGER NOT NULL DEFAULT 0,
            route_id TEXT NOT NULL DEFAULT '', route_name TEXT NOT NULL DEFAULT '',
            t0 INTEGER, status TEXT, arrival INTEGER, distance_nm REAL, grib_name TEXT, polar TEXT,
            request TEXT NOT NULL DEFAULT 'null', result TEXT NOT NULL DEFAULT 'null', deleted_at INTEGER);

        CREATE TABLE barriers (
            id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', kind TEXT NOT NULL DEFAULT 'line',
            points TEXT NOT NULL DEFAULT '[]', active INTEGER NOT NULL DEFAULT 1, notes TEXT NOT NULL DEFAULT '',
            modified INTEGER NOT NULL DEFAULT 0, deleted_at INTEGER);

        CREATE TABLE tracks (
            id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', notes TEXT NOT NULL DEFAULT '',
            colour TEXT NOT NULL DEFAULT '', source TEXT NOT NULL DEFAULT '', modified INTEGER NOT NULL DEFAULT 0,
            points INTEGER NOT NULL DEFAULT 0, start INTEGER, "end" INTEGER, distance_nm REAL NOT NULL DEFAULT 0,
            max_sog REAL, west REAL, south REAL, east REAL, north REAL,
            has_depth INTEGER NOT NULL DEFAULT 0, has_temp INTEGER NOT NULL DEFAULT 0,
            import_id INTEGER, trip_id INTEGER, deleted_at INTEGER);
        CREATE INDEX tracks_start ON tracks(start);
        CREATE INDEX tracks_import ON tracks(import_id);
        CREATE TABLE track_data (track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE, data BLOB NOT NULL);
        COMMIT;
        "#,
    )?;
    set_meta(c, "schema", "1")?; // then migrate() brings it up to date
    Ok(())
}

pub fn meta(c: &Connection, key: &str) -> Result<Option<String>> {
    Ok(c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}

pub fn set_meta(c: &Connection, key: &str, value: &str) -> Result<()> {
    c.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
    Ok(())
}

/// A GeoPackage point: the "GP" header (version 0, little-endian, no envelope, srs 4326) and WKB.
pub fn gpkg_point(lon: f64, lat: f64) -> Vec<u8> {
    let mut b = Vec::with_capacity(29);
    b.extend_from_slice(b"GP");
    b.push(0); // version
    b.push(0b0000_0001); // little-endian, no envelope, not empty
    b.extend_from_slice(&4326i32.to_le_bytes());
    b.push(1); // WKB little-endian
    b.extend_from_slice(&1u32.to_le_bytes()); // point
    b.extend_from_slice(&lon.to_le_bytes());
    b.extend_from_slice(&lat.to_le_bytes());
    b
}

// ---------- track fixes ----------
//
// A track's segments as one blob: deflate of
//   u8 format (1), varint number of segments, then each: varint number of fixes, then each fix:
//   u8 which values follow (bit 0 time, 1 SOG, 2 COG, 3 depth, 4 temperature), then
//   time (zigzag varint, change from the last fix with a time, seconds),
//   lat, lon (zigzag varint, change from the last fix, 1e-7 degrees: about 1 cm),
//   SOG (varint, 0.01 kn), COG (varint, 0.1°), depth (zigzag varint, 0.1 m), temperature (zigzag varint, 0.01 °C).
// About 8 to 10 bytes a fix before compression, a third of that after.

fn put_var(b: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        b.push((v as u8) | 0x80);
        v >>= 7;
    }
    b.push(v as u8);
}
fn put_zig(b: &mut Vec<u8>, v: i64) {
    put_var(b, ((v << 1) ^ (v >> 63)) as u64);
}
fn get_var(b: &[u8], at: &mut usize) -> Option<u64> {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let x = *b.get(*at)?;
        *at += 1;
        v |= ((x & 0x7f) as u64) << shift;
        if x & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}
fn get_zig(b: &[u8], at: &mut usize) -> Option<i64> {
    get_var(b, at).map(|v| ((v >> 1) as i64) ^ -((v & 1) as i64))
}

pub fn encode_segments(segs: &[Vec<TrackPoint>]) -> Vec<u8> {
    let mut b = Vec::with_capacity(segs.iter().map(Vec::len).sum::<usize>() * 10 + 8);
    b.push(1);
    put_var(&mut b, segs.len() as u64);
    let (mut lt, mut la, mut lo) = (0i64, 0i64, 0i64);
    for s in segs {
        put_var(&mut b, s.len() as u64);
        for p in s {
            let q = |v: Option<f64>| v.filter(|x| x.is_finite());
            let (sog, cog, dep, tmp) = (q(p.sog), q(p.cog), q(p.depth), q(p.temp));
            let flags = p.t.is_some() as u8 | (sog.is_some() as u8) << 1 | (cog.is_some() as u8) << 2 | (dep.is_some() as u8) << 3 | (tmp.is_some() as u8) << 4;
            b.push(flags);
            if let Some(t) = p.t {
                put_zig(&mut b, t - lt);
                lt = t;
            }
            let a = (p.lat * 1e7).round() as i64;
            let o = (p.lon * 1e7).round() as i64;
            put_zig(&mut b, a - la);
            put_zig(&mut b, o - lo);
            la = a;
            lo = o;
            if let Some(v) = sog {
                put_var(&mut b, (v.max(0.0) * 100.0).round() as u64);
            }
            if let Some(v) = cog {
                put_var(&mut b, (v.rem_euclid(360.0) * 10.0).round() as u64);
            }
            if let Some(v) = dep {
                put_zig(&mut b, (v * 10.0).round() as i64);
            }
            if let Some(v) = tmp {
                put_zig(&mut b, (v * 100.0).round() as i64);
            }
        }
    }
    use std::io::Write;
    let mut z = flate2::write::DeflateEncoder::new(Vec::with_capacity(b.len() / 2), flate2::Compression::fast());
    let _ = z.write_all(&b);
    z.finish().unwrap_or_default()
}

pub fn decode_segments(z: &[u8]) -> Result<Vec<Vec<TrackPoint>>> {
    use std::io::Read;
    let mut b = Vec::with_capacity(z.len() * 3);
    flate2::read::DeflateDecoder::new(z).read_to_end(&mut b)?;
    let bad = || crate::Error::Store("damaged track data".into());
    if b.first() != Some(&1) {
        return Err(bad());
    }
    let mut at = 1;
    let n = get_var(&b, &mut at).ok_or_else(bad)? as usize;
    let mut segs = Vec::with_capacity(n.min(10_000));
    let (mut lt, mut la, mut lo) = (0i64, 0i64, 0i64);
    for _ in 0..n {
        let k = get_var(&b, &mut at).ok_or_else(bad)? as usize;
        let mut s = Vec::with_capacity(k.min(1 << 20));
        for _ in 0..k {
            let f = *b.get(at).ok_or_else(bad)?;
            at += 1;
            let mut p = TrackPoint::default();
            if f & 1 != 0 {
                lt += get_zig(&b, &mut at).ok_or_else(bad)?;
                p.t = Some(lt);
            }
            la += get_zig(&b, &mut at).ok_or_else(bad)?;
            lo += get_zig(&b, &mut at).ok_or_else(bad)?;
            p.lat = la as f64 / 1e7;
            p.lon = lo as f64 / 1e7;
            if f & 2 != 0 {
                p.sog = Some(get_var(&b, &mut at).ok_or_else(bad)? as f64 / 100.0);
            }
            if f & 4 != 0 {
                p.cog = Some(get_var(&b, &mut at).ok_or_else(bad)? as f64 / 10.0);
            }
            if f & 8 != 0 {
                p.depth = Some(get_zig(&b, &mut at).ok_or_else(bad)? as f64 / 10.0);
            }
            if f & 16 != 0 {
                p.temp = Some(get_zig(&b, &mut at).ok_or_else(bad)? as f64 / 100.0);
            }
            s.push(p);
        }
        segs.push(s);
    }
    Ok(segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_round_trip() {
        let mk = |t: i64, lat: f64, lon: f64| TrackPoint { t: Some(t), lat, lon, sog: Some(6.23), cog: Some(359.9), depth: Some(121.5), temp: Some(16.84) };
        let segs = vec![
            vec![
                mk(1_700_000_000, -41.2612345, 173.2812345),
                mk(1_700_000_010, -41.2613, 173.2815),
                TrackPoint { t: None, lat: -41.3, lon: 173.3, ..Default::default() },
            ],
            vec![mk(1_699_999_000, -41.0, 179.9999999), mk(1_699_999_010, -41.0, -179.9999999)],
        ];
        let z = encode_segments(&segs);
        let back = decode_segments(&z).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].len(), 3);
        for (a, b) in segs.iter().flatten().zip(back.iter().flatten()) {
            assert_eq!(a.t, b.t);
            assert!((a.lat - b.lat).abs() < 1e-7 && (a.lon - b.lon).abs() < 1e-7);
            assert_eq!(a.sog.map(|v| (v * 100.0).round()), b.sog.map(|v| (v * 100.0).round()));
            assert_eq!(a.depth, b.depth);
            assert_eq!(a.temp.map(|v| (v * 100.0).round()), b.temp.map(|v| (v * 100.0).round()));
        }
        assert_eq!(back[0][2].sog, None);
    }

    #[test]
    fn geopackage_point_header() {
        let g = gpkg_point(174.5, -36.8);
        assert_eq!(&g[..2], b"GP");
        assert_eq!(g.len(), 29);
        assert_eq!(f64::from_le_bytes(g[13..21].try_into().unwrap()), 174.5);
    }
}
