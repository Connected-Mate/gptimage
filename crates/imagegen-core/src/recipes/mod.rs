//! Image recipes ported from the Agentic HOI4 Modding Kit's Python tools, plus the Rust-only steps the kit lacks.
//!
//! One file per recipe. Everything here is a pure function of pixels and parameters, deterministic on every
//! platform, and independent of the MCP engine, which wires the recipes in. [`pillow`] holds the Pillow-compatible
//! primitives the ports share.
//!
//! | Recipe | Source | Module |
//! |---|---|---|
//! | Report event card (tilt, sepia, grain, shadow) | `process_report_event_image.py` | [`report_card`] |
//! | Achievement triplet | `process_achievement_icons.py` | [`achievement`] |
//! | Advisor card | `create_advisor_icon.py` | [`advisor`] |
//! | Portrait source crop 156x210 | `extract_portrait_source_crop.py` | [`portrait`], `face` (feature `face-detect`) |
//! | Flag ladder 82x52, 41x26, 10x7 | Rust only | [`flags`] |
//! | Focus `_shine` registration block | Rust only | [`focus_shine`] |
//! | Selected and disabled frame 2 | Rust only, fitted to vanilla | [`frames`] |
//! | Horizontal frame strips | Rust only | [`strip`] |
//! | Flatten on colour | Rust only | [`flatten`] |
//! | Companions the game looks up (small emblem, text icon, miniature, half size) | Rust only, fitted to vanilla | [`derived`] |
//! | Unit art: silhouette to vanilla material, NATO box frame 2 | Rust only, fitted to vanilla | [`unit_art`] |
//!
//! `convert_to_dds.py` is already [`crate::dds::write_a8r8g8b8`].

pub mod achievement;
pub mod advisor;
pub mod derived;
#[cfg(feature = "face-detect")]
pub mod face;
pub mod flags;
pub mod flatten;
pub mod focus_shine;
pub mod frames;
pub mod pillow;
pub mod portrait;
pub mod report_card;
pub mod strip;
pub mod unit_art;
