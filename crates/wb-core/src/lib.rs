//! Wise Bucket core library.
//!
//! Configuration (layered, with origins), data roots and the discovery of the
//! recordings in them, what the engineer said about them (context), structured errors, file-system helpers and the public
//! sample recordings.

pub mod config;
pub mod context;
pub mod error;
pub mod fsutil;
pub mod paths;
pub mod roots;
pub mod samples;

/// Version of Wise Bucket (workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use error::{ErrorKind, Result, WbError};
