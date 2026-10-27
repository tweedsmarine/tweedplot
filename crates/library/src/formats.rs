//! Other plotters' and programs' files: KML and KMZ (Google Earth, and many plotters'
//! exports) and CSV (a spreadsheet of marks). They're read into the same shapes as GPX, so
//! they're matched, grouped and undone as any import.
//!
//! KML: a Placemark's Point is a mark, a LineString a route (60 points or fewer) or else a
//! track, a gx:Track a track with its times, a Polygon an area (a barrier, switched off).
//! Folders become groups; a style's icon or line colour is kept.
//!
//! CSV: a header row naming the columns (any order, any case): name, lat / latitude,
//! lon / long / lng / longitude, and if wanted notes / description, group / layer,
//! date / time, symbol / type, colour, depth. Positions may be decimal (-41.25) or degrees
//! and minutes (41 15.000 S, 41°15.000'S). Commas, semicolons or tabs.

use std::collections::HashMap;
use std::io::Read;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::gpx::{GpxData, GpxRoute, GpxTrack, GpxWaypoint};
use crate::tracks::{parse_iso, TrackPoint};
use crate::tzd::TzShape;
use crate::{Error, Result};

/// KML colour "aabbggrr" as "#rrggbb".
fn kml_colour(v: &str) -> String {
    let h = v.trim().trim_start_matches('#');
    if h.len() == 8 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!("#{}{}{}", &h[6..8], &h[4..6], &h[2..4]).to_ascii_lowercase();
    }
    String::new()
}

fn coords(text: &str) -> Vec<[f64; 2]> {
    text.split_whitespace()
        .filter_map(|t| {
            let mut it = t.split(',');
            let lon = it.next()?.trim().parse::<f64>().ok()?;
            let lat = it.next()?.trim().parse::<f64>().ok()?;
            (lat.abs() <= 90.0 && lon.abs() <= 360.0).then_some([lon, lat])
        })
        .collect()
}

/// Read a .kml file, or the KML inside a .kmz.
pub fn read_kml_file(path: &std::path::Path) -> Result<(GpxData, Vec<TzShape>)> {
    let bytes = std::fs::read(path)?;
    let text = if bytes.starts_with(b"PK") {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| Error::Store(format!("not a KMZ: {e}")))?;
        let name = (0..z.len())
            .filter_map(|i| z.name_for_index(i).and_then(|n| n.ok()).map(|n| n.to_string()))
            .find(|n| n.to_ascii_lowercase().ends_with(".kml"))
            .ok_or_else(|| Error::Store("no KML inside this KMZ".into()))?;
        let mut e = z.by_name(&name).map_err(|e| Error::Store(e.to_string()))?;
        let mut v = Vec::new();
        e.read_to_end(&mut v)?;
        String::from_utf8_lossy(&v).into_owned()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    parse_kml(&text)
}

#[derive(Default)]
struct Placemark {
    name: String,
    desc: String,
    when: Option<i64>,
    style: String,
    points: Vec<[f64; 2]>,
    lines: Vec<Vec<[f64; 2]>>,
    polygons: Vec<Vec<[f64; 2]>>,
    track_when: Vec<Option<i64>>,
    track_coord: Vec<[f64; 2]>,
}

