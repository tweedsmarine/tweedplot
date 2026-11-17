//! The text and picture files that go with ENC cells (S-57 3.1, Appendix B.1 5.4):
//! a feature's TXTDSC / NTXTDS names a text file and PICREP a picture, kept beside the
//! cell in its exchange set (`ENC_ROOT/<agency>/<cell>/<edition>/<update>/NZPUB271.TXT`).
//! They are never encrypted, even in S-63 sets, so they can be read without permits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The kinds of file a feature can refer to.
fn is_ancillary(p: &Path) -> bool {
    p.extension().is_some_and(|e| {
        let e = e.to_string_lossy().to_ascii_uppercase();
        e == "TXT" || e == "TIF" || e == "TIFF" || e == "PNG" || e == "JPG" || e == "JPEG" || e == "GIF"
    })
}

/// A cell file: `.000` base or a three-digit update.
fn is_cell_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.len() == 3 && e.to_string_lossy().bytes().all(|c| c.is_ascii_digit()))
}

/// Every text and picture file kept with a cell under `dir`: those in a folder that also
/// holds a cell or update file (an exchange set's README.TXT, PRODUCTS.TXT and the like
/// aren't). Name (upper case) → the file, the newest copy where an update brings one again:
/// a later edition, then a later update, wins.
pub fn find_ancillary(dir: &Path) -> HashMap<String, PathBuf> {
    // (edition, update) from an exchange set's numbered folders, (0, 0) otherwise
    let order = |p: &Path| -> (u32, u32) {
        let num = |q: Option<&Path>| q.and_then(|q| q.file_name()).and_then(|n| n.to_string_lossy().parse::<u32>().ok());
        let up = p.parent();
        match (num(up.and_then(|u| u.parent())), num(up)) {
            (Some(e), Some(u)) => (e, u),
            _ => (0, 0),
        }
    };
    let mut found: HashMap<String, (u32, u32, PathBuf)> = HashMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                    stack.push(p);
                }
            } else {
                files.push(p);
            }
        }
        if !files.iter().any(|p| is_cell_file(p)) {
            continue;
        }
        for p in files.into_iter().filter(|p| is_ancillary(p)) {
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()) else { continue };
            let (e, u) = order(&p);
            if found.get(&name).is_none_or(|(fe, fu, _)| (e, u) >= (*fe, *fu)) {
                found.insert(name, (e, u, p));
            }
        }
    }
    found.into_iter().map(|(n, (_, _, p))| (n, p)).collect()
}

/// Copy every text and picture file kept with the cells under `from` into `to` (one flat
/// folder, by name; a copy already there is replaced). How many texts and pictures.
pub fn copy_ancillary(from: &Path, to: &Path) -> std::io::Result<(usize, usize)> {
    let found = find_ancillary(from);
    if found.is_empty() {
        return Ok((0, 0));
    }
    std::fs::create_dir_all(to)?;
    let (mut texts, mut pictures) = (0, 0);
    for (name, p) in found {
        std::fs::copy(&p, to.join(&name))?;
        if name.ends_with(".TXT") {
            texts += 1;
        } else {
            pictures += 1;
        }
    }
    Ok((texts, pictures))
}

/// A text file's contents. TXTDSC files are ASCII or ISO 8859-1 (lexical level 1);
/// NTXTDS ones may be UCS-2 (level 2), which starts with a byte-order mark. Line ends
/// become `\n`.
pub fn decode_text(b: &[u8]) -> String {
    let s = if b.len() >= 2 && (b[0..2] == [0xFF, 0xFE] || b[0..2] == [0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) }).collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = b.strip_prefix(b"\xEF\xBB\xBF") {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        match std::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            // ISO 8859-1: each byte is its own code point
            Err(_) => b.iter().map(|&c| c as char).collect(),
        }
    };
    s.replace("\r\n", "\n").replace('\r', "\n").trim_end_matches(['\n', '\u{1a}', '\0']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_newest_copy_beside_cells() {
        let d = std::env::temp_dir().join(format!("enc-texts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let cell = d.join("ENC_ROOT/NZ/NZ14600E/3");
        std::fs::create_dir_all(cell.join("0")).unwrap();
        std::fs::create_dir_all(cell.join("1")).unwrap();
        std::fs::write(cell.join("0/NZ14600E.000"), b"x").unwrap();
        std::fs::write(cell.join("0/NZCAU070.TXT"), b"old\r\n").unwrap();
        std::fs::write(cell.join("0/NZPUB271.TXT"), b"notes").unwrap();
        std::fs::write(cell.join("1/NZ14600E.001"), b"x").unwrap();
        std::fs::write(cell.join("1/NZCAU070.TXT"), b"new\r\n").unwrap();
        std::fs::write(d.join("ENC_ROOT/README.TXT"), b"readme").unwrap();
        let f = find_ancillary(&d);
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(f["NZCAU070.TXT"].ends_with("1/NZCAU070.TXT"));
        let to = d.join("out");
        assert_eq!(copy_ancillary(&d, &to).unwrap(), (2, 0));
        assert_eq!(decode_text(&std::fs::read(to.join("NZCAU070.TXT")).unwrap()), "new");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn decodes_text() {
        assert_eq!(decode_text(b"A\r\nB\r\n"), "A\nB");
        assert_eq!(decode_text(b"caf\xe9"), "café");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'H', 0, b'i', 0]), "Hi");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'H', 0, b'i']), "H");
        assert_eq!(decode_text(&[0xFE, 0xFF, 0, b'H', 0, b'i', 0]), "Hi");
    }
}
