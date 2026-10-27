//! The fishing log: events saved with one press (net shot and hauled, longline set and
//! hauled, pots set and hauled, catch, a note), each with the time, position and what the
//! instruments said then. A haul is paired with the shot (or set) it ends, so a tow's time
//! and distance can be shown. Events belong to the trip they were made in (by time).

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{new_id, now, Library, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Event {
    pub id: String,
    /// net-shot, net-haul, line-set, line-haul, pot-set, pot-haul, catch, note, timer
    pub kind: String,
    /// What it's called in lists ("Net shot").
    pub name: String,
    /// Unix seconds.
    pub at: i64,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub depth: Option<f64>,
    pub temp: Option<f64>,
    pub sog: Option<f64>,
    pub cog: Option<f64>,
    pub notes: String,
    /// A haul: the shot or set it ends.
    pub pair: String,
    /// The event's own details, e.g. a line set's radio buoys ({"buoy_start": "A12",
    /// "buoy_end": "B7"}), baskets, snood spacing; a radio buoy's name.
    pub data: serde_json::Value,
}

/// The kind a haul ends.
/// Kinds: net-shot, net-haul, line-set, line-end (the set finished), line-haul, pot-set,
/// pot-haul, float (a float snapped on during a set), radio-buoy, catch, note.
pub fn starts(kind: &str) -> Option<&'static str> {
    match kind {
        "net-haul" => Some("net-shot"),
        "line-haul" => Some("line-set"),
        "pot-haul" => Some("pot-set"),
        _ => None,
    }
}

const COLS: &str = "id, kind, name, at, lat, lon, depth, temp, sog, cog, notes, pair, data";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        at: r.get(3)?,
        lat: r.get(4)?,
        lon: r.get(5)?,
        depth: r.get(6)?,
        temp: r.get(7)?,
        sog: r.get(8)?,
        cog: r.get(9)?,
        notes: r.get(10)?,
        pair: r.get(11)?,
        data: serde_json::from_str(&r.get::<_, String>(12)?).unwrap_or(serde_json::Value::Null),
    })
}

impl Library {
    /// Every event, newest first.
    pub fn events(&self) -> Result<Vec<Event>> {
        let c = self.db();
        let mut q = c.prepare(&format!("SELECT {COLS} FROM events WHERE deleted_at IS NULL ORDER BY at DESC"))?;
        let v = q.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    /// Save an event (new when its id is empty; now when it has no time). A new haul is
    /// paired with the latest shot (or set) of the same gear not hauled yet.
    pub fn save_event(&self, mut e: Event) -> Result<Event> {
        let fresh = e.id.is_empty();
        if fresh {
            e.id = new_id("e");
        }
        if e.at == 0 {
            e.at = now();
        }
        let c = self.db();
        if fresh && e.pair.is_empty() {
            if let Some(start) = starts(&e.kind) {
                e.pair = c
                    .query_row(
                        "SELECT id FROM events s WHERE s.kind = ?1 AND s.deleted_at IS NULL AND s.at <= ?2
                           AND NOT EXISTS (SELECT 1 FROM events h WHERE h.pair = s.id AND h.deleted_at IS NULL)
                         ORDER BY s.at DESC LIMIT 1",
                        params![start, e.at],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
            }
        }
        c.execute(
            "INSERT INTO events (id, kind, name, at, lat, lon, depth, temp, sog, cog, notes, pair, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, name = excluded.name, at = excluded.at, lat = excluded.lat, lon = excluded.lon,
               depth = excluded.depth, temp = excluded.temp, sog = excluded.sog, cog = excluded.cog, notes = excluded.notes, pair = excluded.pair,
               data = excluded.data, deleted_at = NULL",
            params![e.id, e.kind, e.name, e.at, e.lat, e.lon, e.depth, e.temp, e.sog, e.cog, e.notes, e.pair, if e.data.is_null() { "{}".to_string() } else { e.data.to_string() }],
        )?;
        Ok(e)
    }

    /// Delete events (to the Trash).
    pub fn delete_events(&self, ids: &[String]) -> Result<usize> {
        let c = self.db();
        let t = now();
        let mut n = 0;
        for id in ids {
            n += c.execute("UPDATE events SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, t])?;
        }
        Ok(n)
    }

    /// Events as CSV (for a spreadsheet or an e-logbook), written to the export folder.
    pub fn export_events_csv(&self, name: &str, ids: &[String]) -> Result<std::path::PathBuf> {
        let want: std::collections::HashSet<&String> = ids.iter().collect();
        let mut v: Vec<Event> = self.events()?.into_iter().filter(|e| want.contains(&e.id)).collect();
        v.sort_by_key(|e| e.at);
        let by_id: std::collections::HashMap<String, Event> = v.iter().map(|e| (e.id.clone(), e.clone())).collect();
        let q = |s: &str| if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
        let f = |x: Option<f64>, dp: usize| x.map(|v| format!("{v:.dp$}")).unwrap_or_default();
        let mut out = String::from("time_utc,event,latitude,longitude,depth_m,sea_temp_c,sog_kn,cog_t,started_utc,notes\n");
        for e in &v {
            let start = by_id.get(&e.pair).map(|s| crate::tracks::iso_time(s.at)).unwrap_or_default();
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{}\n",
                crate::tracks::iso_time(e.at),
                q(&e.name),
                f(e.lat, 6),
                f(e.lon, 6),
                f(e.depth, 1),
                f(e.temp, 1),
                f(e.sog, 1),
                f(e.cog, 0),
                start,
                q(&e.notes)
            ));
        }
        let mut base = crate::safe(name);
        if base.is_empty() {
            base = "events".into();
        }
        let p = self.dir().join("export").join(format!("{base}.csv"));
        std::fs::write(&p, out)?;
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::Event;
    use crate::Library;

    #[test]
    fn shot_and_haul_pair() {
        let dir = std::env::temp_dir().join(format!("lib-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).unwrap();
        let shot = lib.save_event(Event { kind: "net-shot".into(), name: "Net shot".into(), at: 1000, lat: Some(-41.0), lon: Some(174.0), ..Default::default() }).unwrap();
        let haul = lib.save_event(Event { kind: "net-haul".into(), name: "Net hauled".into(), at: 5000, ..Default::default() }).unwrap();
        assert_eq!(haul.pair, shot.id);
        // a second haul has no open shot
        let h2 = lib.save_event(Event { kind: "net-haul".into(), name: "Net hauled".into(), at: 6000, ..Default::default() }).unwrap();
        assert!(h2.pair.is_empty());
        assert_eq!(lib.events().unwrap().len(), 3);
        let p = lib.export_events_csv("log", &[shot.id.clone(), haul.id.clone()]).unwrap();
        let t = std::fs::read_to_string(p).unwrap();
        assert!(t.lines().nth(2).unwrap().contains("Net hauled"));
        assert_eq!(lib.delete_events(&[h2.id]).unwrap(), 1);
        assert_eq!(lib.events().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
