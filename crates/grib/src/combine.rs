//! Put a wave GRIB's fields into the same file as the wind GRIB of the same run
//! (ECMWF IFS + ECMWF wave, GFS + GFS-Wave), so one file shows wind and waves.
//!
//! GRIB files are a plain sequence of messages, each with its own grid, so the
//! combined file is the two files one after the other. Nothing is re-encoded.

use crate::{scan, GribError};
use std::path::{Path, PathBuf};

/// Why two files can't be combined, in words for the user.
#[derive(Debug)]
pub enum CombineError {
    Grib(GribError),
    Io(std::io::Error),
    Unsuitable(String),
}

impl std::fmt::Display for CombineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CombineError::Grib(e) => write!(f, "{e}"),
            CombineError::Io(e) => write!(f, "{e}"),
            CombineError::Unsuitable(s) => write!(f, "{s}"),
        }
    }
}

impl From<GribError> for CombineError {
    fn from(e: GribError) -> Self {
        CombineError::Grib(e)
    }
}
impl From<std::io::Error> for CombineError {
    fn from(e: std::io::Error) -> Self {
        CombineError::Io(e)
    }
}

fn has(info: &crate::FileInfo, what: &str) -> bool {
    info.runs.iter().any(|r| r.contents.iter().any(|c| c == what))
}

/// Which of two files is the wind file and which the wave file, if they can be
/// combined: one has wind and no waves, the other waves and no wind, both from
/// the same run time, and their areas overlap.
pub fn pair(a: &Path, b: &Path) -> Result<(PathBuf, PathBuf), CombineError> {
    let (ia, ib) = (scan(a)?, scan(b)?);
    let (wind, waves, iw, iv) = match (has(&ia, "wind") && !has(&ia, "waves"), has(&ib, "wind") && !has(&ib, "waves")) {
        (true, false) if has(&ib, "waves") => (a, b, ia, ib),
        (false, true) if has(&ia, "waves") => (b, a, ib, ia),
        _ => return Err(CombineError::Unsuitable("Combine needs one wind GRIB without waves and one wave GRIB without wind.".into())),
    };
    let (rw, rv) = (&iw.runs[0], &iv.runs[0]);
    if rw.ref_time != rv.ref_time {
        return Err(CombineError::Unsuitable(format!(
            "The runs differ ({} and {}): combine the wave run that matches the wind run.",
            crate::fmt_short(rw.ref_time),
            crate::fmt_short(rv.ref_time)
        )));
    }
    let overlap = |p: [f64; 4], q: [f64; 4]| {
        // longitudes may run 0..360 in one file and -180..180 in the other
        let lat = p[1] < q[3] && q[1] < p[3];
        let lon = [-360.0, 0.0, 360.0].iter().any(|s| p[0] + s < q[2] && q[0] < p[2] + s);
        lat && lon
    };
    if !overlap(rw.bbox, rv.bbox) {
        return Err(CombineError::Unsuitable("The wind and wave GRIBs cover different areas.".into()));
    }
    Ok((wind.to_path_buf(), waves.to_path_buf()))
}

/// The combined file's name: the wind file's with "+waves" before the extension.
pub fn combined_path(wind: &Path) -> PathBuf {
    let stem = wind.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "combined".into());
    let ext = wind.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "grb2".into());
    wind.with_file_name(format!("{stem}+waves.{ext}"))
}

/// Write `wind` followed by `waves` to `out` (through a temporary file).
pub fn write(wind: &Path, waves: &Path, out: &Path) -> Result<(), CombineError> {
    let part = out.with_extension("part");
    {
        use std::io::Write;
        let mut f = std::io::BufWriter::new(std::fs::File::create(&part)?);
        for p in [wind, waves] {
            let mut r = std::fs::File::open(p)?;
            std::io::copy(&mut r, &mut f)?;
        }
        f.flush()?;
    }
    std::fs::rename(&part, out)?;
    Ok(())
}

/// Check the pair, write the combined file next to the wind file and return its path.
pub fn combine(a: &Path, b: &Path) -> Result<PathBuf, CombineError> {
    let (wind, waves) = pair(a, b)?;
    let out = combined_path(&wind);
    write(&wind, &waves, &out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::{encode, PROCESS_ECMWF_IFS, PROCESS_ECMWF_WAM};
    use crate::{Field, GribFile, Grid, Param};

    fn field(param: Param, discipline: u8, category: u8, number: u8, surface: u8, run: i64, step_h: i64, v: f32) -> Field {
        let g = Grid::regular(9, 9, -44.0, 170.0, 0.5, 0.5);
        Field {
            param, discipline, category, number, surface_type: surface, surface_value: if surface == 103 { 10.0 } else { 0.0 }, centre: 98,
            ref_time: run, valid_time: run + step_h * 3600, member: None, grid: g, values: vec![v; 81], local: None,
        }
    }

    #[test]
    fn wind_and_waves_in_one_file() {
        let run = 1_792_108_800;
        let dir = std::env::temp_dir().join(format!("racenav-combine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wind = dir.join("ECMWF_IFS_test.grb2");
        let waves = dir.join("ECMWF_wave_test.grb2");
        let mut w = Vec::new();
        let mut v = Vec::new();
        for h in [0, 3, 6] {
            w.extend(encode(&field(Param::WindU, 0, 2, 2, 103, run, h, 5.0), PROCESS_ECMWF_IFS));
            w.extend(encode(&field(Param::WindV, 0, 2, 3, 103, run, h, -2.0), PROCESS_ECMWF_IFS));
            v.extend(encode(&field(Param::WaveHeight, 10, 0, 3, 1, run, h, 1.5), PROCESS_ECMWF_WAM));
        }
        std::fs::write(&wind, &w).unwrap();
        std::fs::write(&waves, &v).unwrap();

        // either order
        let out = combine(&waves, &wind).unwrap();
        assert_eq!(out.file_name().unwrap().to_string_lossy(), "ECMWF_IFS_test+waves.grb2");
        let g = GribFile::open(&out).unwrap();
        assert!(g.skipped.is_empty(), "{:?}", g.skipped);
        assert_eq!(g.times(Param::WindU).len(), 3);
        assert_eq!(g.times(Param::WaveHeight).len(), 3);
        assert!((g.value(Param::WaveHeight, -42.0, 172.0, run + 3 * 3600).unwrap() - 1.5).abs() < 0.01);
        assert_eq!(g.model().0, "ECMWF IFS + waves");
        let info = scan(&out).unwrap();
        assert!(info.label.contains("+waves"), "{}", info.label);

        // two wind files, or a different run, are refused
        assert!(matches!(pair(&wind, &wind), Err(CombineError::Unsuitable(_))));
        let later = dir.join("ECMWF_wave_later.grb2");
        std::fs::write(&later, encode(&field(Param::WaveHeight, 10, 0, 3, 1, run + 43_200, 0, 1.0), PROCESS_ECMWF_WAM)).unwrap();
        let e = pair(&wind, &later).unwrap_err().to_string();
        assert!(e.contains("runs differ"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
