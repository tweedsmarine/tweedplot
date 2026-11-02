# Third-party notices

TweedPlot includes or uses the following. Each keeps its own licence and copyright.

## Bundled data

| Item | Source | Licence |
| --- | --- | --- |
| `app/src-tauri/resources/world_depth.gpkg`: world bathymetry bands | Natural Earth 10 m bathymetry (https://www.naturalearthdata.com, nvkelso/natural-earth-vector) | Public domain |
| `app/src-tauri/resources/polars/*`: example polar tables | TweedsMarine and the boats named in the files | Same as the code |
| `app/public/data/land-*.geojson`: land outlines | Derived from OpenStreetMap data via simonepri/geo-maps; © OpenStreetMap contributors (https://www.openstreetmap.org/copyright) | ODbL 1.0: distributed with TweedPlot, so the credit is shown on the chart and in About |
| `crates/tides/src/foreman.rs`: tidal constituent tables (frequencies, Doodson numbers, nodal satellites, shallow-water recipes) | M.G.G. Foreman, Institute of Ocean Sciences; as distributed with UTide (D. Codiga; Python port by W. Bowman and E. Firing) | MIT (UTide) |
| `app/src-tauri/resources/s101-pc/PortrayalCatalog`: IHO S-101 Portrayal Catalogue 2.1.0 (chart symbols, line styles, area patterns, day/dusk/night colours and portrayal rules), unchanged | International Hydrographic Organization (IHO), NOAA and the contributors (https://github.com/iho-ohi/S-101_Portrayal-Catalogue) | IHO licence: copy and distribute with the notice kept and IHO, NOAA and the contributors credited; provided "as is", no warranty or liability. Full text: `app/src-tauri/resources/s101-pc/LICENSE-IHO.txt`, also in the installed program and in Manual › Reference › Data sources and credits |
| Atkinson Hyperlegible Next font (`@fontsource/atkinson-hyperlegible-next`) | Braille Institute of America | SIL Open Font License 1.1 |

## Data downloaded by the user at run time (not distributed with TweedPlot)

| Data | Source | Terms |
| --- | --- | --- |
| LINZ charts, Topo50, tide predictions and tide constituents | Toitū Te Whenua Land Information New Zealand | CC BY 4.0: © LINZ. Attribute LINZ when you share anything made from it. |
| OpenStreetMap tiles (optional, off by default, fetched at run time, not bundled) | © OpenStreetMap contributors; subject to https://operations.osmfoundation.org/policies/tiles/ | ODbL 1.0 (data) |
| OpenSeaMap seamarks (optional, off by default, fetched at run time, not bundled) | OpenSeaMap contributors (https://www.openseamap.org); not for navigation | ODbL 1.0 (data) |
| GSHHG coastline 2.3.7 | Wessel and Smith, University of Hawaiʻi and NOAA (https://www.soest.hawaii.edu/pwessel/gshhg) | LGPL 3 |
| ETOPO 2022 | NOAA NCEI, via NOAA CoastWatch ERDDAP | Free to use and redistribute |
| GFS and GFS-Wave | NOAA NCEP (NOMADS) | US Government work, free to use |
| ECMWF IFS and AIFS open data | ECMWF | CC BY 4.0, © ECMWF |
| MUR SST | NASA JPL, via NOAA CoastWatch | Free to use and share; acknowledge NASA JPL / PO.DAAC |
| Himawari-9 SST | JMA, processed by NOAA STAR (ACSPO L3C, GHRSST) | Free and open |
| Chlorophyll-a (VIIRS) | NOAA CoastWatch | Free to use and share |
| HYCOM ESPC | US Navy / HYCOM Consortium (hycom.org) | Public release; acknowledge HYCOM |
| Fishing rules | Ministry for Primary Industries | CC BY 4.0 |
| Weather observations | MetService WIS2, NOAA Aviation Weather Center, NOAA NDBC | Provider terms |
| AIS | aisstream.io, Signal K servers | Provider terms |
| Race tracking | YB Tracking (yb.tl) | Provider terms; positions may be delayed |

Check the provider's current terms before redistributing downloaded data.

## S-63 / IHO

TweedPlot implements the IHO S-63 data protection scheme and S-57 ENC reading from the IHO
specifications. IHO test data, the IHO test credentials (S-64) and the IHO Scheme Administrator
public certificate are **not** redistributed in releases; IHO and S-63 are their owners' marks.
Official TweedsMarine builds use TweedsMarine's own manufacturer credentials, which are not in
this repository.

## Libraries

About 560 Rust crates and 40 JavaScript packages are used. Licences found in the dependency
tree, all compatible with the GPL-3.0-or-later:

- MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, BSL-1.0, 0BSD, CC0-1.0, Unlicense
- MPL-2.0: `cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors`, `serialport`; and two JavaScript packages. Used unmodified; their source is available from their projects.
- Dual-licensed, used under the permissive option: `unescaper` (MIT, not GPL-3.0), `r-efi` (MIT, not LGPL)
- OFL-1.1: the font above

Notable direct dependencies: Tauri (MIT or Apache-2.0), MapLibre GL JS (BSD-3-Clause),
SQLite via `rusqlite` (MIT; SQLite itself is public domain), `marked` (MIT), Vite (MIT).

The full list with licence texts can be regenerated at release time:

```
cargo install cargo-about
cargo about init
cargo about generate about.hbs > licenses-rust.html
cd app && npx license-checker-rseidelsohn --production --csv > ../licenses-js.csv
```

Ship the generated licence texts with each installer, as the MIT, BSD and Apache licences require.
