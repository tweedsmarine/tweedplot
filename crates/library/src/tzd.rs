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
