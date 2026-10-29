//! Managing the library as a whole: the Trash (bring back, delete for good), undoing an
//! import, and writing a chosen part of the library as a library file of its own.

use std::path::Path;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::{now, Library, Result};

/// Which categories a merge brings in (all by default).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    pub marks: bool,
    pub routes: bool,
    pub tracks: bool,
    pub events: bool,
    pub trips: bool,
    pub routings: bool,
    pub barriers: bool,
}

impl Default for Scope {
    fn default() -> Self {
        Scope { marks: true, routes: true, tracks: true, events: true, trips: true, routings: true, barriers: true }
    }
}

/// The categories an export keeps besides the chosen marks, routes, tracks and logs
/// (none by default).
#[derive(Debug, Clone, Copy, Default)]
pub struct Also {
    pub trips: bool,
    pub routings: bool,
    pub barriers: bool,
}

/// The tables that have a Trash, with how a row is named and described in it.
const KINDS: [(&str, &str, &str); 7] = [
    ("mark", "marks", "grp"),
    ("route", "routes", "''"),
    ("track", "tracks", "coalesce(start, modified)"),
    ("routing", "routings", "route_name"),
    ("barrier", "barriers", "kind"),
    ("trip", "trips", "fishery"),
    ("event", "events", "notes"),
];

fn table(kind: &str) -> Option<&'static str> {
    KINDS.iter().find(|k| k.0 == kind).map(|k| k.1)
}

/// Something in the Trash.
#[derive(Debug, Clone, Serialize)]
pub struct TrashItem {
    pub kind: String,
    pub id: String,
    pub name: String,
    /// A mark's group, a track's start (Unix seconds, as text), a routing's route.
    pub detail: String,
    pub deleted_at: i64,
}

/// An import still in the library.
#[derive(Debug, Clone, Serialize)]
pub struct ImportInfo {
    pub id: i64,
    pub file: String,
    pub kind: String,
    pub at: i64,
    /// What it brought in that is still here (not deleted).
    pub marks: usize,
    pub routes: usize,
    pub tracks: usize,
    pub barriers: usize,
}

/// How many of each an action took.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub marks: usize,
    pub routes: usize,
    pub tracks: usize,
    pub routings: usize,
    pub barriers: usize,
    pub events: usize,
    pub trips: usize,
}

fn fill_ids(c: &Connection, ids: &[String]) -> Result<()> {
    c.execute("CREATE TEMP TABLE IF NOT EXISTS pick (id TEXT PRIMARY KEY)", [])?;
    c.execute("DELETE FROM pick", [])?;
    let mut ins = c.prepare_cached("INSERT OR IGNORE INTO pick (id) VALUES (?1)")?;
    for id in ids {
        ins.execute([id])?;
    }
    Ok(())
}

