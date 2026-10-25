//! AIS straight from an AIS receiver or transponder over NMEA 0183 (`!AIVDM`,
//! `!AIVDO`), as ITU-R M.1371 messages: positions (types 1–3, 18, 19, 27), static
//! and voyage data (5, 24), base stations (4), SAR aircraft (9) and aids to
//! navigation (21). Messages split over several sentences are put back together.
//!
//! `!AIVDO` is our own ship as our transponder reports it: it is not a target, so it
//! is left out.

use crate::{Kind, Update};

/// A message in progress: (sequence id, channel), parts expected, parts so far, fill bits.
type Partial = ((String, String), usize, Vec<String>, u8);

/// Puts multi-sentence messages back together and decodes them.
#[derive(Debug, Default)]
pub struct Decoder {
    parts: Vec<Partial>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// One VDM/VDO sentence, as its formatter (`VDM`, `VDO`) and the fields after it
    /// (count, number, sequence id, channel, payload, fill bits). Returns the target
    /// update when this sentence completes a message we decode.
    pub fn push(&mut self, formatter: &str, fields: &[String]) -> Option<Update> {
        if fields.len() < 6 {
            return None;
        }
        let total: usize = fields[0].parse().ok()?;
        let number: usize = fields[1].parse().ok()?;
        let fill: u8 = fields[5].parse().unwrap_or(0);
        if total == 0 || number == 0 || number > total || total > 9 {
            return None;
        }
        let payload = if total == 1 {
            fields[4].clone()
        } else {
            let key = (fields[2].clone(), fields[3].clone());
            if number == 1 {
                self.parts.retain(|p| p.0 != key);
                self.parts.push((key, total, vec![fields[4].clone()], fill));
                // keep only a few messages in progress (a lost last part never completes)
                if self.parts.len() > 8 {
                    self.parts.remove(0);
                }
                return None;
            }
            let i = self
                .parts
                .iter()
                .position(|p| p.0 == key && p.1 == total && p.2.len() + 1 == number)?;
            self.parts[i].2.push(fields[4].clone());
            if number < total {
                return None;
            }
            let (_, _, done, _) = self.parts.remove(i);
            done.concat()
        };
        if formatter == "VDO" {
            return None;
        }
        decode(&Bits::new(&payload, fill)?)
    }
}

/// The message's bits, from the 6-bit ASCII armour.
struct Bits(Vec<u8>);

