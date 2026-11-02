//! Finding ENC cells and their updates in a folder.

use std::path::{Path, PathBuf};

/// A base cell and its updates, in order.
pub struct CellFiles {
    pub base: PathBuf,
    pub updates: Vec<PathBuf>,
    /// The newest time any of its files was changed (from the folder listing).
    pub newest: Option<std::time::SystemTime>,
}

/// Every ENC base cell under `dir` (not in hidden folders), with its updates.
pub fn find_cells(dir: &Path) -> Vec<CellFiles> {
    find_cells_except(dir, &[])
}

/// [`find_cells`], not looking in the folders `off` at all (chart sets not in use).
pub fn find_cells_except(dir: &Path, off: &[PathBuf]) -> Vec<CellFiles> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> = Vec::new();
        // file times from the listing (Windows gives them with it: no extra disk query)
        let mut times: std::collections::HashMap<PathBuf, std::time::SystemTime> = std::collections::HashMap::new();
        for e in rd.flatten() {
            let p = e.path();
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            // the type from the directory listing itself: asking the disk about every file
            // again (is_dir) is slow on Windows with tens of thousands of cells
            let is_dir = e.file_type().map(|t| t.is_dir() || (t.is_symlink() && p.is_dir())).unwrap_or(false);
            if is_dir {
                if !hidden && !off.iter().any(|o| o == &p) {
                    stack.push(p);
                }
            } else {
                if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    times.insert(p.clone(), t);
                }
                files.push(p);
            }
        }
        let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        // S-63 signature files share the cell's name with the usage digit as a letter I-N
        let is_signature = |p: &Path| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            n.chars().nth(2).is_some_and(|c| ('I'..='N').contains(&c))
                && std::fs::read(p).is_ok_and(|b| b.starts_with(b"//") || b.starts_with(b"\xef\xbb\xbf//"))
        };
        for b in files.iter().filter(|p| ext(p) == "000" && !is_signature(p)) {
            let stem = b.file_stem().map(|s| s.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            let mut updates: Vec<PathBuf> = files
                .iter()
                .filter(|p| {
                    let e = ext(p);
                    e.len() == 3
                        && e != "000"
                        && e.bytes().all(|c| c.is_ascii_digit())
                        && p.file_stem().is_some_and(|s| s.to_string_lossy().to_ascii_uppercase() == stem)
                })
                .cloned()
                .collect();
            // an exchange set's layout, ENC_ROOT/<agency>/<cell>/<edition>/<update>/: the
            // updates are in the edition folder's other update folders
            let numeric = |p: Option<&Path>| p.and_then(|p| p.file_name()).is_some_and(|n| n.to_string_lossy().bytes().all(|c| c.is_ascii_digit()));
            if numeric(b.parent()) && numeric(b.parent().and_then(|p| p.parent())) {
                let ed = b.parent().unwrap().parent().unwrap();
                for e in std::fs::read_dir(ed).into_iter().flatten().flatten() {
                    let d = e.path();
                    if !d.is_dir() || Some(d.as_path()) == b.parent() || !numeric(Some(&d)) {
                        continue;
                    }
                    for f in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                        let p = f.path();
                        let e = ext(&p);
                        if e.len() == 3
                            && e != "000"
                            && e.bytes().all(|c| c.is_ascii_digit())
                            && p.file_stem().is_some_and(|s| s.to_string_lossy().to_ascii_uppercase() == stem)
                        {
                            if let Ok(t) = f.metadata().and_then(|m| m.modified()) {
                                times.insert(p.clone(), t);
                            }
                            updates.push(p);
                        }
                    }
                }
            }
            updates.sort_by_key(|p| ext(p));
            updates.dedup_by_key(|p| ext(p));
            let newest = std::iter::once(b)
                .chain(&updates)
                .map(|p| times.get(p).copied().or_else(|| std::fs::metadata(p).and_then(|m| m.modified()).ok()))
                .max()
                .flatten();
            out.push(CellFiles { base: b.clone(), updates, newest });
        }
    }
    out.sort_by(|a, b| a.base.cmp(&b.base));
    out
}
