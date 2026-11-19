//! Raster images drawn on the chart: KAP/BSB raster charts, GeoTIFFs (the user's own or
//! downloaded), and Sentinel-2 satellite images fetched from Element 84's Earth Search
//! catalogue on AWS. Every image is drawn as web-mercator PNG tiles for the map, under or
//! over the chart.
//!
//! Images are not surveyed: they are only as good as their georeferencing.

pub mod geo;
pub mod kap;
pub mod render;
pub mod s2;
pub mod source;
pub mod store;
pub mod tiff;
pub mod tiffw;

use std::fmt;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(s: impl Into<String>) -> Result<T> {
    Err(Error(s.into()))
}

pub use render::Raster;