impl Bits {
    fn new(payload: &str, fill: u8) -> Option<Bits> {
        let mut v = Vec::with_capacity(payload.len() * 6);
        for c in payload.bytes() {
            if !(48..=119).contains(&c) || (88..96).contains(&c) {
                return None;
            }
            let mut n = c - 48;
            if n > 40 {
                n -= 8;
            }
            for k in (0..6).rev() {
                v.push((n >> k) & 1);
            }
        }
        v.truncate(v.len().saturating_sub(fill.min(5) as usize));
        Some(Bits(v))
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    /// Unsigned field from bit `a` up to (not including) `b`.
    fn u(&self, a: usize, b: usize) -> Option<u32> {
        if b > self.0.len() || b <= a || b - a > 32 {
            return None;
        }
        Some(self.0[a..b].iter().fold(0u32, |n, &x| (n << 1) | x as u32))
    }
    /// Two's-complement signed field.
    fn i(&self, a: usize, b: usize) -> Option<i32> {
        let n = self.u(a, b)?;
        let w = b - a;
        Some(if n >> (w - 1) & 1 == 1 {
            n as i64 - (1i64 << w)
        } else {
            n as i64
        } as i32)
    }
    /// Text in the AIS 6-bit alphabet; padding (`@`) and trailing spaces dropped.
    fn text(&self, a: usize, b: usize) -> Option<String> {
        let b = b.min(self.0.len() / 6 * 6).min(self.0.len());
        if b <= a {
            return None;
        }
        let mut s = String::new();
        let mut i = a;
        while i + 6 <= b {
            let n = self.u(i, i + 6)? as u8;
            s.push(if n < 32 { (n + 64) as char } else { n as char });
            i += 6;
        }
        let t = s.split('@').next().unwrap_or("").trim().to_string();
        (!t.is_empty()).then_some(t)
    }
}

/// Position in 1/10000 minute (the usual), or 1/10 minute (type 27); None when "not available".
fn position(b: &Bits, lon_at: usize, lat_at: usize, coarse: bool) -> Option<(f64, f64)> {
    let (lw, aw, scale) = if coarse {
        (18, 17, 600.0)
    } else {
        (28, 27, 600_000.0)
    };
    let lon = b.i(lon_at, lon_at + lw)? as f64 / scale;
    let lat = b.i(lat_at, lat_at + aw)? as f64 / scale;
    ((-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat)).then_some((lat, lon))
}

fn sog(raw: Option<u32>) -> Option<f64> {
    raw.filter(|s| *s < 1023).map(|s| s as f64 / 10.0)
}
fn cog(raw: Option<u32>) -> Option<f64> {
    raw.filter(|c| *c < 3600).map(|c| c as f64 / 10.0)
}
fn heading(raw: Option<u32>) -> Option<f64> {
    raw.filter(|h| *h < 360).map(|h| h as f64)
}

/// Size and antenna position from the distances to bow, stern, port and starboard.
fn dims(u: &mut Update, b: &Bits, at: usize) {
    let (Some(bow), Some(stern), Some(port), Some(stbd)) = (
        b.u(at, at + 9),
        b.u(at + 9, at + 18),
        b.u(at + 18, at + 24),
        b.u(at + 24, at + 30),
    ) else {
        return;
    };
    let (l, w) = ((bow + stern) as f64, (port + stbd) as f64);
    if l > 0.0 {
        u.length = Some(l);
        u.from_bow = Some(bow as f64);
    }
    if w > 0.0 {
        u.beam = Some(w);
        u.from_centre = Some(port as f64 - w / 2.0);
    }
}

fn decode(b: &Bits) -> Option<Update> {
    let ty = b.u(0, 6)?;
    let mmsi = b.u(8, 38)?;
    if mmsi == 0 {
        return None;
    }
    let mut u = Update {
        mmsi,
        source: "nmea",
        kind: Kind::from_mmsi(mmsi),
        ..Default::default()
    };
    match ty {
        1..=3 => {
            u.status = b.u(38, 42).map(|s| s as u8);
            u.sog = sog(b.u(50, 60));
            u.pos = position(b, 61, 89, false);
            u.cog = cog(b.u(116, 128));
            u.heading = heading(b.u(128, 137));
            u.class = Some("A");
        }
        4 => {
            u.kind = Some(Kind::Base);
            u.pos = position(b, 79, 107, false);
        }
        5 => {
            if b.len() < 420 {
                return None;
            }
            u.callsign = b.text(70, 112);
            u.name = b.text(112, 232);
            u.ship_type = b.u(232, 240).map(|t| t as u16).filter(|t| *t > 0);
            dims(&mut u, b, 240);
            u.draught = b.u(294, 302).filter(|d| *d > 0).map(|d| d as f64 / 10.0);
            u.destination = b.text(302, 422);
            u.class = Some("A");
        }
        9 => {
            u.kind = Some(Kind::Aircraft);
            u.altitude = b.u(38, 50).filter(|a| *a < 4095).map(|a| a as f64);
            u.sog = b.u(50, 60).filter(|s| *s < 1023).map(|s| s as f64);
            u.pos = position(b, 61, 89, false);
            u.cog = cog(b.u(116, 128));
        }
        18 | 19 => {
            u.sog = sog(b.u(46, 56));
            u.pos = position(b, 57, 85, false);
            u.cog = cog(b.u(112, 124));
            u.heading = heading(b.u(124, 133));
            u.class = Some("B");
            if ty == 19 {
                u.name = b.text(143, 263);
                u.ship_type = b.u(263, 271).map(|t| t as u16).filter(|t| *t > 0);
                dims(&mut u, b, 271);
            }
        }
        21 => {
            u.kind = Some(Kind::Aton);
            u.aton_type = b.u(38, 43).map(|t| t as u8);
            let mut name = b.text(43, 163).unwrap_or_default();
            if b.len() > 272 {
                name.push_str(&b.text(272, b.len()).unwrap_or_default());
            }
            u.name = (!name.is_empty()).then_some(name);
            u.pos = position(b, 164, 192, false);
            dims(&mut u, b, 219);
            u.off_position = b.u(259, 260).map(|v| v == 1);
            u.virtual_aton = b.u(269, 270).map(|v| v == 1);
        }
        24 => {
            match b.u(38, 40)? {
                0 => u.name = b.text(40, 160),
                1 => {
                    u.ship_type = b.u(40, 48).map(|t| t as u16).filter(|t| *t > 0);
                    u.callsign = b.text(90, 132);
                    dims(&mut u, b, 132);
                }
                _ => return None,
            }
            u.class = Some("B");
        }
        27 => {
            u.status = b.u(40, 44).map(|s| s as u8);
            u.pos = position(b, 44, 62, true);
            u.sog = b.u(79, 85).filter(|s| *s < 63).map(|s| s as f64);
            u.cog = b.u(85, 94).filter(|c| *c < 511).map(|c| c as f64);
        }
        _ => return None,
    }
    Some(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> (String, Vec<String>) {
        let body = s.split('*').next().unwrap();
        let mut it = body[1..].split(',');
        let id = it.next().unwrap();
        (id[2..].to_string(), it.map(str::to_string).collect())
    }

    #[test]
    fn position_report_class_a() {
        // the usual worked example (gpsd's AIVDM guide)
        let mut d = Decoder::new();
        let (fmt, fields) = f("!AIVDM,1,1,,B,15M67FC000G?ufbE`FepT@3n00Sa,0*5C");
        let u = d.push(&fmt, &fields).unwrap();
        assert_eq!(u.mmsi, 366053209);
        assert_eq!(u.class, Some("A"));
        let (lat, lon) = u.pos.unwrap();
        assert!(
            (lat - 37.80211).abs() < 1e-4 && (lon + 122.34161).abs() < 1e-4,
            "{lat} {lon}"
        );
        assert_eq!(u.sog, Some(0.0));
        assert_eq!(u.status, Some(3));
        assert!((u.cog.unwrap() - 219.3).abs() < 1e-9);
    }

    #[test]
    fn static_data_in_two_parts() {
        let mut d = Decoder::new();
        let (fmt, a) =
            f("!AIVDM,2,1,1,A,55?MbV02;H;s<HtKR20EHE:0@T4@Dn2222222216L961O5Gf0NSQEp6ClRp8,0*1C");
        assert!(d.push(&fmt, &a).is_none());
        let (fmt, b) = f("!AIVDM,2,2,1,A,88888888880,2*25");
        let u = d.push(&fmt, &b).unwrap();
        assert_eq!(u.mmsi, 351759000);
        assert_eq!(u.name.as_deref(), Some("EVER DIADEM"));
        assert_eq!(u.callsign.as_deref(), Some("3FOF8"));
        assert_eq!(u.destination.as_deref(), Some("NEW YORK"));
        assert_eq!(u.ship_type, Some(70));
        assert_eq!(u.length, Some(225.0 + 70.0));
        assert_eq!(u.beam, Some(1.0 + 31.0));
        assert_eq!(u.draught, Some(12.2));
        // a second part on its own, or out of order, gives nothing
        assert!(d.push(&fmt, &b).is_none());
    }

    #[test]
    fn class_b_and_own_ship() {
        let mut d = Decoder::new();
        let (fmt, fields) = f("!AIVDM,1,1,,B,B52K>;h00Fc>jpUlNV@ikwpUoP06,0*4C");
        let u = d.push(&fmt, &fields).unwrap();
        assert_eq!(u.mmsi, 338087471);
        assert_eq!(u.class, Some("B"));
        assert_eq!(u.sog, Some(0.1));
        let (lat, lon) = u.pos.unwrap();
        assert!(
            (lat - 40.68454).abs() < 1e-4 && (lon + 74.07213).abs() < 1e-4,
            "{lat} {lon}"
        );
        assert_eq!(u.cog, Some(79.6));
        // the same message as our own ship's (VDO) is not a target
        assert!(d.push("VDO", &fields).is_none());
    }

    #[test]
    fn bad_payloads_are_ignored() {
        let mut d = Decoder::new();
        let fields: Vec<String> = ["1", "1", "", "A", "15M67F~~", "0"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.push("VDM", &fields).is_none());
        assert!(d.push("VDM", &fields[..3]).is_none());
    }
}
