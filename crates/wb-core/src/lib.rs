//! Wise Bucket core library.
//!
//! Configuration (layered, with origins), data roots, structured errors, the
//! managed PostgreSQL + pgvector lifecycle, migrations and sessions.

pub mod config;
pub mod db;
pub mod error;
pub mod fsutil;
pub mod paths;
pub mod roots;
pub mod session;

/// Version of Wise Bucket (workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use error::{ErrorKind, Result, WbError};
