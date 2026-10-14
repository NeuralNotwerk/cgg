//! Dead-code mode: CLI-side configuration and rendering.
//!
//! The analysis itself lives in `cgg_resolve::deadcode`. This module
//! owns only what the binary is responsible for: turning flags into
//! options, and turning a report into bytes.

pub mod config;
pub mod report;
