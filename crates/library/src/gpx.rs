//! GPX 1.0 / 1.1 reading (waypoints and routes) and writing helpers.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::{Error, Mark, Result};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpxWaypoint {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub desc: String,
    pub sym: String,
}

#[derive(Debug, Clone, Default)]
pub struct GpxRoute {
    pub name: String,
    pub points: Vec<GpxWaypoint>,
}

#[derive(Debug, Clone, Default)]
pub struct GpxData {
    pub waypoints: Vec<GpxWaypoint>,
    pub routes: Vec<GpxRoute>,
    /// Tracks are not imported (they are logs, not plans); counted for the report.
    pub tracks: usize,
}

/// Read waypoints and routes from GPX text.
pub fn parse_gpx(text: &str) -> Result<GpxData> {
    let mut rd = Reader::from_str(text);
    rd.config_mut().trim_text(false);
    let mut out = GpxData::default();
    let mut stack: Vec<String> = Vec::new();
    let mut point: Option<GpxWaypoint> = None;
    let mut route: Option<GpxRoute> = None;
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
            Event::Start(e) => {
                let n = local(e.name().into_inner());
                match n.as_str() {
                    "gpx" => saw_gpx = true,
                    "rte" => route = Some(GpxRoute::default()),
                    "trk" => out.tracks += 1,
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
    s.push_str(&format!("{indent}  <sym>{}</sym>\n{indent}</{tag}>\n", if m.kind == "mark" { "Buoy" } else { "Waypoint" }));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_waypoints_routes_and_skips_tracks() {
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
        assert_eq!(d.tracks, 1);
    }

    #[test]
    fn refuses_other_xml() {
        assert!(parse_gpx("<kml></kml>").is_err());
    }
}
