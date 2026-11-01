//! Check an S-63 exchange set: `s63_check <folder> <SA public key file> [user permit] [M_KEY]`
//! (default: the IHO's generic S-64 test credentials). Finds PERMIT.TXT, checks each
//! permit, then for each cell file checks its signature, decrypts it and reads it.
use chart::enc::s63;
use std::path::{Path, PathBuf};

fn walk(d: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out)
        } else {
            out.push(p)
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(&a[0]);
    let sa = s63::parse_public_key(&std::fs::read(&a[1]).unwrap()).expect("SA key");
    let up = a.get(2).map(String::as_str).unwrap_or("66B5CBFDF7E4139D5B6086C23130");
    let mk = a.get(3).map(String::as_str).unwrap_or("10121");
    let hw = s63::hw_id_from_user_permit(up, mk).expect("user permit");
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();
    let mut permits = Vec::new();
    for p in files.iter().filter(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("PERMIT.TXT"))) {
        match s63::read_permits(&std::fs::read_to_string(p).unwrap_or_default(), &hw) {
            Ok(v) => {
                for (cell, r) in v {
                    match r {
                        Ok(pm) => {
                            println!("permit {cell}: ok, expires {}", pm.expiry);
                            permits.push(pm)
                        }
                        Err(e) => println!("permit {cell}: {e}"),
                    }
                }
            }
            Err(e) => println!("{}: {e}", p.display()),
        }
    }
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let ext = f.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        if ext.len() != 3 || !ext.bytes().all(|c| c.is_ascii_digit()) || name.chars().nth(2).is_none_or(|c| !c.is_ascii_digit()) {
            continue;
        }
        let rel = f.strip_prefix(&dir).unwrap().display().to_string();
        let data = std::fs::read(f).unwrap();
        let sig = s63::signature_name(&name).map(|s| f.with_file_name(s));
        let auth = match sig.as_ref().and_then(|s| std::fs::read(s).ok()) {
            Some(s) => match s63::verify_cell(&s, &data, &sa) {
                Ok(()) => "signature ok".to_string(),
                Err(e) => e.to_string(),
            },
            None => "no signature file".into(),
        };
        let stem = &name[..8];
        let dec = match permits.iter().find(|p| p.cell == stem) {
            Some(p) => match s63::decrypt_cell(&data, p) {
                Ok(plain) => match chart::enc::cell::Cell::from_bytes(&plain, &[]) {
                    Ok(c) => format!("decrypted: {} ed {} upd {}, {} features", c.name, c.edition, c.update, c.features.len()),
                    Err(e) => format!("decrypted but unreadable: {e}"),
                },
                Err(e) => e.to_string(),
            },
            None => "no permit".into(),
        };
        println!("{rel}: {auth}; {dec}");
    }
}
