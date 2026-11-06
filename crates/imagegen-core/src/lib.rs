//! HOI4 asset logic with no network and no secrets.
//!
//! Families and profiles, the deterministic processing chain, validation, uncompressed A8R8G8B8 DDS and
//! 32-bit TGA writers, naming, never-overwritten versioned outputs, plans, a hash-chained provenance ledger and
//! the install gate. Providers live in `imagegen-providers`; the tool surface lives in `imagegen-mcp`.
//!
//! Derived in intent from Connected-Mate/gptimage (MIT); the code is a Rust rewrite. See `UPSTREAM.md`.

#![forbid(unsafe_code)]

pub mod animation;
pub mod audit;
pub mod builtin;
pub mod calibrate;
pub mod chain;
pub mod config;
pub mod dds;
pub mod derive;
pub mod diversity;
pub mod error;
pub mod export;
pub mod family;
pub mod fsio;
pub mod gate;
pub mod hashing;
pub mod ledger;
pub mod lock;
pub mod lookup;
pub mod measurement;
pub mod naming;
pub mod pack;
pub mod pixels;
pub mod plan;
pub mod process;
pub mod profile;
pub mod recipes;
pub mod records;
pub mod refs;
pub mod refselect;
pub mod resize;
pub mod reuse;
pub mod rubric;
pub mod set;
pub mod sprite;
pub mod style;
pub mod styledesc;
pub mod stylesrc;
pub mod subfamily;
pub mod tga;
pub mod validate;
pub mod vanillamatch;

pub use error::{Error, Result, RetryClass};
pub use family::{Automation, Family, Group, GuiTextureKind};
pub use pixels::Rgba;