pub fn parse_kml(text: &str) -> Result<(GpxData, Vec<TzShape>)> {
    let mut r = Reader::from_str(text.trim_start_matches('\u{feff}'));
    let mut path: Vec<String> = Vec::new(); // element names
    let mut folders: Vec<String> = Vec::new(); // folder names, innermost last
    let mut styles: HashMap<String, String> = HashMap::new(); // style id → colour
    let mut style_maps: HashMap<String, String> = HashMap::new(); // StyleMap id → normal style url
    let mut cur_style: Option<String> = None;
    let mut cur_map: Option<String> = None;
    let mut map_key = String::new();
    let mut pm: Option<Placemark> = None;
    let mut in_outer = false;
    let mut text_buf = String::new();
    let mut marks: Vec<(Placemark, String)> = Vec::new();
    loop {
        match r.read_event().map_err(|e| Error::Store(format!("KML: {e}")))? {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.local_name().into_inner().to_string();
                let id = e.attributes().flatten().find(|a| a.key.local_name().into_inner() == "id").map(|a| { let v: &str = &a.value; v.to_string() });
                match name.as_str() {
                    "Folder" => folders.push(String::new()),
                    "Placemark" => pm = Some(Placemark::default()),
                    "Style" => cur_style = id,
                    "StyleMap" => cur_map = id,
                    "outerBoundaryIs" => in_outer = true,
                    _ => {}
                }
                path.push(name);
                text_buf.clear();
            }
            Event::Text(t) => text_buf.push_str(&t),
            Event::CData(t) => text_buf.push_str(&t),
            Event::GeneralRef(g) => {
                let n: &str = &g;
                text_buf.push_str(match n { "amp" => "&", "lt" => "<", "gt" => ">", "quot" => "\"", "apos" => "'", _ => "" });
            }
            Event::End(_) => {
                let name = path.pop().unwrap_or_default();
                let parent = path.last().map(String::as_str).unwrap_or("");
                let t = text_buf.trim().to_string();
                match name.as_str() {
                    "name" if parent == "Folder" => {
                        if let Some(f) = folders.last_mut() {
                            *f = t.clone();
                        }
                    }
                    "name" if parent == "Placemark" => {
                        if let Some(p) = pm.as_mut() {
                            p.name = t.clone();
                        }
                    }
                    "description" if parent == "Placemark" => {
                        if let Some(p) = pm.as_mut() {
                            p.desc = strip_html(&t);
                        }
                    }
                    "when" if parent == "TimeStamp" => {
                        if let Some(p) = pm.as_mut() {
                            p.when = parse_iso(&t);
                        }
                    }
                    "when" if parent == "Track" => {
                        if let Some(p) = pm.as_mut() {
                            p.track_when.push(parse_iso(&t));
                        }
                    }
                    "coord" if parent == "Track" => {
                        if let Some(p) = pm.as_mut() {
                            let v: Vec<f64> = t.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                            if v.len() >= 2 {
                                p.track_coord.push([v[0], v[1]]);
                            }
                        }
                    }
                    "styleUrl" if parent == "Placemark" => {
                        if let Some(p) = pm.as_mut() {
                            p.style = t.trim_start_matches('#').to_string();
                        }
                    }
                    "key" if parent == "Pair" => map_key = t.clone(),
                    "styleUrl" if parent == "Pair" => {
                        if let (Some(m), "normal") = (cur_map.as_ref(), map_key.as_str()) {
                            style_maps.insert(m.clone(), t.trim_start_matches('#').to_string());
                        }
                    }
                    "color" if parent == "IconStyle" || parent == "LineStyle" => {
                        if let Some(s) = cur_style.as_ref() {
                            // the icon's colour wins over the line's
                            let c = kml_colour(&t);
                            if !c.is_empty() && (parent == "IconStyle" || !styles.contains_key(s)) {
                                styles.insert(s.clone(), c);
                            }
                        }
                    }
                    "coordinates" => {
                        if let Some(p) = pm.as_mut() {
                            let c = coords(&t);
                            match parent {
                                "Point" => p.points.extend(c.into_iter().take(1)),
                                "LineString" => p.lines.push(c),
                                "LinearRing" if in_outer => p.polygons.push(c),
                                _ => {}
                            }
                        }
                    }
                    "outerBoundaryIs" => in_outer = false,
                    "Style" => cur_style = None,
                    "StyleMap" => cur_map = None,
                    "Folder" => {
                        folders.pop();
                    }
                    "Placemark" => {
                        if let Some(p) = pm.take() {
                            let group = folders.iter().rev().find(|f| !f.is_empty()).cloned().unwrap_or_default();
                            marks.push((p, group));
                        }
                    }
                    _ => {}
                }
                text_buf.clear();
            }
            _ => {}
        }
    }
    let colour = |s: &str| styles.get(style_maps.get(s).map(String::as_str).unwrap_or(s)).cloned().unwrap_or_default();
    let mut out = GpxData::default();
    let mut shapes = Vec::new();
    for (k, (p, group)) in marks.into_iter().enumerate() {
        let col = colour(&p.style);
        for pt in &p.points {
            out.waypoints.push(GpxWaypoint { name: p.name.clone(), lat: pt[1], lon: pt[0], desc: p.desc.clone(), time: p.when, colour: col.clone(), group: group.clone(), ..Default::default() });
        }
        for line in p.lines.into_iter().filter(|l| l.len() >= 2) {
            if line.len() <= 60 {
                out.routes.push(GpxRoute {
                    name: if p.name.is_empty() { format!("Route {}", k + 1) } else { p.name.clone() },
                    points: line.iter().map(|c| GpxWaypoint { lat: c[1], lon: c[0], group: group.clone(), ..Default::default() }).collect(),
                });
            } else {
                out.tracks.push(GpxTrack {
                    name: p.name.clone(),
                    desc: p.desc.clone(),
                    segments: vec![line.iter().map(|c| TrackPoint { lat: c[1], lon: c[0], ..Default::default() }).collect()],
                });
            }
        }
        if p.track_coord.len() >= 2 {
            let seg = p.track_coord.iter().enumerate().map(|(i, c)| TrackPoint { lat: c[1], lon: c[0], t: p.track_when.get(i).copied().flatten(), ..Default::default() }).collect();
            out.tracks.push(GpxTrack { name: p.name.clone(), desc: p.desc.clone(), segments: vec![seg] });
        }
        for (j, poly) in p.polygons.into_iter().enumerate() {
            let mut pts = poly;
            if pts.len() > 1 && pts.first() == pts.last() {
                pts.pop();
            }
            if pts.len() < 3 {
                continue;
            }
            // an id from the shape itself, so importing the file again matches it
            let key = format!("{}|{:.5}|{:.5}|{}", p.name, pts[0][0], pts[0][1], pts.len());
            shapes.push(TzShape { id: format!("kml-{:016x}-{j}", fnv(&key)), name: p.name.clone(), kind: "area".into(), points: pts, radius_m: None, layer: group.clone(), colour: col.clone() });
        }
    }
    Ok((out, shapes))
}

