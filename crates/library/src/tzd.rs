//! TimeZero user data (`.tzd`, also inside a `.TzBackup`): a zip holding
//! `WaypointRouteDatabase.xml` (marks, routes, lines, areas, circles, layers),
//! `MobileDescription.xml` (AIS names) and `Tracks.bin` (tracks, not read here yet).
//!
//! WaypointRouteDatabase.xml (Version 1.0, TimeZero 2.3 / 4.x):
//! - `<Layer Id Name>`: TimeZero's layers; they become the marks' groups.
//! - `<Point Id Latitude Longitude PointType Icon Color LayerId Name CreationDate HtmlNote ...>`:
//!   PointType 0 is a mark, 2 a text label. Icon is the symbol number (none = 0), the same
//!   number TimeZero's GPX writes as MxTimeZeroSymbol. Color is a palette number (below).
//! - `<MultiPoint Id MultiPointType Name Color LayerId [Closed Radius]>`: type 0 a route,
//!   2 a line (Closed="true": an area), 3 a circle (one centre point and a Radius).
//! - `<Relation IdMultiPoint IdPoint Rank>`: the points of each MultiPoint, in Rank order.
//!   Points used by a MultiPoint are its vertices, not marks of their own.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::gpx::{GpxData, GpxRoute, GpxWaypoint};
use crate::tracks::parse_iso;
use crate::{Error, Result};

/// TimeZero's mark colour numbers, worked out by matching the same marks in a `.tzd` and
/// in TimeZero's GPX export of them (which writes the colour itself). Numbers not seen
/// yet (0, 10, 12, 13, 14) give no colour, so the mark is drawn in TweedPlot's own.
pub fn tz_colour(n: u32) -> &'static str {
    match n {
        1 => "#ff0000",
        2 => "#008000",
        3 => "#0000ff",
        4 => "#00ffff",
        5 => "#ff00ff",
        6 => "#000000",
        7 => "#ffc0cb",
        8 => "#90ee90",
        9 => "#ffff00",
        11 => "#ffa500",
        15 => "#9400d3",
        16 => "#7f5d5d",
        17 => "#800000",
        18 => "#d3d3d3",
        19 => "#a9a9a9",
        _ => "",
    }
}

#[derive(Debug, Default)]
pub struct TzdData {
    pub gpx: GpxData,
    /// Text labels (PointType 2) brought in as marks.
    pub labels: usize,
    /// Lines, areas and circles: not imported yet.
    pub lines: usize,
    pub areas: usize,
    pub circles: usize,
}

/// Read a `.tzd`/`.TzBackup` zip, or a `WaypointRouteDatabase.xml` taken out of one.
pub fn read_tzd(path: &Path) -> Result<TzdData> {
    let xml = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) {
        std::fs::read(path)?
    } else {
        let f = std::fs::File::open(path)?;
        let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| Error::Tz(format!("not a TimeZero file: {e}")))?;
        let name = (0..z.len())
            .filter_map(|i| z.name_for_index(i).and_then(|n| n.ok()).map(|n| n.to_string()))
            .find(|n| n.rsplit(['/', '\\']).next().is_some_and(|b| b.eq_ignore_ascii_case("WaypointRouteDatabase.xml")))
            .ok_or_else(|| Error::Tz("no WaypointRouteDatabase.xml in this file".into()))?;
        let mut e = z.by_name(&name).map_err(|e| Error::Tz(e.to_string()))?;
        let mut v = Vec::new();
        e.read_to_end(&mut v)?;
        v
    };
    parse_tzd_xml(&String::from_utf8_lossy(&xml))
}

#[derive(Default)]
struct Pt {
    w: GpxWaypoint,
    layer: String,
    label: bool,
}

struct Multi {
    id: String,
    kind: u32,
    name: String,
    closed: bool,
}

