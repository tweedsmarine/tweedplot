//! S-57 ENC cells for TweedPlot: the ISO 8211 reader, the cell reader (with updates),
//! the IHO object and attribute catalogue, the chart's object classes, and the writer
//! that turns a cell into a GeoPackage laid out like the LINZ layers.
//!
//! Licensed MIT OR Apache-2.0 (unlike the GPL TweedPlot app), so the TweedPlot S-63
//! Charts plugin can use it too.

pub mod catalogue;
pub mod cell;
pub mod classes;
pub mod files;
pub mod gpkg;
pub mod iso8211;
pub mod texts;

pub use files::{find_cells, find_cells_except, CellFiles};
pub use gpkg::{band_for, write_gpkg, write_into, GpkgTable};
