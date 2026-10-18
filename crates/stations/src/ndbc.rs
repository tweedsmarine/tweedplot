//! NOAA National Data Buoy Center: latest observations of every station
//! (`latest_obs.txt`) and a station's last 45 days (`realtime2/<id>.txt`).

use crate::{get_text, Obs, Result, MS_TO_KN};

/// The latest report of every NDBC station (one ~200 kB file).
pub fn fetch_latest() -> Result<Vec<Obs>> {
    parse_latest(&get_text("https://www.ndbc.noaa.gov/data/latest_obs/latest_obs.txt")?)
}

/// One station's reports over the last `hours`, with its position.
pub fn history(id: &str, lat: f64, lon: f64, hours: u32) -> Result<Vec<Obs>> {
    let text = get_text(&format!("https://www.ndbc.noaa.gov/data/realtime2/{}.txt", id.to_ascii_uppercase()))?;
    let since = chrono::Utc::now().timestamp() - hours as i64 * 3600;
    Ok(parse_realtime(&text, id, lat, lon).into_iter().filter(|o| o.t >= since).collect())
}

fn num(s: &str) -> Option<f64> {
    if s == "MM" {
        None
    } else {
        s.parse().ok()
    }
}

fn time(y: &str, mo: &str, d: &str, h: &str, mi: &str) -> Option<i64> {
    let dt = chrono::NaiveDate::from_ymd_opt(y.parse().ok()?, mo.parse().ok()?, d.parse().ok()?)?.and_hms_opt(h.parse().ok()?, mi.parse().ok()?, 0)?;
    Some(dt.and_utc().timestamp())
}

/// `#STN LAT LON YYYY MM DD hh mm WDIR WSPD GST WVHT DPD APD MWD PRES PTDY ATMP WTMP DEWP VIS TIDE`
pub fn parse_latest(text: &str) -> Result<Vec<Obs>> {
    let mut out = Vec::new();
    for l in text.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = l.split_whitespace().collect();
        if c.len() < 21 {
            continue;
        }
        let (Some(lat), Some(lon), Some(t)) = (num(c[1]), num(c[2]), time(c[3], c[4], c[5], c[6], c[7])) else { continue };
        out.push(Obs {
            id: format!("ndbc:{}", c[0]),
            source: "ndbc",
            name: format!("NDBC {}", c[0]),
            lat,
            lon,
            t,
            wind_dir: num(c[8]),
            wind_kn: num(c[9]).map(|v| v * MS_TO_KN),
            gust_kn: num(c[10]).map(|v| v * MS_TO_KN),
            wave_m: num(c[11]),
            wave_period_s: num(c[12]),
            wave_dir: num(c[14]),
            pressure_hpa: num(c[15]),
            temp_c: num(c[17]),
            water_temp_c: num(c[18]),
            dewpoint_c: num(c[19]),
            vis_km: num(c[20]).map(|nmi| nmi * 1.852),
            kind: if c[0].chars().next().is_some_and(|ch| ch.is_ascii_digit()) { "buoy" } else { "station" },
            ..Default::default()
        });
    }
    Ok(out)
}

/// `#YY MM DD hh mm WDIR WSPD GST WVHT DPD APD MWD PRES ATMP WTMP DEWP VIS PTDY TIDE`, newest first.
pub fn parse_realtime(text: &str, id: &str, lat: f64, lon: f64) -> Vec<Obs> {
    let mut out = Vec::new();
    for l in text.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = l.split_whitespace().collect();
        if c.len() < 17 {
            continue;
        }
        let Some(t) = time(c[0], c[1], c[2], c[3], c[4]) else { continue };
        out.push(Obs {
            id: format!("ndbc:{id}"),
            source: "ndbc",
            name: format!("NDBC {id}"),
            lat,
            lon,
            t,
            wind_dir: num(c[5]),
            wind_kn: num(c[6]).map(|v| v * MS_TO_KN),
            gust_kn: num(c[7]).map(|v| v * MS_TO_KN),
            wave_m: num(c[8]),
            wave_period_s: num(c[9]),
            wave_dir: num(c[11]),
            pressure_hpa: num(c[12]),
            temp_c: num(c[13]),
            water_temp_c: num(c[14]),
            dewpoint_c: num(c[15]),
            vis_km: num(c[16]).map(|nmi| nmi * 1.852),
            kind: "buoy",
            ..Default::default()
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_and_realtime() {
        let t = "#STN       LAT      LON  YYYY MM DD hh mm WDIR WSPD   GST WVHT  DPD APD MWD   PRES  PTDY  ATMP  WTMP  DEWP  VIS   TIDE\n\
#text      deg      deg   yr mo day hr mn degT  m/s   m/s   m   sec sec degT   hPa   hPa  degC  degC  degC  nmi     ft\n\
51004    17.533 -152.230 2026 10 19 08 50  80   7.0   9.0   2.1  9  6.4  70 1015.2  -0.4  25.1  26.3    MM   MM    MM\n\
AUCN3    -36.8   174.8  2026 10 19 08 50  MM    MM    MM    MM  MM   MM  MM     MM    MM    MM    MM    MM   MM    MM\n";
        let o = parse_latest(t).unwrap();
        assert_eq!(o.len(), 2);
        assert_eq!((o[0].id.as_str(), o[0].wind_dir, o[0].wave_m, o[0].pressure_hpa, o[0].kind), ("ndbc:51004", Some(80.0), Some(2.1), Some(1015.2), "buoy"));
        assert!((o[0].wind_kn.unwrap() - 13.6).abs() < 0.05);
        assert_eq!(o[1].wind_kn, None);
        let r = "#YY  MM DD hh mm WDIR WSPD GST  WVHT   DPD   APD MWD   PRES  ATMP  WTMP  DEWP  VIS PTDY  TIDE\n\
#yr  mo dy hr mn degT m/s  m/s     m   sec   sec degT   hPa  degC  degC  degC  nmi  hPa    ft\n\
2026 10 19 08 50  80  7.0  9.0   2.1     9   6.4  70 1015.2  25.1  26.3    MM   MM -0.4    MM\n\
2026 10 19 08 40  75  6.0  8.0    MM    MM    MM  MM 1015.3  25.1  26.3    MM   MM   MM    MM\n";
        let h = parse_realtime(r, "51004", 17.5, -152.2);
        assert_eq!(h.len(), 2);
        assert_eq!(h[1].wind_dir, Some(75.0));
        assert_eq!(h[0].t - h[1].t, 600);
    }
}
