//! A fingerprint of the computer, for S-63 activation and for the key that protects
//! the decrypted chart data on disk.
//!
//! It is made from the main board's identity, which stays the same when Windows is
//! reinstalled or a disk is changed: the SMBIOS system UUID and the board's serial
//! number. Where the firmware leaves those blank or filled with placeholder values, the
//! operating system's machine ID is used instead (which changes with a reinstall).

use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Values firmware makers leave in place of a real UUID or serial number.
fn placeholder(v: &str) -> bool {
    let t = v.trim().to_ascii_lowercase();
    let digits: String = t.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    t.is_empty()
        || digits.chars().all(|c| c == '0')
        || digits.chars().all(|c| c == 'f')
        || t == "03000200-0400-0500-0006-000700080009"
        || ["to be filled", "default string", "not applicable", "not specified", "none", "system serial", "base board serial", "0123456789", "n/a", "oem"]
            .iter()
            .any(|p| t.contains(p))
}

#[cfg(windows)]
fn run(prog: &str, args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    // no console window flashing up from the GUI app
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(prog).args(args).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(not(windows))]
#[allow(dead_code)]
fn run(prog: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(prog).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// (name, value) pairs identifying the board, and the OS machine ID as a fallback.
#[cfg(windows)]
fn sources() -> (Vec<(&'static str, String)>, Option<String>) {
    let mut board = Vec::new();
    let ps = run(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance -ClassName Win32_ComputerSystemProduct).UUID; '|'; (Get-CimInstance -ClassName Win32_BaseBoard).SerialNumber",
        ],
    );
    if let Some(t) = ps {
        let mut parts = t.split('|');
        if let Some(u) = parts.next() {
            board.push(("uuid", u.trim().to_ascii_uppercase()));
        }
        if let Some(s) = parts.next() {
            board.push(("board", s.trim().to_string()));
        }
    }
    let guid = run("reg", &["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"])
        .and_then(|t| t.lines().find(|l| l.contains("MachineGuid")).and_then(|l| l.split_whitespace().last()).map(|s| s.to_ascii_uppercase()));
    (board, guid)
}

#[cfg(target_os = "macos")]
fn sources() -> (Vec<(&'static str, String)>, Option<String>) {
    let mut board = Vec::new();
    if let Some(t) = run("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"]) {
        for (key, name) in [("IOPlatformUUID", "uuid"), ("IOPlatformSerialNumber", "board")] {
            if let Some(l) = t.lines().find(|l| l.contains(&format!("\"{key}\""))) {
                if let Some(v) = l.split('=').nth(1) {
                    board.push((name, v.trim().trim_matches('"').to_string()));
                }
            }
        }
    }
    (board, None)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn sources() -> (Vec<(&'static str, String)>, Option<String>) {
    let read = |p: &str| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let mut board = Vec::new();
    // readable by root only on most systems
    if let Some(u) = read("/sys/class/dmi/id/product_uuid") {
        board.push(("uuid", u.to_ascii_uppercase()));
    }
    if let Some(s) = read("/sys/class/dmi/id/board_serial") {
        board.push(("board", s));
    }
    (board, read("/etc/machine-id").or_else(|| read("/var/lib/dbus/machine-id")))
}

/// What the fingerprint is made from (for checking it on a new computer).
pub fn describe() -> Vec<String> {
    let (board, os) = sources();
    let mut v: Vec<String> = board.iter().map(|(k, val)| format!("{k}: {val}{}", if placeholder(val) { " (placeholder, not used)" } else { "" })).collect();
    v.push(format!("machine id: {}", os.unwrap_or_else(|| "none".into())));
    v
}

fn compute() -> [u8; 16] {
    let (board, os) = sources();
    let mut text = String::from("TweedPlot fingerprint 1\n");
    let good: Vec<&(&str, String)> = board.iter().filter(|(_, v)| !placeholder(v)).collect();
    if good.is_empty() {
        text.push_str(&format!("os={}\n", os.unwrap_or_default()));
    } else {
        for (k, v) in good {
            text.push_str(&format!("{k}={v}\n"));
        }
    }
    let h = Sha256::digest(text.as_bytes());
    h[..16].try_into().unwrap()
}

static FINGERPRINT: OnceLock<[u8; 16]> = OnceLock::new();

/// This computer's fingerprint (worked out once; a second or so on Windows).
pub fn fingerprint() -> [u8; 16] {
    *FINGERPRINT.get_or_init(compute)
}

/// The fingerprint as the installation code the user sends: "TP1-" and five groups of
/// hex, the last a check on the others.
pub fn installation_code() -> String {
    code_for(&fingerprint())
}

pub fn code_for(fp: &[u8; 16]) -> String {
    let hex: String = fp.iter().map(|b| format!("{b:02X}")).collect();
    let check = Sha256::digest(fp);
    let mut groups: Vec<String> = (0..8).map(|i| hex[i * 4..i * 4 + 4].to_string()).collect();
    groups.push(format!("{:02X}{:02X}", check[0], check[1]));
    format!("TP1-{}", groups.join("-"))
}

/// Read an installation code back into the fingerprint (the OEM tool's side).
pub fn read_code(code: &str) -> Option<[u8; 16]> {
    let t: String = code.trim().to_ascii_uppercase().strip_prefix("TP1-")?.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if t.len() != 36 {
        return None;
    }
    let b: Vec<u8> = (0..18).map(|i| u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()).collect::<Option<_>>()?;
    let fp: [u8; 16] = b[..16].try_into().ok()?;
    let check = Sha256::digest(fp);
    (b[16] == check[0] && b[17] == check[1]).then_some(fp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        let fp = [0x12u8, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 1, 2, 3, 4, 5, 6, 7, 8];
        let c = code_for(&fp);
        assert_eq!(c.len(), 4 + 9 * 4 + 8);
        assert_eq!(read_code(&c), Some(fp));
        assert_eq!(read_code(&c.to_lowercase()), Some(fp));
        // a typo is caught
        let bad = c.replacen("1234", "1235", 1);
        assert_eq!(read_code(&bad), None);
        assert!(placeholder("To Be Filled By O.E.M."));
        assert!(placeholder("03000200-0400-0500-0006-000700080009"));
        assert!(placeholder("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"));
        assert!(!placeholder("4C4C4544-0051-3510-8052-B4C04F395432"));
        // and this computer has one, the same each time
        assert_eq!(fingerprint(), fingerprint());
    }
}
