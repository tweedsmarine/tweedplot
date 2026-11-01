//! Trips: a time out from port (or a fishing trip) with a name or number, fishery and crew.
//! What belongs to a trip is what was made while it lasted: marks made between its start
//! and end, and tracks that ran in that time. So a trip can be started now (and the
//! recorder with it), or added afterwards for a past trip from its dates, and years of
//! imported marks and tracks fall into the trips they were made in.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{now, Library, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Trip {
    /// 0 for a new trip.
    pub id: i64,
    pub name: String,
    pub number: String,
    pub fishery: String,
    pub crew: String,
    pub notes: String,
    /// Unix seconds.
    pub started: Option<i64>,
    /// None while the trip is on.
    pub ended: Option<i64>,
    // the summary (not saved): what was made in its time
    pub marks: usize,
    pub tracks: usize,
    pub distance_nm: f64,
    pub max_sog: Option<f64>,
    /// Fishing log events in it, and how many of them were shots or sets.
    pub events: usize,
    pub sets: usize,
}

/// The trip's time as SQL bounds: an open trip runs to now.
const TRIP_SELECT: &str = "SELECT t.id, t.name, t.number, t.fishery, t.crew, t.notes, t.started, t.ended,
    (SELECT count(*) FROM marks m WHERE m.deleted_at IS NULL AND m.date_unknown = 0 AND m.created BETWEEN t.started AND coalesce(t.ended, ?1)),
    (SELECT count(*) FROM tracks k WHERE k.deleted_at IS NULL AND k.start <= coalesce(t.ended, ?1) AND coalesce(k.\"end\", k.start) >= t.started),
    (SELECT coalesce(sum(distance_nm), 0) FROM tracks k WHERE k.deleted_at IS NULL AND k.start <= coalesce(t.ended, ?1) AND coalesce(k.\"end\", k.start) >= t.started),
    (SELECT max(max_sog) FROM tracks k WHERE k.deleted_at IS NULL AND k.start <= coalesce(t.ended, ?1) AND coalesce(k.\"end\", k.start) >= t.started),
    (SELECT count(*) FROM events e WHERE e.deleted_at IS NULL AND e.at BETWEEN t.started AND coalesce(t.ended, ?1)),
    (SELECT count(*) FROM events e WHERE e.deleted_at IS NULL AND e.kind IN ('net-shot', 'line-set', 'pot-set') AND e.at BETWEEN t.started AND coalesce(t.ended, ?1))
  FROM trips t WHERE t.deleted_at IS NULL";

fn trip_row(r: &rusqlite::Row) -> rusqlite::Result<Trip> {
    Ok(Trip {
        id: r.get(0)?,
        name: r.get(1)?,
        number: r.get(2)?,
        fishery: r.get(3)?,
        crew: r.get(4)?,
        notes: r.get(5)?,
        started: r.get(6)?,
        ended: r.get(7)?,
        marks: r.get::<_, i64>(8)? as usize,
        tracks: r.get::<_, i64>(9)? as usize,
        distance_nm: r.get(10)?,
        max_sog: r.get(11)?,
        events: r.get::<_, i64>(12)? as usize,
        sets: r.get::<_, i64>(13)? as usize,
    })
}

impl Library {
    /// Every trip, newest first, with its summary.
    pub fn trips(&self) -> Result<Vec<Trip>> {
        let c = self.db();
        let mut q = c.prepare(&format!("{TRIP_SELECT} ORDER BY t.started DESC"))?;
        let v = q.query_map([now()], trip_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn trip(&self, id: i64) -> Result<Trip> {
        let c = self.db();
        c.query_row(&format!("{TRIP_SELECT} AND t.id = ?2"), params![now(), id], trip_row)
            .optional()?
            .ok_or_else(|| crate::Error::NotFound(format!("trip {id}")))
    }

    /// The trip on now (started, not ended), if any.
    pub fn current_trip(&self) -> Result<Option<Trip>> {
        let id: Option<i64> = self
            .db()
            .query_row("SELECT id FROM trips WHERE deleted_at IS NULL AND ended IS NULL ORDER BY started DESC LIMIT 1", [], |r| r.get(0))
            .optional()?;
        id.map(|id| self.trip(id)).transpose()
    }

    /// Save a trip (new when its id is 0). A trip with no start starts now. Only one trip
    /// can be on at a time; the end can't be before the start.
    pub fn save_trip(&self, mut t: Trip) -> Result<Trip> {
        let started = t.started.unwrap_or_else(now);
        if t.ended.is_some_and(|e| e < started) {
            return Err(crate::Error::Store("the trip can't end before it starts".into()));
        }
        if t.ended.is_none() {
            if let Some(on) = self.current_trip()? {
                if on.id != t.id {
                    return Err(crate::Error::Store(format!("{} is still on: end it first", if on.name.is_empty() { "a trip" } else { on.name.as_str() })));
                }
            }
        }
        t.started = Some(started);
        if t.name.trim().is_empty() {
            let n: i64 = self.db().query_row("SELECT count(*) FROM trips", [], |r| r.get(0))?;
            t.name = if t.number.trim().is_empty() { format!("Trip {}", n + 1) } else { format!("Trip {}", t.number.trim()) };
        }
        {
            let c = self.db();
            if t.id == 0 {
                c.execute(
                    "INSERT INTO trips (name, number, fishery, crew, notes, started, ended) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![t.name.trim(), t.number.trim(), t.fishery.trim(), t.crew.trim(), t.notes, t.started, t.ended],
                )?;
                t.id = c.last_insert_rowid();
            } else {
                c.execute(
                    "UPDATE trips SET name = ?2, number = ?3, fishery = ?4, crew = ?5, notes = ?6, started = ?7, ended = ?8 WHERE id = ?1 AND deleted_at IS NULL",
                    params![t.id, t.name.trim(), t.number.trim(), t.fishery.trim(), t.crew.trim(), t.notes, t.started, t.ended],
                )?;
            }
        }
        self.trip(t.id)
    }

    /// End the trip on now (at `at`, or now).
    pub fn end_trip(&self, id: i64, at: Option<i64>) -> Result<Trip> {
        let mut t = self.trip(id)?;
        t.ended = Some(at.unwrap_or_else(now).max(t.started.unwrap_or(0)));
        self.save_trip(t)
    }

    /// Delete trips (to the Trash). Their marks and tracks stay.
    pub fn delete_trips(&self, ids: &[i64]) -> Result<usize> {
        let c = self.db();
        let mut n = 0;
        for id in ids {
            n += c.execute("UPDATE trips SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL", params![id, now()])?;
        }
        Ok(n)
    }

    /// The ids of the marks and tracks made in a trip (for showing, exporting).
    pub fn trip_events(&self, id: i64) -> Result<Vec<String>> {
        let t = self.trip(id)?;
        let c = self.db();
        let mut q = c.prepare("SELECT id FROM events WHERE deleted_at IS NULL AND at BETWEEN ?1 AND ?2")?;
        let v = q.query_map([t.started.unwrap_or(0), t.ended.unwrap_or_else(now)], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    pub fn trip_contents(&self, id: i64) -> Result<(Vec<String>, Vec<String>)> {
        let t = self.trip(id)?;
        let (a, b) = (t.started.unwrap_or(0), t.ended.unwrap_or_else(now));
        let c = self.db();
        let mut q = c.prepare("SELECT id FROM marks WHERE deleted_at IS NULL AND date_unknown = 0 AND created BETWEEN ?1 AND ?2")?;
        let marks = q.query_map([a, b], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        let mut q = c.prepare("SELECT id FROM tracks WHERE deleted_at IS NULL AND start <= ?2 AND coalesce(\"end\", start) >= ?1")?;
        let tracks = q.query_map([a, b], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok((marks, tracks))
    }
}

#[cfg(test)]
mod tests {
    use super::Trip;
    use crate::{Library, Mark};

    #[test]
    fn trips() {
        let dir = std::env::temp_dir().join(format!("lib-trips-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).unwrap();
        let mut m = Mark { name: "Bigeye".into(), lat: -40.0, lon: 174.0, ..Default::default() };
        m.created = 1_000_500;
        lib.save_mark(m).unwrap();
        // a past trip, from its dates
        let t = lib
            .save_trip(Trip { number: "12".into(), fishery: "Tuna".into(), started: Some(1_000_000), ended: Some(1_001_000), ..Default::default() })
            .unwrap();
        assert_eq!(t.name, "Trip 12");
        assert_eq!(t.marks, 1);
        assert_eq!(lib.trip_contents(t.id).unwrap().0.len(), 1);
        // one on now; a second can't start
        let on = lib.save_trip(Trip::default()).unwrap();
        assert!(on.ended.is_none());
        assert_eq!(lib.current_trip().unwrap().unwrap().id, on.id);
        assert!(lib.save_trip(Trip::default()).is_err());
        let done = lib.end_trip(on.id, None).unwrap();
        assert!(done.ended.is_some());
        assert!(lib.current_trip().unwrap().is_none());
        assert!(lib.save_trip(Trip { started: Some(10), ended: Some(5), ..Default::default() }).is_err());
        assert_eq!(lib.delete_trips(&[t.id]).unwrap(), 1);
        assert_eq!(lib.trips().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
