//! GPX 1.0 / 1.1 reading (waypoints and routes) and writing helpers.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::tracks::{parse_iso, TrackPoint};
use crate::{Error, Mark, Result};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpxWaypoint {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub desc: String,
    pub sym: String,
    /// `<time>`: when the mark was made (Unix seconds).
    pub time: Option<i64>,
    /// Colour from the file's extensions ("#rrggbb"): TimeZero's `<color>` (a signed ARGB number).
    pub colour: String,
    /// TimeZero's symbol number (`<MxTimeZeroSymbol>`).
    pub tz_symbol: Option<u32>,
    /// The group (TimeZero layer) it came from; "" means the import's own group.
    pub group: String,
}

/// A colour as written in GPX extensions: "#rrggbb", "rrggbb", or a signed 32-bit ARGB
/// number (.NET's Color.ToArgb, as TimeZero writes). "" if it isn't one.
pub fn parse_colour(v: &str) -> String {
    let v = v.trim();
    if let Ok(n) = v.parse::<i64>() {
        let x = (n as i32) as u32;
        return format!("#{:06x}", x & 0xff_ffff);
    }
    let h = v.trim_start_matches('#');
    if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!("#{}", h.to_ascii_lowercase());
    }
    String::new()
}

#[derive(Debug, Clone, Default)]
pub struct GpxRoute {
    pub name: String,
    pub points: Vec<GpxWaypoint>,
}

#[derive(Debug, Clone, Default)]
pub struct GpxTrack {
    pub name: String,
    pub desc: String,
    /// `<trkseg>` pieces; points with time (`<time>`), and speed and course when the file has them
    /// (`<speed>` m/s and `<course>` from GPX 1.0, or the same names inside `<extensions>`).
    pub segments: Vec<Vec<TrackPoint>>,
}

#[derive(Debug, Clone, Default)]
pub struct GpxData {
    pub waypoints: Vec<GpxWaypoint>,
    pub routes: Vec<GpxRoute>,
    pub tracks: Vec<GpxTrack>,
}

