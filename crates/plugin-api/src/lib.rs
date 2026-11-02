//! The interface between TweedPlot and its plugins.
//!
//! A plugin is a separate program. TweedPlot starts it as a child process and the two
//! exchange **frames** over the child's standard input and output:
//!
//! ```text
//! u32 LE  header length   u32 LE  body length   header (JSON)   body (bytes)
//! ```
//!
//! - A request (app → plugin): `{"id": 7, "method": "rows", "params": {…}}`, body optional.
//! - Its reply (plugin → app): `{"id": 7, "ok": true, "result": {…}}` (or `"ok": false,
//!   "error": "…"`), body optional (table rows travel as bytes: see [`rows`]).
//! - An event (plugin → app, unasked): `{"event": "progress", "data": {…}}`.
//!
//! The methods of the S-63 Charts plugin are listed in [`protocol`]. Plugins are
//! distributed as signed packages: see [`package`].
//!
//! Licensed MIT OR Apache-2.0, so plugins under any licence can use it.

pub mod client;
pub mod frame;
pub mod package;
pub mod protocol;
pub mod rows;
pub mod server;

pub use client::PluginClient;
pub use rows::{Row, Value};

/// The interface version. Raised when a change would break the other side; the app
/// refuses a plugin built for another version and says which update is needed.
pub const API_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the plugin sent something that isn't a message: {0}")]
    Protocol(String),
    #[error("the plugin stopped")]
    Closed,
    #[error("the plugin didn't answer in time ({0})")]
    Timeout(String),
    #[error("{0}")]
    Plugin(String),
}
