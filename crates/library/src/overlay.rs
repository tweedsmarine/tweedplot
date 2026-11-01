//! Another library shown alongside, read-only: a mate's marks over yours without mixing
//! them in. It's opened read-only, so nothing in it can change.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::tracks::Track;
use crate::{mark_row, store, track_summary_row, Error, Mark, Result, TrackSummary, MARK_COLS, TRACK_COLS};

pub struct Overlay {
    path: PathBuf,
    db: Mutex<Connection>,
}

impl Overlay {
    /// Open a library file read-only. One made by an older TweedPlot must be opened (as
    /// the library in use) once first, to bring it up to date.
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        let v: Option<String> = c
            .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get(0))
            .optional()
            .map_err(|_| Error::Store("that isn't a TweedPlot library".into()))?;
        let v: i64 = v.and_then(|s| s.parse().ok()).unwrap_or(1);
        if v < 2 {
            return Err(Error::Store(
                "that library is from an older TweedPlot: open it once (Library menu > Open library) to update it, then show it alongside".into(),
            ));
        }
        Ok(Overlay { path: path.to_path_buf(), db: Mutex::new(c) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn name(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn marks(&self) -> Result<Vec<Mark>> {
        let c = self.db();
        let mut q = c.prepare(&format!("SELECT {MARK_COLS} FROM marks WHERE deleted_at IS NULL ORDER BY fid"))?;
        let v = q.query_map([], mark_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn tracks(&self) -> Result<Vec<TrackSummary>> {
        let c = self.db();
        let mut q = c.prepare(&format!("SELECT {TRACK_COLS} FROM tracks WHERE deleted_at IS NULL ORDER BY coalesce(start, modified) DESC"))?;
        let v = q.query_map([], track_summary_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn track(&self, id: &str) -> Result<Track> {
        let c = self.db();
        let s = c
            .query_row(&format!("SELECT {TRACK_COLS} FROM tracks WHERE id = ?1 AND deleted_at IS NULL"), [id], track_summary_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(id.into()))?;
        let data: Vec<u8> = c.query_row("SELECT data FROM track_data WHERE track_id = ?1", [id], |r| r.get(0)).optional()?.unwrap_or_default();
        let segments = if data.is_empty() { Vec::new() } else { store::decode_segments(&data)? };
        Ok(Track { id: s.id, name: s.name, notes: s.notes, colour: s.colour, source: s.source, modified: s.modified, stats: s.stats, segments })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Library, Mark};

    #[test]
    fn read_only_overlay() {
        let dir = std::env::temp_dir().join(format!("lib-ovl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).unwrap();
        lib.save_mark(Mark { name: "Mate's spot".into(), lat: -41.0, lon: 174.0, ..Default::default() }).unwrap();
        let o = super::Overlay::open(&lib.path()).unwrap();
        assert_eq!(o.marks().unwrap().len(), 1);
        assert!(o.tracks().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
