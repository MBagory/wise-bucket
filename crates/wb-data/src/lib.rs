//! Wise Bucket data: recording readers, the columnar store and SQL.
//!
//! Each recording found in a data root is extracted once into the store: one
//! Parquet file per stream, with a common time column `t`. The agent then
//! computes any metric with SQL ([`sql`]), executed by DataFusion over those
//! datasets. Nothing is computed in advance: storage is traded for query speed.

pub mod columns;
pub mod extract;
pub mod rerun;
pub mod sql;
pub mod store;
pub mod tables;

mod db3;
mod mcap_reader;
mod ros2;
mod rows;
mod sink;
mod tabular;
mod tlog;
mod ulog;

/// Version of the extraction output. Bump it whenever the content of extracted
/// streams changes: every recording is then extracted again automatically.
pub const EXTRACTOR_VERSION: u32 = 4;

/// Name of the single stream of a table-like file (CSV, JSON Lines, Parquet, Arrow IPC).
pub const TABLE_STREAM: &str = "table";
