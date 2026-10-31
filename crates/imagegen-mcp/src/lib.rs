//! Operations and the MCP stdio server for HOI4 graphical assets.
//!
//! [`Engine`] implements the thirteen tools once; [`server::ImageServer`] exposes them over MCP and the
//! `hoi4-imagegen` command-line tool calls the same functions.

#![forbid(unsafe_code)]

pub mod api;
pub mod engine;
pub mod plugin;
pub mod server;

pub use engine::{Engine, EngineError, RunMode};
