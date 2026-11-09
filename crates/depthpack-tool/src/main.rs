//! `tweedplot-depthpack`: TweedPlot's survey depth packages from LINZ's published depth
//! models (LINZ Data Service, Hydrographic & Maritime › Bathymetric Surfaces, CC BY 4.0).
//!
//! ```text
//! tweedplot-depthpack catalogue  --dir D                 list the depth models (no key needed)
//! tweedplot-depthpack export     --dir D --key KEY [--only 122423,116625]
//!                                                         export and download them, and the survey index
//! tweedplot-depthpack build      --dir D --edition 2026-11 [--national-max-zoom 14] [--only ...]
//!                                                         make the packages and manifest.json in D/out
//! tweedplot-depthpack all        --dir D --key KEY --edition 2026-11
//! tweedplot-depthpack check      --dir D [--at LON,LAT] [--png Z/X/Y]
//!                                                         check D/out against its manifest; depth at a place; draw a tile
//! ```
//! The key is a LINZ Data Service API key (free account; Settings › API keys, with
//! "Full access to exports" or the export scope). It can also be given as LDS_API_KEY.
//! Downloads are kept in D/src and only fetched again when LINZ publishes a new version.

mod build;
mod geotiff;
mod lds;
mod regions;

use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "tweedplot-depthpack: TweedPlot depth packages from LINZ's depth models\n\n\
         usage:\n  tweedplot-depthpack catalogue --dir D\n  tweedplot-depthpack export --dir D --key KEY [--only id,id]\n  \
         tweedplot-depthpack build --dir D --edition YYYY-MM [--national-max-zoom 14] [--only id,id]\n  \
         tweedplot-depthpack all --dir D --key KEY --edition YYYY-MM\n  \
         tweedplot-depthpack check --dir D [--at LON,LAT] [--png Z/X/Y]\n\n\
         The key can also be in LDS_API_KEY. D holds catalogue.json, src/ (downloads) and out/ (packages)."
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().cloned() else { usage() };
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let dir = PathBuf::from(opt("--dir").unwrap_or_else(|| ".".into()));
    let key = opt("--key").or_else(|| std::env::var("LDS_API_KEY").ok());
    let only: Option<Vec<i64>> = opt("--only").map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect());
    let edition = opt("--edition");
    let nat_max: u8 = opt("--national-max-zoom").and_then(|s| s.parse().ok()).unwrap_or(14);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{}: {e}", dir.display());
        std::process::exit(1);
    }
    let r = match cmd.as_str() {
        "catalogue" => lds::catalogue(&dir, key.as_deref()).map(|c| println!("{} depth models listed in {}", c.len(), dir.join("catalogue.json").display())),
        "export" => match key {
            Some(k) => lds::export_all(&dir, &k, only.as_deref()),
            None => Err("export needs --key (or LDS_API_KEY)".into()),
        },
        "build" => match edition {
            Some(ed) => build::build(&dir, &ed, nat_max, only.as_deref()),
            None => Err("build needs --edition, e.g. 2026-11".into()),
        },
        "all" => match (key, edition) {
            (Some(k), Some(ed)) => lds::catalogue(&dir, Some(&k))
                .and_then(|_| lds::export_all(&dir, &k, only.as_deref()))
                .and_then(|_| build::build(&dir, &ed, nat_max, only.as_deref())),
            _ => Err("all needs --key and --edition".into()),
        },
        "check" => build::check(&dir, opt("--at").as_deref(), opt("--png").as_deref()),
        _ => usage(),
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