pub fn parse_tzd_xml(text: &str) -> Result<TzdData> {
    let mut r = Reader::from_str(text.trim_start_matches('\u{feff}'));
    let mut layers: HashMap<String, String> = HashMap::new();
    let mut points: Vec<(String, Pt)> = Vec::new();
    let mut multis: Vec<Multi> = Vec::new();
    let mut rel: HashMap<String, Vec<(f64, String)>> = HashMap::new();
    loop {
        match r.read_event().map_err(|e| Error::Tz(e.to_string()))? {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => {
                let tag = e.local_name();
                let mut a: HashMap<String, String> = HashMap::new();
                for x in e.attributes().flatten() {
                    let raw: &str = &x.value;
                    let v = quick_xml::escape::unescape(raw).map(|v| v.into_owned()).unwrap_or_else(|_| raw.to_string());
                    let k: &str = x.key.local_name().into_inner();
                    a.insert(k.to_string(), v);
                }
                let get = |k: &str| a.get(k).map(String::as_str).unwrap_or("");
                match tag.as_ref() {
                    "Layer" => {
                        layers.insert(get("Id").into(), get("Name").trim().into());
                    }
                    "Point" => {
                        let (Ok(lat), Ok(lon)) = (get("Latitude").parse::<f64>(), get("Longitude").parse::<f64>()) else { continue };
                        let w = GpxWaypoint {
                            name: get("Name").trim().into(),
                            lat,
                            lon,
                            desc: html_text(get("HtmlNote")),
                            sym: String::new(),
                            time: parse_iso(get("CreationDate")),
                            colour: get("Color").parse().map(tz_colour).unwrap_or("").into(),
                            tz_symbol: Some(get("Icon").parse().unwrap_or(0)),
                            group: String::new(),
                        };
                        points.push((get("Id").into(), Pt { w, layer: get("LayerId").into(), label: get("PointType") == "2" }));
                    }
                    "MultiPoint" => multis.push(Multi {
                        id: get("Id").into(),
                        kind: get("MultiPointType").parse().unwrap_or(0),
                        name: get("Name").trim().into(),
                        closed: get("Closed") == "true",
                    }),
                    "Relation" => {
                        rel.entry(get("IdMultiPoint").into()).or_default().push((get("Rank").parse().unwrap_or(0.0), get("IdPoint").into()));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let used: HashSet<&str> = rel.values().flatten().map(|(_, p)| p.as_str()).collect();
    let mut out = TzdData::default();
    let by_id: HashMap<&str, &Pt> = points.iter().map(|(i, p)| (i.as_str(), p)).collect();
    let with_group = |p: &Pt| GpxWaypoint { group: layers.get(&p.layer).cloned().unwrap_or_default(), ..p.w.clone() };
    for (id, p) in &points {
        if used.contains(id.as_str()) {
            continue;
        }
        out.labels += p.label as usize;
        out.gpx.waypoints.push(with_group(p));
    }
    for m in &multis {
        let mut pts = rel.get(&m.id).cloned().unwrap_or_default();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        match m.kind {
            0 if pts.len() >= 2 => out.gpx.routes.push(GpxRoute {
                name: m.name.clone(),
                points: pts.iter().filter_map(|(_, i)| by_id.get(i.as_str()).map(|p| with_group(p))).collect(),
            }),
            0 => {} // an empty or one-point route
            2 if m.closed => out.areas += 1,
            2 => out.lines += 1,
            3 => out.circles += 1,
            _ => {}
        }
    }
    Ok(out)
}

/// The text of a TimeZero HTML note ("" for its empty `<TITLE></TITLE>` default).
fn html_text(h: &str) -> String {
    let mut s = String::new();
    let mut tag = false;
    for c in h.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => s.push(c),
            _ => {}
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_marks_routes_and_shapes() {
        let x = r#"<?xml version="1.0"?><WaypointRouteDatabase Version="1.0"><Layers><Layer Id="5" Name="A2" /><Layer Id="2" Name="Routes" /></Layers>
<Points>
<Point Id="a" Latitude="-41.5" Longitude="175.5" PointType="0" Icon="14" Color="1" LayerId="5" CreationDate="2007-03-24T06:30:28Z" HtmlNote="&lt;TITLE&gt;&lt;/TITLE&gt;" />
<Point Id="b" Name="  6  " Latitude="-41.6" Longitude="175.6" PointType="0" Color="2" LayerId="5" CreationDate="2008-06-25T12:00:00Z" HtmlNote="&lt;TITLE&gt;good &amp; deep&lt;/TITLE&gt;" />
<Point Id="t" Name="TIM'S" Latitude="-39.9" Longitude="178.1" PointType="2" Color="1" LayerId="5" CreationDate="2007-05-06T18:19:15Z" FontSize="14" />
<Point Id="r1" Latitude="-41" Longitude="174" PointType="0" LayerId="2" CreationDate="2012-06-22T08:56:26Z" />
<Point Id="r2" Latitude="-41.1" Longitude="174.1" PointType="0" LayerId="2" CreationDate="2012-06-22T08:56:26Z" />
<Point Id="v1" Latitude="-40" Longitude="170" PointType="0" LayerId="5" CreationDate="2012-06-22T08:56:26Z" />
</Points><MultiPoints>
<MultiPoint Id="R" MultiPointType="0" Name="Rte 6/22/2012" Color="3" LayerId="2" />
<MultiPoint Id="L" MultiPointType="2" Color="4" Closed="true" LayerId="5" />
</MultiPoints><Relations>
<Relation Id="x" IdMultiPoint="R" IdPoint="r2" Rank="1" /><Relation Id="y" IdMultiPoint="R" IdPoint="r1" Rank="0" />
<Relation Id="z" IdMultiPoint="L" IdPoint="v1" Rank="0" />
</Relations></WaypointRouteDatabase>"#;
        let d = parse_tzd_xml(x).unwrap();
        assert_eq!(d.gpx.waypoints.len(), 3); // a, b and the label; route points and vertices aren't marks
        let a = &d.gpx.waypoints[0];
        assert_eq!((a.tz_symbol, a.colour.as_str(), a.group.as_str(), a.name.as_str()), (Some(14), "#ff0000", "A2", ""));
        assert_eq!(a.time, parse_iso("2007-03-24T06:30:28Z"));
        let b = &d.gpx.waypoints[1];
        assert_eq!((b.tz_symbol, b.name.as_str(), b.desc.as_str(), b.colour.as_str()), (Some(0), "6", "good & deep", "#008000"));
        assert_eq!(d.labels, 1);
        assert_eq!(d.gpx.routes.len(), 1);
        assert_eq!(d.gpx.routes[0].points.iter().map(|p| p.lon).collect::<Vec<_>>(), vec![174.0, 174.1]);
        assert_eq!(d.gpx.routes[0].points[0].group, "Routes");
        assert_eq!(d.areas, 1);
    }
}

// ---------- tracks (Tracks.bin) ----------
//
// Little-endian, written by .NET's BinaryWriter (strings: a 7-bit length, then the bytes).
//   u16 version (1), u32 number of tracks, then each track:
//   string "TRACK", u8 n + n bytes (GUID), u8 mobile type (0 own ship, 6 imported from
//   MaxSea), string mobile id ("OwnShip", "PtfMobile"), u32, u32, string name ("Ownship",
//   "AIS SARDA"), u32 bytes of points, u32 number of points, then each point:
//   u32 n, then n bytes: f32 x, f32 y (WGS84 ellipsoidal Mercator metres, EPSG:3395),
//   f32 depth (m, negative down; 0 or f32::MAX for none), i64 time (.NET DateTime.ToBinary:
//   ticks of 100 ns since 0001-01-01, the top two bits the kind), u8 1, u8 colour (palette
//   number, as marks), u16 0, u16 k, then k times u8 tag + f32 value: 2 heading (radians),
//   3 SOG (kn), 6 COG (radians), 8 sea temperature (°C); f32::MAX for none.
// Worked out from TimeZero 4 files: positions match the marks dropped at the boat to 0.1 m.

/// One TimeZero track, as stored (every fix, usually 2 s apart).
#[derive(Debug, Clone, Default)]
pub struct TzTrack {
    /// The object's type: "TRACK" (a logged track), "LINE" (a line drawn or brought in from MaxSea).
    pub kind: String,
    pub name: String,
    pub mobile: String,
    pub colour: String,
    pub points: Vec<TzFix>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TzFix {
    pub t: i64,
    pub lat: f64,
    pub lon: f64,
    pub depth: Option<f32>,
    pub sog: Option<f32>,
    pub cog: Option<f32>,
    pub temp: Option<f32>,
    pub colour: u8,
}

const R_WGS84: f64 = 6_378_137.0;
const E_WGS84: f64 = 0.081_819_190_842_622;

/// Latitude and longitude (degrees) from WGS84 ellipsoidal Mercator metres.
pub fn mercator_to_latlon(x: f64, y: f64) -> (f64, f64) {
    let t = (-y / R_WGS84).exp();
    let mut phi = std::f64::consts::FRAC_PI_2 - 2.0 * t.atan();
    for _ in 0..10 {
        let es = E_WGS84 * phi.sin();
        let next = std::f64::consts::FRAC_PI_2 - 2.0 * (t * ((1.0 - es) / (1.0 + es)).powf(E_WGS84 / 2.0)).atan();
        let done = (next - phi).abs() < 1e-12;
        phi = next;
        if done {
            break;
        }
    }
    (phi.to_degrees(), (x / R_WGS84).to_degrees())
}

/// Unix seconds from .NET DateTime.ToBinary.
fn dotnet_time(v: i64) -> i64 {
    let ticks = v & 0x3fff_ffff_ffff_ffff;
    ticks / 10_000_000 - 62_135_596_800
}

struct Bin<R: Read> {
    r: R,
}

impl<R: Read> Bin<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b)?;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn skip(&mut self, n: u64) -> Result<()> {
        std::io::copy(&mut (&mut self.r).take(n), &mut std::io::sink())?;
        Ok(())
    }
    fn string(&mut self) -> Result<String> {
        let (mut n, mut shift) = (0usize, 0);
        loop {
            let b = self.u8()?;
            n |= ((b & 0x7f) as usize) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return Err(Error::Tz("bad string in Tracks.bin".into()));
            }
        }
        let mut v = vec![0u8; n];
        self.r.read_exact(&mut v)?;
        Ok(String::from_utf8_lossy(&v).into_owned())
    }
}

fn some(v: f32) -> Option<f32> {
    (v.is_finite() && v.abs() < 1e30).then_some(v)
}

/// Read TimeZero tracks one at a time (a Tracks.bin can be gigabytes).
pub fn read_tz_tracks(r: impl Read, mut each: impl FnMut(TzTrack) -> Result<()>) -> Result<TzTracksRead> {
    let mut b = Bin { r: std::io::BufReader::with_capacity(1 << 20, Counted { r, at: 0 }) };
    let _version = b.u16()?;
    let count = b.u32()? as usize;
    let mut out = TzTracksRead { count, ..Default::default() };
    for k in 0..count {
        let at = b.r.get_ref().at - b.r.buffer().len() as u64;
        match read_one(&mut b) {
            Ok(Some(t)) if t.kind == "TRACK" => {
                out.read += 1;
                each(t)?;
            }
            Ok(Some(t)) => *out.other.entry(t.kind).or_default() += 1,
            Ok(None) => {}
            Err(e) => {
                // keep what came before; say where it stopped, with the bytes there, so the
                // format can be worked out
                let mut peek = [0u8; 48];
                let n = b.r.read(&mut peek).unwrap_or(0);
                let hex: Vec<String> = peek[..n].iter().map(|x| format!("{x:02x}")).collect();
                out.problem = Some(format!("stopped at object {} of {count} (byte {at}): {e}; next bytes {}", k + 1, hex.join(" ")));
                break;
            }
        }
    }
    Ok(out)
}

/// How a Tracks.bin read went.
#[derive(Debug, Default)]
pub struct TzTracksRead {
    /// Objects in the file (tracks and lines).
    pub count: usize,
    /// Objects other than tracks, by type ("LINE": n).
    pub other: std::collections::BTreeMap<String, usize>,
    pub read: usize,
    pub problem: Option<String>,
}

struct Counted<R: Read> {
    r: R,
    at: u64,
}
impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.r.read(buf)?;
        self.at += n as u64;
        Ok(n)
    }
}

fn read_one<R: Read>(b: &mut Bin<std::io::BufReader<Counted<R>>>) -> Result<Option<TzTrack>> {
    // TRACK, and LINE (seen in files brought in from MaxSea), share one layout; other
    // types may too, so any short upper-case name is read the same way
    let tag = b.string()?;
    if tag.is_empty() || tag.len() > 16 || !tag.bytes().all(|c| c.is_ascii_uppercase()) {
        return Err(Error::Tz(format!("an object starts with {tag:?}, not TRACK or LINE")));
    }
    let g = b.u8()? as u64;
    b.skip(g)?;
    let _mobile_type = b.u8()?;
    let mobile = b.string()?;
    let (_a, _b) = (b.u32()?, b.u32()?);
    let name = b.string()?;
    let bytes = b.u32()? as usize;
    let n = b.u32()? as usize;
    // the points are read by their byte count (not their number), so a track whose count
    // doesn't match its points can't put the rest of the file out of step
    let mut buf = vec![0u8; bytes];
    b.r.read_exact(&mut buf)?;
    let mut points = Vec::with_capacity(n.min(bytes / 30 + 1));
    let mut colours = [0usize; 256];
    let mut pos = 0;
    while pos + 4 <= buf.len() {
        let len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + len > buf.len() {
            break;
        }
        let rec = &buf[pos..pos + len];
        pos += len;
        if len < 26 {
            continue;
        }
        let f = |i: usize| f32::from_le_bytes(rec[i..i + 4].try_into().unwrap());
        let (lat, lon) = mercator_to_latlon(f(0) as f64, f(4) as f64);
        let mut p = TzFix {
            t: dotnet_time(i64::from_le_bytes(rec[12..20].try_into().unwrap())),
            lat,
            lon,
            depth: some(f(8)).filter(|d| *d != 0.0).map(f32::abs),
            colour: rec[21],
            ..Default::default()
        };
        colours[rec[21] as usize] += 1;
        let k = u16::from_le_bytes([rec[24], rec[25]]) as usize;
        for j in 0..k {
            let at = 26 + j * 5;
            if at + 5 > len {
                break;
            }
            let v = some(f(at + 1));
            match rec[at] {
                3 => p.sog = v,
                6 => p.cog = v.map(|r| r.to_degrees().rem_euclid(360.0)),
                8 => p.temp = v,
                _ => {}
            }
        }
        if lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 {
            points.push(p);
        }
    }
    let common = (0..256).max_by_key(|&i| colours[i]).unwrap_or(0) as u32;
    Ok(Some(TzTrack { kind: tag, name, mobile, colour: tz_colour(common).into(), points }))
}

/// The Tracks.bin inside a .tzd / .TzBackup, read as it unzips.
pub fn read_tzd_tracks(path: &Path, each: impl FnMut(TzTrack) -> Result<()>) -> Result<Option<TzTracksRead>> {
    let f = std::fs::File::open(path)?;
    let Ok(mut z) = zip::ZipArchive::new(std::io::BufReader::new(f)) else { return Ok(None) };
    let Some(name) = (0..z.len())
        .filter_map(|i| z.name_for_index(i).and_then(|n| n.ok()).map(|n| n.to_string()))
        .find(|n| n.rsplit(['/', '\\']).next().is_some_and(|b| b.eq_ignore_ascii_case("Tracks.bin")))
    else {
        return Ok(None);
    };
    let e = z.by_name(&name).map_err(|e| Error::Tz(e.to_string()))?;
    read_tz_tracks(e, each).map(Some)
}

#[cfg(test)]
mod track_tests {
    use super::*;

    #[test]
    fn mercator_and_time() {
        // a fix from a TimeZero track and the mark dropped there (to 0.1 m)
        let (lat, lon) = mercator_to_latlon(18_970_678.0, -4_637_761.5);
        assert!((lon - 170.416_498).abs() < 2e-6, "{lon}");
        assert!((lat - -38.599_202).abs() < 2e-6, "{lat}");
        assert_eq!(dotnet_time(0x48d8_cdfe_1a46_6715u64 as i64), 1_612_987_051); // 2021-02-10T19:57:31Z
    }

    #[test]
    fn reads_a_track() {
        let mut v = Vec::new();
        v.extend(1u16.to_le_bytes());
        v.extend(1u32.to_le_bytes());
        let s = |v: &mut Vec<u8>, t: &str| {
            v.push(t.len() as u8);
            v.extend(t.as_bytes());
        };
        s(&mut v, "TRACK");
        v.push(16);
        v.extend([0u8; 16]);
        v.push(0);
        s(&mut v, "OwnShip");
        v.extend([0u8; 8]);
        s(&mut v, "Ownship");
        v.extend(45u32.to_le_bytes()); // bytes of points
        v.extend(1u32.to_le_bytes());
        v.extend(41u32.to_le_bytes());
        v.extend(18_970_678.0f32.to_le_bytes());
        v.extend((-4_637_761.5f32).to_le_bytes());
        v.extend((-121.5f32).to_le_bytes());
        v.extend((0x48d8_cdfe_1a46_6715u64).to_le_bytes());
        v.extend([1, 9, 0, 0]);
        v.extend(3u16.to_le_bytes());
        for (t, x) in [(3u8, 6.13f32), (6, std::f32::consts::PI), (8, 16.8)] {
            v.push(t);
            v.extend(x.to_le_bytes());
        }
        let mut got = Vec::new();
        read_tz_tracks(&v[..], |t| {
            got.push(t);
            Ok(())
        })
        .unwrap();
        let p = got[0].points[0];
        assert_eq!((got[0].name.as_str(), got[0].colour.as_str()), ("Ownship", "#ffff00"));
        assert_eq!((p.depth, p.sog, p.temp), (Some(121.5), Some(6.13), Some(16.8)));
        assert!((p.cog.unwrap() - 180.0).abs() < 1e-3);
    }

    #[test]
    fn skips_lines_between_tracks() {
        // the LINE that stopped sarda's import (TimeZero 4, a line brought in from MaxSea), then a TRACK
        let s = |v: &mut Vec<u8>, t: &str| {
            v.push(t.len() as u8);
            v.extend(t.as_bytes());
        };
        let mut v = Vec::new();
        v.extend(1u16.to_le_bytes());
        v.extend(2u32.to_le_bytes());
        s(&mut v, "LINE");
        v.push(16);
        v.extend([0x7f, 0x8e, 0x25, 0x5d, 0xbe, 0xa9, 0xe1, 0x11, 0xaa, 0x3f, 0x14, 0xda, 0xe9, 0xcf, 0x4e, 0x2d]);
        v.push(6);
        s(&mut v, "PtfMobile");
        v.extend(5u32.to_le_bytes());
        v.extend(1u32.to_le_bytes());
        s(&mut v, "");
        v.extend(120u32.to_le_bytes());
        v.extend(4u32.to_le_bytes());
        for k in 0..4 {
            v.extend(26u32.to_le_bytes());
            v.extend((18_970_678.0f32 + k as f32 * 100.0).to_le_bytes());
            v.extend((-4_637_761.5f32).to_le_bytes());
            v.extend(0f32.to_le_bytes());
            v.extend((0x48d8_cdfe_1a46_6715u64).to_le_bytes());
            v.extend([1, 5, 0, 0, 0, 0]);
        }
        s(&mut v, "TRACK");
        v.push(16);
        v.extend([0u8; 16]);
        v.push(0);
        s(&mut v, "OwnShip");
        v.extend([0u8; 8]);
        s(&mut v, "Ownship");
        v.extend(30u32.to_le_bytes());
        v.extend(1u32.to_le_bytes());
        v.extend(26u32.to_le_bytes());
        v.extend(18_970_678.0f32.to_le_bytes());
        v.extend((-4_637_761.5f32).to_le_bytes());
        v.extend(0f32.to_le_bytes());
        v.extend((0x48d8_cdfe_1a46_6715u64).to_le_bytes());
        v.extend([1, 9, 0, 0, 0, 0]);
        let mut got = Vec::new();
        let r = read_tz_tracks(&v[..], |t| {
            got.push(t);
            Ok(())
        })
        .unwrap();
        assert_eq!(r.problem, None);
        assert_eq!((r.read, r.other.get("LINE").copied()), (1, Some(1)));
        assert_eq!(got[0].points.len(), 1);
    }
}