/// Read waypoints, routes and tracks from GPX text.
pub fn parse_gpx(text: &str) -> Result<GpxData> {
    let mut rd = Reader::from_str(text);
    rd.config_mut().trim_text(false);
    let mut out = GpxData::default();
    let mut stack: Vec<String> = Vec::new();
    let mut point: Option<GpxWaypoint> = None;
    let mut route: Option<GpxRoute> = None;
    let mut track: Option<GpxTrack> = None;
    let mut tp: Option<TrackPoint> = None;
    let mut text_buf = String::new();
    let mut saw_gpx = false;

    loop {
        let ev = rd.read_event().map_err(|e| Error::Gpx(e.to_string()))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) if matches!(local(e.name().into_inner()).as_str(), "wpt" | "rtept") => {
                let self_closing = matches!(ev, Event::Empty(_));
                let mut w = GpxWaypoint::default();
                for a in e.attributes().flatten() {
                    let v: &str = &a.value;
                    let v = v.trim().to_string();
                    match local(a.key.into_inner()).as_str() {
                        "lat" => w.lat = v.parse().map_err(|_| Error::Gpx(format!("bad lat {v}")))?,
                        "lon" => w.lon = v.parse().map_err(|_| Error::Gpx(format!("bad lon {v}")))?,
                        _ => {}
                    }
                }
                point = Some(w);
                stack.push(local(e.name().into_inner()));
                text_buf.clear();
                if self_closing {
                    finish_point(&mut stack, &mut point, &mut route, &mut out);
                }
            }
            Event::Start(ref e) | Event::Empty(ref e) if local(e.name().into_inner()) == "trkpt" => {
                let self_closing = matches!(ev, Event::Empty(_));
                let mut p = TrackPoint { t: None, lat: f64::NAN, lon: f64::NAN, ..Default::default() };
                for a in e.attributes().flatten() {
                    let v: &str = &a.value;
                    match local(a.key.into_inner()).as_str() {
                        "lat" => p.lat = v.trim().parse().unwrap_or(f64::NAN),
                        "lon" => p.lon = v.trim().parse().unwrap_or(f64::NAN),
                        _ => {}
                    }
                }
                if self_closing {
                    finish_trkpt(Some(p), &mut track);
                } else {
                    tp = Some(p);
                    stack.push("trkpt".into());
                }
                text_buf.clear();
            }
            Event::Start(e) => {
                let n = local(e.name().into_inner());
                match n.as_str() {
                    "gpx" => saw_gpx = true,
                    "rte" => route = Some(GpxRoute::default()),
                    "trk" => track = Some(GpxTrack::default()),
                    "trkseg" => {
                        if let Some(t) = track.as_mut() {
                            t.segments.push(Vec::new());
                        }
                    }
                    _ => {}
                }
                stack.push(n);
                text_buf.clear();
            }
            Event::Empty(_) => {}
            Event::Text(t) => {
                text_buf.push_str(&lossy(&*t));
            }
            Event::CData(t) => text_buf.push_str(&lossy(&*t)),
            Event::GeneralRef(r) => {
                let name = lossy(&*r); 
                text_buf.push_str(&entity(&name));
            }
            Event::End(e) => {
                let n = local(e.name().into_inner());
                let parent = stack.len().checked_sub(2).and_then(|i| stack.get(i)).cloned().unwrap_or_default();
                let value = text_buf.trim().to_string();
                match n.as_str() {
                    "wpt" | "rtept" => {
                        finish_point(&mut stack, &mut point, &mut route, &mut out);
                        text_buf.clear();
                        continue;
                    }
                    "rte" => {
                        if let Some(r) = route.take() {
                            out.routes.push(r);
                        }
                    }
                    "trkpt" => {
                        finish_trkpt(tp.take(), &mut track);
                    }
                    "trk" => {
                        if let Some(mut t) = track.take() {
                            t.segments.retain(|s| !s.is_empty());
                            if !t.segments.is_empty() {
                                out.tracks.push(t);
                            }
                        }
                    }
                    "time" | "speed" | "course" if tp.is_some() && stack.iter().any(|s| s == "trkpt") => {
                        let p = tp.as_mut().unwrap();
                        match n.as_str() {
                            "time" => p.t = parse_iso(&value),
                            // m/s to knots
                            "speed" => p.sog = value.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v * 3600.0 / 1852.0),
                            _ => p.cog = value.parse::<f64>().ok().filter(|v| v.is_finite()),
                        }
                    }
                    "time" | "MxTimeZeroSymbol" | "color" | "colour" if point.is_some() && stack.iter().any(|s| s == "wpt" || s == "rtept") => {
                        let w = point.as_mut().unwrap();
                        match n.as_str() {
                            "time" => w.time = parse_iso(&value),
                            "MxTimeZeroSymbol" => w.tz_symbol = value.trim().parse().ok(),
                            _ => {
                                let c = parse_colour(&value);
                                if !c.is_empty() {
                                    w.colour = c;
                                }
                            }
                        }
                    }
                    "name" | "desc" | "cmt" | "sym" => {
                        if parent == "wpt" || parent == "rtept" {
                            if let Some(w) = point.as_mut() {
                                match n.as_str() {
                                    "name" => w.name = value,
                                    "sym" => w.sym = value,
                                    _ => {
                                        if w.desc.is_empty() {
                                            w.desc = value
                                        }
                                    }
                                }
                            }
                        } else if parent == "rte" && n == "name" {
                            if let Some(r) = route.as_mut() {
                                r.name = value;
                            }
                        } else if parent == "trk" {
                            if let Some(t) = track.as_mut() {
                                match n.as_str() {
                                    "name" => t.name = value,
                                    "desc" | "cmt" if t.desc.is_empty() => t.desc = value,
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
                stack.pop();
                text_buf.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_gpx {
        return Err(Error::Gpx("not a GPX file".into()));
    }
    Ok(out)
}

fn finish_trkpt(p: Option<TrackPoint>, track: &mut Option<GpxTrack>) {
    let (Some(p), Some(t)) = (p, track.as_mut()) else { return };
    if !(p.lat.is_finite() && p.lon.is_finite()) {
        return;
    }
    if t.segments.is_empty() {
        t.segments.push(Vec::new());
    }
    t.segments.last_mut().unwrap().push(p);
}

fn finish_point(stack: &mut Vec<String>, point: &mut Option<GpxWaypoint>, route: &mut Option<GpxRoute>, out: &mut GpxData) {
    let kind = stack.pop().unwrap_or_default();
    if let Some(w) = point.take() {
        if kind == "rtept" {
            if let Some(r) = route.as_mut() {
                r.points.push(w);
            }
        } else {
            out.waypoints.push(w);
        }
    }
}

/// Element or attribute name without its namespace prefix.
fn local(name: impl AsRef<[u8]>) -> String {
    let s = String::from_utf8_lossy(name.as_ref());
    s.rsplit(':').next().unwrap_or("").to_string()
}

fn lossy(b: &str) -> String {
    b.to_string()
}

fn entity(name: &str) -> String {
    match name {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        _ => {
            let code = name.strip_prefix("#x").map(|h| u32::from_str_radix(h, 16).ok()).unwrap_or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()));
            code.and_then(char::from_u32).map(String::from).unwrap_or_default()
        }
    }
}

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// One `<wpt>` or `<rtept>` element.
pub(crate) fn wpt(tag: &str, m: &Mark, indent: &str) -> String {
    let mut s = format!("{indent}<{tag} lat=\"{:.6}\" lon=\"{:.6}\">\n{indent}  <name>{}</name>\n", m.lat, m.lon, esc(&m.name));
    if !m.notes.is_empty() {
        s.push_str(&format!("{indent}  <desc>{}</desc>\n", esc(&m.notes)));
    }
    if m.created > 0 {
        s.push_str(&format!("{indent}  <time>{}</time>\n", crate::tracks::iso_time(m.created)));
    }
    let sym = match m.symbol.as_str() {
        x if x.is_empty() || x.starts_with("tz:") => if m.kind == "mark" { "Buoy" } else { "Waypoint" },
        x => x,
    };
    s.push_str(&format!("{indent}  <sym>{}</sym>\n", esc(sym)));
    // TimeZero's own symbol and colour, so its marks go back to it as they came
    let tz = m.symbol.strip_prefix("tz:");
    if tz.is_some() || !m.colour.is_empty() {
        s.push_str(&format!("{indent}  <extensions>\n"));
        if let Some(n) = tz {
            s.push_str(&format!("{indent}    <MxTimeZeroSymbol>{n}</MxTimeZeroSymbol>\n"));
        }
        if let Some(rgb) = m.colour.strip_prefix('#').and_then(|h| u32::from_str_radix(h, 16).ok()) {
            s.push_str(&format!("{indent}    <color>{}</color>\n", (0xff00_0000u32 | rgb) as i32));
        }
        s.push_str(&format!("{indent}  </extensions>\n"));
    }
    s.push_str(&format!("{indent}</{tag}>\n"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_waypoints_routes_and_tracks() {
        let g = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="qtVlm" xmlns="http://www.topografix.com/GPX/1/1">
  <wpt lat="-36.8317" lon="174.8303"><name>Bean Rock</name><sym>Buoy</sym></wpt>
  <wpt lat="-36.5" lon="175.0"/>
  <rte><name>RTB &amp; back</name>
    <rtept lat="-36.8" lon="174.8"><name>A</name></rtept>
    <rtept lat="-36.7" lon="174.9"><name><![CDATA[B <x>]]></name></rtept>
  </rte>
  <trk><name>log</name><trkseg><trkpt lat="1" lon="2"/></trkseg></trk>
</gpx>"#;
        let d = parse_gpx(g).unwrap();
        assert_eq!(d.waypoints.len(), 2);
        assert_eq!(d.waypoints[0].name, "Bean Rock");
        assert_eq!(d.waypoints[0].sym, "Buoy");
        assert_eq!(d.waypoints[1].lat, -36.5);
        assert_eq!(d.routes.len(), 1);
        assert_eq!(d.routes[0].name, "RTB & back");
        assert_eq!(d.routes[0].points[1].name, "B <x>");
        assert_eq!(d.tracks.len(), 1);
        assert_eq!(d.tracks[0].name, "log");
        assert_eq!((d.tracks[0].segments[0][0].lat, d.tracks[0].segments[0][0].lon), (1.0, 2.0));
    }

    #[test]
    fn reads_track_times_speeds_and_segments() {
        // GPX 1.0 speed/course, Garmin-style extensions, and two segments
        let g = r#"<?xml version="1.0"?>
<gpx version="1.0" creator="OpenCPN">
  <trk><name>Coastal Classic</name><desc>Morgane</desc>
    <trkseg>
      <trkpt lat="-36.84" lon="174.77"><time>2025-10-24T00:00:00Z</time><course>20.5</course><speed>3.0866</speed></trkpt>
      <trkpt lat="-36.83" lon="174.78"><ele>0</ele><time>2025-10-24T13:01:00+13:00</time><extensions><gpxtpx:TrackPointExtension xmlns:gpxtpx="x"><gpxtpx:speed>2.5</gpxtpx:speed><gpxtpx:course>25</gpxtpx:course></gpxtpx:TrackPointExtension></extensions></trkpt>
    </trkseg>
    <trkseg><trkpt lat="-36.5" lon="174.9"/></trkseg>
    <trkseg></trkseg>
  </trk>
</gpx>"#;
        let d = parse_gpx(g).unwrap();
        let t = &d.tracks[0];
        assert_eq!((t.name.as_str(), t.desc.as_str()), ("Coastal Classic", "Morgane"));
        assert_eq!(t.segments.len(), 2);
        let a = t.segments[0][0];
        assert_eq!(a.t, Some(1761264000));
        assert!((a.sog.unwrap() - 6.0).abs() < 0.001);
        assert_eq!(a.cog, Some(20.5));
        let b = t.segments[0][1];
        assert_eq!(b.t, Some(1761264060));
        assert!((b.sog.unwrap() - 4.86).abs() < 0.01);
        assert_eq!(b.cog, Some(25.0));
        assert_eq!(t.segments[1][0].t, None);
    }

    #[test]
    fn refuses_other_xml() {
        assert!(parse_gpx("<kml></kml>").is_err());
    }
}
