//! Run the IHO S-64 encrypted ENC tests (sections 2.5.2 to 2.5.8) against TweedPlot's ENC
//! import, printing what the system reports at each step.
//!
//!     cargo run --release -p chart --example s64 -- <S-64_ENC_Encrypted_TDS folder> [test ...]
//!
//! Each test starts from an empty ENC store (unless it follows on from the one before, as
//! S-64 says), with the IHO generic test user permit and the test's own date.

use chart::enc::senc::{Level, Msg, Senc};
use std::path::{Path, PathBuf};

enum Step {
    /// start again: no cells, permits or key
    Reset,
    /// start again with the built-in IHO key
    ResetIho,
    Date(&'static str),
    Key(&'static str),
    Permits(&'static str),
    Import(&'static str),
    RemovePermits(&'static str),
    ListPermits,
    Status,
}
use Step::*;

fn tests() -> Vec<(&'static str, &'static str, Vec<Step>)> {
    vec![
        (
            "2a",
            "Check permit string availability",
            vec![ResetIho, Date("20130501"), Permits("2 ENC Licencing/Test 2a/PERMIT.TXT"), Permits("2 ENC Licencing/Test 2a/TEXT.TXT")],
        ),
        (
            "2b",
            "ENC cell permit string incorrect format",
            vec![ResetIho, Date("20130501"), Permits("2 ENC Licencing/Test 2b/PERMIT.TXT"), Import("2 ENC Licencing/Test 2b/V01X01"), Status],
        ),
        (
            "2c",
            "Validate permit CRC",
            vec![ResetIho, Date("20130501"), Permits("2 ENC Licencing/Test 2c/1/PERMIT.TXT"), Permits("2 ENC Licencing/Test 2c/2/PERMIT.TXT")],
        ),
        ("2d", "Check remaining permit expiry period", vec![ResetIho, Date("20121203"), Permits("2 ENC Licencing/Test 2d/PERMIT.TXT")]),
        ("2e", "Check for expired permits", vec![ResetIho, Date("20130101"), Permits("2 ENC Licencing/Test 2e/PERMIT.TXT")]),
        ("2f", "Permit installation and reporting", vec![ResetIho, Date("20181101"), Permits("2 ENC Licencing/Test 2f/PERMIT.TXT")]),
        (
            "2g",
            "Permits from multiple data servers",
            vec![ResetIho, Date("20130501"), Permits("2 ENC Licencing/Test 2g/DS1/PERMIT.TXT"), Permits("2 ENC Licencing/Test 2g/DS2/PERMIT.TXT"), ListPermits],
        ),
        ("2h", "Management of installed permits", vec![RemovePermits("PM"), ListPermits]),
        (
            "4a",
            "Install and validate the SA certificate (non-SA certificate)",
            vec![
                Reset,
                Date("20100801"),
                Key("4 Authentication_Part1/Test 4a/UKHO.CRT"),
                Permits("4 Authentication_Part1/Test 4a/PERMIT.TXT"),
                Import("4 Authentication_Part1/Test 4a/V01X01"),
                Status,
            ],
        ),
        (
            "4b",
            "Change and update installed certificate",
            vec![
                Key("4 Authentication_Part1/Test 4b/IHO.CRT"),
                Permits("4 Authentication_Part1/Test 4b/PERMIT.TXT"),
                Import("4 Authentication_Part1/Test 4b/V01X01"),
                Status,
            ],
        ),
        (
            "4c",
            "No pre-installed certificate",
            vec![Reset, Date("20130501"), Permits("4 Authentication_Part1/Test 4c/PERMIT.TXT"), Import("4 Authentication_Part1/Test 4c/V01X01"), Status],
        ),
        (
            "4d",
            "Check SA certificate expiry date",
            vec![
                Reset,
                Date("20120101"),
                Key("4 Authentication_Part1/Test 4d/Expired/IHO.CRT"),
                Permits("4 Authentication_Part1/Test 4d/Expired/PERMIT.TXT"),
                Import("4 Authentication_Part1/Test 4d/Expired/V01X01"),
                Key("4 Authentication_Part1/Test 4d/Current/IHO.CRT"),
                Permits("4 Authentication_Part1/Test 4d/Current/PERMIT.TXT"),
                Import("4 Authentication_Part1/Test 4d/Current/V01X01"),
                Status,
            ],
        ),
        (
            "4e",
            "Incorrectly formatted certificate and public key",
            vec![
                Reset,
                Date("20130501"),
                Key("4 Authentication_Part1/Test 4e/IHO.CRT"),
                Key("4 Authentication_Part1/Test 4e/IHO.PUB"),
                Permits("4 Authentication_Part1/Test 4e/PERMIT.TXT"),
                Import("4 Authentication_Part1/Test 4e/V01X01"),
            ],
        ),
        (
            "4f",
            "Check certificate parameter values",
            vec![
                Reset,
                Date("20100801"),
                Key("4 Authentication_Part1/Test 4f/DS1/IHO.CRT"),
                Permits("4 Authentication_Part1/Test 4f/DS1/PERMIT.TXT"),
                Import("4 Authentication_Part1/Test 4f/DS1/V01X01"),
                Key("4 Authentication_Part1/Test 4f/DS2/IHO.CRT"),
                Permits("4 Authentication_Part1/Test 4f/DS2/PERMIT.TXT"),
                Import("4 Authentication_Part1/Test 4f/DS2/V01X01"),
                Status,
            ],
        ),
        (
            "5a",
            "Invalid SA signature in the ENC signature file",
            vec![
                ResetIho,
                Date("20130501"),
                Key("5 Authentication_Part2/Test 5a/IHO.CRT"),
                Permits("5 Authentication_Part2/Test 5a/PERMIT.TXT"),
                Import("5 Authentication_Part2/Test 5a/V01X01"),
                Status,
            ],
        ),
        (
            "5b",
            "Authentication against a non-SA key",
            vec![
                Reset,
                Date("20100801"),
                Key("5 Authentication_Part2/Test 5b/NONSA.PUB"),
                Key("5 Authentication_Part2/Test 5b/NONSA.CRT"),
                Permits("5 Authentication_Part2/Test 5b/PERMIT.TXT"),
                Import("5 Authentication_Part2/Test 5b/V01X01"),
                Status,
            ],
        ),
        (
            "5c",
            "ENC signature validation",
            vec![ResetIho, Date("20130501"), Permits("5 Authentication_Part2/Test 5c/PERMIT.TXT"), Import("5 Authentication_Part2/Test 5c/V01X01"), Status],
        ),
        ("5d", "ENC signature format validation", vec![Import("5 Authentication_Part2/Test 5d/V01X01"), Status]),
        (
            "5e",
            "Authentication is continuous and complete",
            vec![Permits("5 Authentication_Part2/Test 5e/PERMIT.TXT"), Import("5 Authentication_Part2/Test 5e/V01X01"), Status],
        ),
        (
            "5f",
            "Signature files from several data servers",
            vec![ResetIho, Date("20130501"), Permits("5 Authentication_Part2/Test 5f/PERMIT.TXT"), Import("5 Authentication_Part2/Test 5f/V01X01"), Status],
        ),
        (
            "6a",
            "Install ENCs when the permits have expired",
            vec![ResetIho, Date("20130101"), Permits("6 ENC Decryption/Test 6a/PERMIT.TXT"), Import("6 ENC Decryption/Test 6a/V01X01"), Status],
        ),
        ("6b", "Permit expiry within 30 days", vec![Date("20121215"), Import("6 ENC Decryption/Test 6b/V01X01"), Status]),
        (
            "6c",
            "Incorrect cell keys in the permits",
            vec![ResetIho, Date("20130501"), Permits("6 ENC Decryption/Test 6c/PERMIT.TXT"), Import("6 ENC Decryption/Test 6c/V01X01"), Status],
        ),
        (
            "6d",
            "Validate ENC data file integrity",
            vec![ResetIho, Date("20130501"), Permits("6 ENC Decryption/Test 6d/PERMIT.TXT"), Import("6 ENC Decryption/Test 6d/V01X01"), Status],
        ),
        (
            "6e",
            "Missing ENC update",
            vec![ResetIho, Date("20130501"), Permits("6 ENC Decryption/Test 6e/V01X01/PERMIT.TXT"), Import("6 ENC Decryption/Test 6e/V01X01"), Status],
        ),
        (
            "7a",
            "ENCs from different data servers",
            vec![
                ResetIho,
                Date("20130501"),
                Permits("7 ENC Data Management/Test 7a/DS1/PERMIT.TXT"),
                Import("7 ENC Data Management/Test 7a/DS1/V01X01"),
                Status,
                Permits("7 ENC Data Management/Test 7a/DS2/PERMIT.TXT"),
                Import("7 ENC Data Management/Test 7a/DS2/V01X01"),
                Status,
            ],
        ),
        (
            "7b",
            "Additional permits and cells from a data server",
            vec![Permits("7 ENC Data Management/Test 7b/PERMIT.TXT"), Import("7 ENC Data Management/Test 7b/V01X01"), Status],
        ),
        (
            "7c",
            "Multiple data provider environment",
            vec![
                ResetIho,
                Date("20130501"),
                Permits("7 ENC Data Management/Test 7c/DS1/PERMIT.TXT"),
                Import("7 ENC Data Management/Test 7c/DS1/V01X01"),
                Import("7 ENC Data Management/Test 7c/DS2/V01X01"),
                Permits("7 ENC Data Management/Test 7c/DS2/PERMIT.TXT"),
                Import("7 ENC Data Management/Test 7c/DS2/V01X01"),
                Status,
            ],
        ),
        (
            "7d",
            "Cancelled cells",
            vec![
                ResetIho,
                Date("20130501"),
                Permits("7 ENC Data Management/Test 7d/PERMIT.TXT"),
                Import("7 ENC Data Management/Test 7d/Base"),
                Status,
                Import("7 ENC Data Management/Test 7d/Update"),
                Status,
            ],
        ),
        (
            "7e",
            "Replacement cells",
            vec![
                Permits("7 ENC Data Management/Test 7e/PERMIT.TXT"),
                Import("7 ENC Data Management/Test 7e/Base"),
                Status,
                Import("7 ENC Data Management/Test 7e/Update"),
                Status,
            ],
        ),
        (
            "7f",
            "Re-issued cells",
            vec![
                ResetIho,
                Date("20130501"),
                Permits("7 ENC Data Management/Test 7f/PERMIT.TXT"),
                Import("7 ENC Data Management/Test 7f/Base"),
                Status,
                Import("7 ENC Data Management/Test 7f/Update"),
                Status,
            ],
        ),
        (
            "7g",
            "Base and update exchange sets",
            vec![ResetIho, Date("20130501"), Permits("7 ENC Data Management/Test 7g/PERMIT.TXT"), Import("7 ENC Data Management/Test 7g"), Status],
        ),
        (
            "8a",
            "Exchange set and media delivery",
            vec![ResetIho, Date("20130501"), Permits("8 Data Exchange Media/Test 8a/PERMIT.TXT"), Import("8 Data Exchange Media/Test 8a"), Status],
        ),
        (
            "8b",
            "Single medium with several exchange sets",
            vec![ResetIho, Date("20130501"), Permits("8 Data Exchange Media/Test 8b/PERMIT.TXT"), Import("8 Data Exchange Media/Test 8b"), Status],
        ),
        (
            "8c",
            "Exchange sets across several media",
            vec![
                ResetIho,
                Date("20130501"),
                Permits("8 Data Exchange Media/Test 8c/PERMIT.TXT"),
                Import("8 Data Exchange Media/Test 8c/UPDATE MEDIA"),
                Import("8 Data Exchange Media/Test 8c/BASE MEDIA"),
                Import("8 Data Exchange Media/Test 8c/UPDATE MEDIA"),
                Status,
            ],
        ),
        (
            "8d",
            "Media validation of service status",
            vec![
                Import("8 Data Exchange Media/Test 8d/UPDATE MEDIA"),
                Status,
                Import("8 Data Exchange Media/Test 8d/BASE MEDIA"),
                Import("8 Data Exchange Media/Test 8d/UPDATE MEDIA"),
                Status,
            ],
        ),
    ]
}

fn show(v: &[Msg]) {
    for m in v {
        let l = match m.level {
            Level::Ok => "  ok  ",
            Level::Info => "  --  ",
            Level::Warn => " WARN ",
            Level::Error => " ERROR",
        };
        if m.cell.is_empty() {
            println!("    {l} {}", m.text);
        } else {
            println!("    {l} {}: {}", m.cell, m.text);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let base = PathBuf::from(args.first().expect("the S-64_ENC_Encrypted_TDS folder"));
    let only: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    let store = std::env::temp_dir().join(format!("s64_{}", std::process::id()));
    chart::enc::set_test_protection();
    let at = |p: &str| -> PathBuf {
        // the folders' names vary in case between tests
        let mut cur = base.clone();
        for part in p.split('/') {
            let next =
                std::fs::read_dir(&cur).ok().and_then(|rd| rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part)).map(|e| e.path()));
            cur = next.unwrap_or_else(|| cur.join(part));
        }
        cur
    };
    for (id, title, steps) in tests() {
        if !only.is_empty() && !only.contains(&id) {
            continue;
        }
        println!("\n=== Test {id}: {title}");
        for s in steps {
            match s {
                Reset | ResetIho => {
                    let _ = std::fs::remove_dir_all(&store);
                    std::fs::create_dir_all(&store).unwrap();
                    let mut senc = Senc::open_bare(&store);
                    if matches!(s, ResetIho) {
                        senc.install_key("IHO.CRT", chart::enc::senc::IHO_CRT);
                        println!("  (empty store, IHO key installed)");
                    } else {
                        senc.remove_key();
                        println!("  (empty store, no key)");
                    }
                }
                Date(d) => {
                    chart::enc::set_today(Some(d));
                    println!("  date {d}");
                }
                Key(p) => {
                    println!("  install key {p}");
                    let b = std::fs::read(at(p)).unwrap_or_default();
                    show(&Senc::open_bare(&store).install_key(Path::new(p).file_name().unwrap().to_str().unwrap(), &b));
                }
                Permits(p) => {
                    println!("  install permits {p}");
                    let t = String::from_utf8_lossy(&std::fs::read(at(p)).unwrap_or_default()).to_string();
                    show(&Senc::open_bare(&store).install_permits(&t));
                }
                Import(p) => {
                    println!("  import {p}");
                    show(&Senc::open_bare(&store).import(&at(p), &mut |_, _, _| {}));
                }
                RemovePermits(ds) => {
                    println!("  remove the permits of {ds}: {}", Senc::open_bare(&store).remove_permits(ds));
                }
                ListPermits => {
                    for (ds, v) in Senc::open_bare(&store).permits() {
                        println!("    permits from {ds}: {}", v.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>().join(" "));
                    }
                }
                Status => {
                    let s = Senc::open_bare(&store);
                    let v: Vec<String> = s.cells().iter().map(|(c, i)| format!("{c} {}/{} ({})", i.edition, i.update, i.ds)).collect();
                    println!("  installed: {}", if v.is_empty() { "nothing".to_string() } else { v.join(", ") });
                    for (c, t) in s.display_notes() {
                        println!("    shown with: {c}: {t}");
                    }
                    // and the installed cells convert and decrypt
                    let n = chart::enc::ensure_gpkgs(&store).len();
                    let notes = chart::enc::notes();
                    chart::enc::clear_notes();
                    let bad: Vec<String> = notes
                        .iter()
                        .filter(|n| !n.text.starts_with("SSE 25") && !n.text.starts_with("SSE 27"))
                        .map(|n| format!("{}: {}", n.cell, n.text))
                        .collect();
                    println!("  converted: {n} band file(s){}", if bad.is_empty() { String::new() } else { format!("; problems: {}", bad.join("; ")) });
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&store);
}