pub(crate) fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn strip_html(h: &str) -> String {
    let mut s = String::new();
    let mut tag = false;
    for c in h.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                s.push(' ');
            }
            _ if !tag => s.push(c),
            _ => {}
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------- CSV ----------

/// A latitude or longitude as people write them: -41.25, 41.25S, 41 15.000 S, 41°15.000'S,
/// 41 15 00 S. `hemi` is "NS" or "EW".
pub fn parse_coord(s: &str, hemi: &str) -> Option<f64> {
    let mut t = s.trim().to_ascii_uppercase();
    if t.is_empty() {
        return None;
    }
    let mut sign = 1.0;
    if let Some(h) = t.chars().find(|c| "NSEW".contains(*c)) {
        if !hemi.contains(h) {
            return None;
        }
        if h == 'S' || h == 'W' {
            sign = -1.0;
        }
        t = t.replace(['N', 'S', 'E', 'W'], " ");
    }
    if t.trim_start().starts_with('-') {
        sign = -sign;
        t = t.replacen('-', " ", 1);
    }
    let nums: Vec<f64> = t.replace(['°', '\'', '"', '′', '″'], " ").split_whitespace().map(|x| x.parse::<f64>()).collect::<std::result::Result<_, _>>().ok()?;
    let (d, m, sec) = (*nums.first()?, nums.get(1).copied().unwrap_or(0.0), nums.get(2).copied().unwrap_or(0.0));
    if !(0.0..60.0).contains(&m) || !(0.0..60.0).contains(&sec) || nums.len() > 3 {
        return None;
    }
    let v = sign * (d.abs() + m / 60.0 + sec / 3600.0);
    let max = if hemi == "NS" { 90.0 } else { 360.0 };
    (v.abs() <= max).then_some(v)
}