impl Library {
    /// Everything in the Trash, newest deletion first.
    pub fn trash(&self) -> Result<Vec<TrashItem>> {
        let c = self.db();
        let mut v = Vec::new();
        for (kind, t, detail) in KINDS {
            let mut q = c.prepare(&format!("SELECT CAST(id AS TEXT), name, CAST({detail} AS TEXT), deleted_at FROM {t} WHERE deleted_at IS NOT NULL"))?;
            let rows = q.query_map([], |r| {
                Ok(TrashItem { kind: kind.into(), id: r.get(0)?, name: r.get(1)?, detail: r.get::<_, Option<String>>(2)?.unwrap_or_default(), deleted_at: r.get(3)? })
            })?;
            for x in rows {
                v.push(x?);
            }
        }
        v.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at).then_with(|| a.name.cmp(&b.name)));
        Ok(v)
    }

    /// Bring things back from the Trash. A route brings back its marks too.
    pub fn restore(&self, kind: &str, ids: &[String]) -> Result<usize> {
        let Some(t) = table(kind) else { return Ok(0) };
        let mut c = self.db();
        let tx = c.transaction()?;
        fill_ids(&tx, ids)?;
        if kind == "route" {
            tx.execute(
                "UPDATE marks SET deleted_at = NULL WHERE deleted_at IS NOT NULL AND id IN
                   (SELECT mark_id FROM route_marks WHERE route_id IN (SELECT id FROM pick)
                    UNION SELECT start_mark FROM routes WHERE id IN (SELECT id FROM pick))",
                [],
            )?;
        }
        let n = tx.execute(&format!("UPDATE {t} SET deleted_at = NULL WHERE deleted_at IS NOT NULL AND id IN (SELECT id FROM pick)"), [])?;
        tx.commit()?;
        Ok(n)
    }

    /// Delete things in the Trash for good (`ids` empty: empty the Trash of this kind).
    pub fn purge(&self, kind: &str, ids: &[String]) -> Result<usize> {
        let Some(t) = table(kind) else { return Ok(0) };
        let mut c = self.db();
        let tx = c.transaction()?;
        let cond = if ids.is_empty() {
            String::new()
        } else {
            fill_ids(&tx, ids)?;
            " AND id IN (SELECT id FROM pick)".into()
        };
        if kind == "route" {
            tx.execute(&format!("DELETE FROM route_marks WHERE route_id IN (SELECT id FROM routes WHERE deleted_at IS NOT NULL{cond})"), [])?;
        }
        if kind == "mark" {
            tx.execute(&format!("DELETE FROM route_marks WHERE mark_id IN (SELECT id FROM marks WHERE deleted_at IS NOT NULL{cond})"), [])?;
        }
        let n = tx.execute(&format!("DELETE FROM {t} WHERE deleted_at IS NOT NULL{cond}"), [])?;
        tx.commit()?;
        Ok(n)
    }

    /// The imports that still have something in the library, newest first.
    pub fn imports(&self) -> Result<Vec<ImportInfo>> {
        let c = self.db();
        let mut q = c.prepare(
            "SELECT i.id, i.file, i.kind, i.at,
               (SELECT count(*) FROM marks WHERE import_id = i.id AND deleted_at IS NULL),
               (SELECT count(*) FROM routes WHERE import_id = i.id AND deleted_at IS NULL),
               (SELECT count(*) FROM tracks WHERE import_id = i.id AND deleted_at IS NULL),
               (SELECT count(*) FROM barriers WHERE import_id = i.id AND deleted_at IS NULL)
             FROM imports i ORDER BY i.at DESC, i.id DESC",
        )?;
        let v = q
            .query_map([], |r| {
                Ok(ImportInfo {
                    id: r.get(0)?,
                    file: r.get(1)?,
                    kind: r.get(2)?,
                    at: r.get(3)?,
                    marks: r.get::<_, i64>(4)? as usize,
                    routes: r.get::<_, i64>(5)? as usize,
                    tracks: r.get::<_, i64>(6)? as usize,
                    barriers: r.get::<_, i64>(7)? as usize,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v.into_iter().filter(|i| i.marks + i.routes + i.tracks + i.barriers > 0).collect())
    }

    /// Take out (to the Trash) the marks, routes and tracks an import brought in. Marks that
    /// were already here and were only matched stay. Routes of your own that used the
    /// import's marks lose those marks; the import's own routes keep theirs, so bringing
    /// the import back from the Trash puts it back as it was.
    pub fn undo_import(&self, id: i64) -> Result<Counts> {
        let mut c = self.db();
        let tx = c.transaction()?;
        let t = now();
        let own_routes = "SELECT id FROM routes WHERE import_id IS NOT ?1";
        let gone = "SELECT id FROM marks WHERE import_id = ?1 AND deleted_at IS NULL";
        tx.execute(&format!("UPDATE routes SET start_mark = NULL, modified = ?2 WHERE start_mark IN ({gone}) AND id IN ({own_routes})"), params![id, t])?;
        tx.execute(&format!("DELETE FROM route_marks WHERE mark_id IN ({gone}) AND route_id IN ({own_routes})"), params![id])?;
        let mut n = Counts::default();
        n.marks = tx.execute("UPDATE marks SET deleted_at = ?2 WHERE import_id = ?1 AND deleted_at IS NULL", params![id, t])?;
        n.routes = tx.execute("UPDATE routes SET deleted_at = ?2 WHERE import_id = ?1 AND deleted_at IS NULL", params![id, t])?;
        n.tracks = tx.execute("UPDATE tracks SET deleted_at = ?2 WHERE import_id = ?1 AND deleted_at IS NULL", params![id, t])?;
        n.barriers = tx.execute("UPDATE barriers SET deleted_at = ?2 WHERE import_id = ?1 AND deleted_at IS NULL", params![id, t])?;
        n.events = tx.execute("UPDATE events SET deleted_at = ?2 WHERE import_id = ?1 AND deleted_at IS NULL", params![id, t])?;
        tx.commit()?;
        Ok(n)
    }

    /// What a library file holds, without opening it as a library (nothing in it changes):
    /// how many of each category, for previewing before a merge.
    pub fn peek(path: &Path) -> Result<Counts> {
        let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // a library made before a table existed (the log, trips) simply has none
        let count = |t: &str| c.query_row(&format!("SELECT count(*) FROM {t} WHERE deleted_at IS NULL"), [], |r| r.get::<_, i64>(0)).map(|n| n as usize).unwrap_or(0);
        Ok(Counts { marks: count("marks"), routes: count("routes"), tracks: count("tracks"), routings: count("routings"), barriers: count("barriers"), events: count("events"), trips: count("trips") })
    }

    /// Write the chosen marks, routes (with their marks), tracks and logs, and whole
    /// categories of trips, routings and barriers (`also`), as a library file of their own
    /// at `to`, e.g. to give another boat. The library in use is unchanged.
    pub fn export_library(&self, to: &Path, marks: &[String], routes: &[String], tracks: &[String], events: &[String], also: Also, keep_notes: bool) -> Result<Counts> {
        if to.canonicalize().ok().is_some_and(|p| Some(p) == self.path().canonicalize().ok()) {
            return Err(crate::Error::Store("that's the library in use: choose another file".into()));
        }
        let tmp = to.with_extension("gpkg.export");
        let _ = std::fs::remove_file(&tmp);
        self.backup_to(&tmp)?;
        let r = (|| -> Result<Counts> {
            let mut c = Connection::open(&tmp)?;
            let tx = c.transaction()?;
            for (name, ids) in [("keep_marks", marks), ("keep_routes", routes), ("keep_tracks", tracks), ("keep_events", events)] {
                tx.execute(&format!("CREATE TEMP TABLE {name} (id TEXT PRIMARY KEY)"), [])?;
                let mut ins = tx.prepare(&format!("INSERT OR IGNORE INTO {name} (id) VALUES (?1)"))?;
                for id in ids {
                    ins.execute([id])?;
                }
            }
            tx.execute("DELETE FROM routes WHERE deleted_at IS NOT NULL OR id NOT IN (SELECT id FROM keep_routes)", [])?;
            tx.execute("DELETE FROM route_marks WHERE route_id NOT IN (SELECT id FROM routes)", [])?;
            tx.execute(
                "DELETE FROM marks WHERE deleted_at IS NOT NULL OR (id NOT IN (SELECT id FROM keep_marks)
                   AND id NOT IN (SELECT mark_id FROM route_marks) AND id NOT IN (SELECT start_mark FROM routes WHERE start_mark IS NOT NULL))",
                [],
            )?;
            tx.execute("DELETE FROM tracks WHERE deleted_at IS NOT NULL OR id NOT IN (SELECT id FROM keep_tracks)", [])?;
            tx.execute("DELETE FROM events WHERE deleted_at IS NOT NULL OR id NOT IN (SELECT id FROM keep_events)", [])?;
            for (t, keep) in [("routings", also.routings), ("barriers", also.barriers), ("trips", also.trips)] {
                if keep {
                    tx.execute(&format!("DELETE FROM {t} WHERE deleted_at IS NOT NULL"), [])?;
                } else {
                    tx.execute(&format!("DELETE FROM {t}"), [])?;
                }
            }
            tx.execute("DELETE FROM imports", [])?;
            tx.execute("UPDATE barriers SET import_id = NULL", [])?;
            tx.execute("UPDATE events SET import_id = NULL", [])?;
            if !keep_notes {
                tx.execute("UPDATE events SET notes = ''", [])?;
                tx.execute("UPDATE trips SET notes = ''", [])?;
                tx.execute("UPDATE barriers SET notes = ''", [])?;
            }
            for t in ["marks", "routes", "tracks"] {
                tx.execute(&format!("UPDATE {t} SET import_id = NULL, trip_id = NULL"), [])?;
                if !keep_notes {
                    tx.execute(&format!("UPDATE {t} SET notes = ''"), [])?;
                }
            }
            let count = |t: &str| tx.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get::<_, i64>(0)).map(|n| n as usize);
            let n = Counts { marks: count("marks")?, routes: count("routes")?, tracks: count("tracks")?, routings: count("routings")?, barriers: count("barriers")?, events: count("events")?, trips: count("trips")? };
            tx.commit()?;
            c.execute_batch("VACUUM")?;
            Ok(n)
        })();
        match r {
            Ok(n) => {
                std::fs::rename(&tmp, to)?;
                Ok(n)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Also, Scope};
    use crate::{Library, Mark};

    fn mark(name: &str, lat: f64) -> Mark {
        Mark { name: name.into(), lat, lon: 174.0, kind: "mark".into(), ..Default::default() }
    }

    #[test]
    fn trash_restore_purge_undo_export() {
        let dir = std::env::temp_dir().join(format!("lib-manage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).unwrap();
        let a = lib.save_mark(mark("A", -36.0)).unwrap();
        let b = lib.save_mark(mark("B", -36.1)).unwrap();
        lib.delete_marks(&[a.id.clone(), b.id.clone()], false).unwrap();
        assert_eq!(lib.trash().unwrap().len(), 2);
        assert_eq!(lib.restore("mark", &[a.id.clone()]).unwrap(), 1);
        assert_eq!(lib.marks().unwrap().len(), 1);
        assert_eq!(lib.purge("mark", &[]).unwrap(), 1);
        assert!(lib.trash().unwrap().is_empty());

        // an import, undone and brought back
        lib.import_gpx(r#"<gpx><wpt lat="-35" lon="174"><name>X</name></wpt><wpt lat="-35.1" lon="174"><name>Y</name></wpt></gpx>"#, "t").unwrap();
        let im = lib.imports().unwrap();
        assert_eq!(im.len(), 1);
        assert_eq!(im[0].marks, 2);
        let n = lib.undo_import(im[0].id).unwrap();
        assert_eq!(n.marks, 2);
        assert_eq!(lib.marks().unwrap().len(), 1);
        assert!(lib.imports().unwrap().is_empty());
        let ids: Vec<String> = lib.trash().unwrap().into_iter().map(|t| t.id).collect();
        lib.restore("mark", &ids).unwrap();
        assert_eq!(lib.marks().unwrap().len(), 3);

        // part of it as a library of its own
        let keep: Vec<String> = lib.marks().unwrap().into_iter().filter(|m| m.name != "A").map(|m| m.id).collect();
        let out = dir.join("part.gpkg");
        let n = lib.export_library(&out, &keep, &[], &[], &[], Also::default(), true).unwrap();
        assert_eq!(n.marks, 2);
        let other = Library::open_file(&out).unwrap();
        assert_eq!(other.marks().unwrap().len(), 2);
        assert_eq!(lib.marks().unwrap().len(), 3);

        // categories as a whole, a peek into the file, and a merge of only some of it
        lib.save_barrier(crate::Barrier { name: "Cable".into(), points: vec![[174.0, -36.0], [174.1, -36.0]], ..Default::default() }).unwrap();
        lib.save_trip(crate::Trip { name: "Trip 1".into(), started: Some(1000), ended: Some(2000), ..Default::default() }).unwrap();
        lib.save_event(crate::Event { kind: "note".into(), name: "Note".into(), at: 1500, ..Default::default() }).unwrap();
        let all = dir.join("all.gpkg");
        let ev: Vec<String> = lib.events().unwrap().into_iter().map(|e| e.id).collect();
        lib.export_library(&all, &[], &[], &[], &ev, Also { trips: true, barriers: true, routings: false }, true).unwrap();
        let c = Library::peek(&all).unwrap();
        assert_eq!((c.marks, c.events, c.trips, c.barriers, c.routings), (0, 1, 1, 1, 0));
        let fresh = Library::open_file(dir.join("fresh.gpkg")).unwrap();
        let only = Scope { marks: false, routes: false, tracks: false, events: true, trips: false, routings: false, barriers: false };
        let rep = fresh.merge_from_scope(&all, only, &mut |_, _, _| {}, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        assert_eq!(rep.events_added, 1);
        assert_eq!((fresh.events().unwrap().len(), fresh.trips().unwrap().len(), fresh.barriers().unwrap().len()), (1, 0, 0));
        let rep = fresh.merge_from(&all, &mut |_, _, _| {}, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        assert_eq!((rep.events_added, rep.trips_added), (0, 1));
        assert_eq!((fresh.trips().unwrap().len(), fresh.barriers().unwrap().len()), (1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
