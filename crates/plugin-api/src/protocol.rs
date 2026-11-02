//! The methods of the TweedPlot S-63 Charts plugin (interface version [`crate::API_VERSION`]).
//!
//! | Method | Params | Result (body) |
//! |---|---|---|
//! | `hello` | `{api, app_version, data_dir, enc_dir}` | `{name, version, api}` |
//! | `status` | `{}` | installation code, activation, user permit, SA key, permits by data server, installed cells, messages |
//! | `activate` | `{code}` | `{user_permit}` |
//! | `deactivate` | `{}` | `{release_code}` (null if none) |
//! | `install_permits` | `{text}` | `[Msg]` |
//! | `remove_permits` | `{ds}` | `bool` |
//! | `install_key` | `{name}`, body = the file | `[Msg]` |
//! | `import` | `{path}` | `{report: [Msg]}`, with `progress` events `{done, total, cell}` |
//! | `remove_cells` | `{cell}` ("" = all) | `{}` |
//! | `open` | `{}` | [`Opened`], with `progress` events while cells are converted |
//! | `rows` | [`RowsQuery`] | body: rows ([`crate::rows`]) |
//! | `feature` | [`FeatureQuery`] | body: 0 or 1 row |
//! | `median` | `{db, table, col}` | `{value}` (null if none) |
//! | `set_today` | `{date}` (YYYYMMDD or null) | `{}` (for the IHO S-64 tests) |
//!
//! Nothing secret crosses: no HW_ID, cell keys or M_KEY, and never a whole database. The
//! app asks for rows of one table in one area, as it draws them.

use serde::{Deserialize, Serialize};

/// A report line (S-63 messages carry their SSE code in the text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Msg {
    pub cell: String,
    /// "ok", "info", "warn" or "error"
    pub level: String,
    pub text: String,
}

/// A table the plugin serves: which of its databases, and the table itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableMeta {
    pub db: u32,
    pub table: enc_s57::GpkgTable,
}

/// A message about an installed cell, with where it is (for the chart's warning banner).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellNote {
    pub cell: String,
    pub text: String,
    /// [w, s, e, n], when known
    pub bbox: Option<[f64; 4]>,
}

/// What `open` returns: the tables to draw, and the messages from opening them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Opened {
    pub tables: Vec<TableMeta>,
    /// cells shown with a warning (SSE 25 expired permit, SSE 27 not up to date)
    pub warnings: Vec<CellNote>,
    /// cells that couldn't be opened, and why
    pub notes: Vec<CellNote>,
    pub cells: usize,
}

/// `rows`: the rows of one table in an area, with the columns asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowsQuery {
    pub db: u32,
    pub table: String,
    /// [w, s, e, n]
    pub bbox: [f64; 4],
    pub cols: Vec<String>,
}

/// `feature`: one row by its key, with the columns asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureQuery {
    pub db: u32,
    pub table: String,
    pub fid: i64,
    pub cols: Vec<String>,
}