fn split_csv(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// A date and time from a spreadsheet: ISO 8601, or day/month/year with an optional
/// hh:mm[:ss] (as New Zealand writes it), taken as UTC.
fn parse_when(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(t) = parse_iso(s).or_else(|| parse_iso(&format!("{}T00:00:00Z", s.get(..10)?)).filter(|_| s.len() == 10)) {
        return Some(t);
    }
    let (date, time) = s.split_once([' ', 'T']).unwrap_or((s, "00:00"));
    let d: Vec<u32> = date.split(['/', '.', '-']).filter_map(|x| x.parse().ok()).collect();
    if d.len() != 3 {
        return None;
    }
    let (day, mon, mut yr) = (d[0], d[1], d[2]);
    if yr < 100 {
        yr += 2000;
    }
    let tm: Vec<u32> = time.split(':').filter_map(|x| x.trim().parse().ok()).collect();
    let iso = format!("{yr:04}-{mon:02}-{day:02}T{:02}:{:02}:{:02}Z", tm.first().copied().unwrap_or(0), tm.get(1).copied().unwrap_or(0), tm.get(2).copied().unwrap_or(0));
    parse_iso(&iso)
}

/// Marks from CSV text; the notes say what was left out and why.
pub fn parse_csv(text: &str) -> Result<(GpxData, Vec<String>)> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head = lines.next().ok_or_else(|| Error::Store("the CSV file is empty".into()))?;
    let sep = [',', ';', '\t'].into_iter().max_by_key(|c| head.matches(*c).count()).unwrap_or(',');
    let cols: Vec<String> = split_csv(head, sep).into_iter().map(|c| c.to_ascii_lowercase().replace([' ', '_'], "")).collect();
    let find = |names: &[&str]| cols.iter().position(|c| names.contains(&c.as_str()));
    let lat = find(&["lat", "latitude", "y"]);
    let lon = find(&["lon", "long", "lng", "longitude", "x"]);
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return Err(Error::Store("the CSV needs a header row with columns named lat and lon (or latitude and longitude)".into()));
    };
    let name = find(&["name", "title", "label", "waypoint", "wpt", "mark"]);
    let notes = find(&["notes", "note", "description", "desc", "comment", "comments"]);
    let group = find(&["group", "layer", "folder", "category"]);
    let when = find(&["date", "time", "datetime", "created", "timestamp", "when"]);
    let sym = find(&["symbol", "sym", "icon", "type", "kind"]);
    let colour = find(&["colour", "color"]);
    let depth = find(&["depth", "depthm"]);
    let mut out = GpxData::default();
    let mut bad = Vec::new();
    for (k, line) in lines.enumerate() {
        let v = split_csv(line, sep);
        let get = |i: Option<usize>| i.and_then(|i| v.get(i)).map(String::as_str).unwrap_or("");
        let (Some(la), Some(lo)) = (parse_coord(get(Some(lat)), "NS"), parse_coord(get(Some(lon)), "EW")) else {
            bad.push(k + 2);
            continue;
        };
        let mut desc = get(notes).to_string();
        if let Some(d) = get(depth).parse::<f64>().ok() {
            desc = if desc.is_empty() { format!("{d} m") } else { format!("{desc} ({d} m)") };
        }
        out.waypoints.push(GpxWaypoint {
            name: get(name).to_string(),
            lat: la,
            lon: lo,
            desc,
            sym: get(sym).to_string(),
            time: parse_when(get(when)),
            colour: crate::gpx::parse_colour(get(colour)),
            group: get(group).to_string(),
            ..Default::default()
        });
    }
    let mut notes = Vec::new();
    if !bad.is_empty() {
        let shown: Vec<String> = bad.iter().take(10).map(|n| n.to_string()).collect();
        notes.push(format!("{} row{} without a position it could read (line{} {}{})", bad.len(), if bad.len() == 1 { "" } else { "s" }, if bad.len() == 1 { "" } else { "s" }, shown.join(", "), if bad.len() > 10 { "…" } else { "" }));
    }
    if when.is_some() {
        notes.push("dates without a time zone are taken as UTC".into());
    }
    Ok((out, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_as_written() {
        assert_eq!(parse_coord("-41.25", "NS"), Some(-41.25));
        assert!((parse_coord("41 15.000 S", "NS").unwrap() + 41.25).abs() < 1e-9);
        assert!((parse_coord("41°15.000'S", "NS").unwrap() + 41.25).abs() < 1e-9);
        assert!((parse_coord("174 30 00 E", "EW").unwrap() - 174.5).abs() < 1e-9);
        assert_eq!(parse_coord("41 75 S", "NS"), None);
        assert_eq!(parse_coord("174 E", "NS"), None);
    }

    #[test]
    fn csv_marks() {
        let t = "Name;Latitude;Longitude;Notes;Date;Depth\n\"Hole, deep\";41 15.000 S;174 30.000 E;bigeye;21/11/2024 05:30;350\nBad;x;y;;;\n";
        let (d, notes) = parse_csv(t).unwrap();
        assert_eq!(d.waypoints.len(), 1);
        let w = &d.waypoints[0];
        assert_eq!(w.name, "Hole, deep");
        assert!((w.lat + 41.25).abs() < 1e-9);
        assert_eq!(w.desc, "bigeye (350 m)");
        assert_eq!(w.time, parse_iso("2024-11-21T05:30:00Z"));
        assert!(notes[0].starts_with("1 row"));
    }

    #[test]
    fn kml_points_lines_polygons() {
        let k = r##"<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2"><Document>
<Style id="red"><IconStyle><color>ff0000ff</color></IconStyle></Style>
<Folder><name>Tuna</name>
<Placemark><name>Spot &amp; hole</name><description><![CDATA[<b>bigeye</b>]]></description><TimeStamp><when>2024-11-21T05:30:00Z</when></TimeStamp><styleUrl>#red</styleUrl><Point><coordinates>174.5,-41.25,0</coordinates></Point></Placemark>
<Placemark><name>Leg</name><LineString><coordinates>174,-41 174.1,-41.1 174.2,-41.2</coordinates></LineString></Placemark>
<Placemark><name>Closed</name><Polygon><outerBoundaryIs><LinearRing><coordinates>174,-41 174.1,-41 174.1,-41.1 174,-41</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark>
<Placemark><name>Logged</name><gx:Track><when>2024-11-21T05:30:00Z</when><when>2024-11-21T05:31:00Z</when><gx:coord>174 -41 0</gx:coord><gx:coord>174.01 -41.01 0</gx:coord></gx:Track></Placemark>
</Folder></Document></kml>"##;
        let (d, shapes) = parse_kml(k).unwrap();
        assert_eq!(d.waypoints.len(), 1);
        let w = &d.waypoints[0];
        assert_eq!(w.name, "Spot & hole");
        assert_eq!(w.desc, "bigeye");
        assert_eq!(w.colour, "#ff0000");
        assert_eq!(w.group, "Tuna");
        assert!(w.time.is_some());
        assert_eq!(d.routes.len(), 1);
        assert_eq!(d.routes[0].points.len(), 3);
        assert_eq!(d.tracks.len(), 1);
        assert_eq!(d.tracks[0].segments[0][1].t, parse_iso("2024-11-21T05:31:00Z"));
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].points.len(), 3);
    }
}
