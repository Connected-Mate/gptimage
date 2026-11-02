//! Image providers for hoi4-imagegen-mcp.
//!
//! One trait ([`ImageProvider`]), four adapters (OpenAI, Gemini, fal, ComfyUI) and a destination-checked HTTP
//! transport. Credentials come from environment variables only. Model ids, limits and prices live in dated JSON
//! tables under `data/`, never in code paths. No provider here is the undocumented ChatGPT-login backend: that
//! lives in a separate opt-in crate (see docs/chatgpt-login.md) that nothing in this crate depends on.

#![forbid(unsafe_code)]
// ProviderError carries the redacted request context every caller reports; boxing it would only move the cost.
#![allow(clippy::result_large_err)]

pub mod comfyui;
pub mod common;
pub mod fal;
pub mod gemini;
pub mod openai;
pub mod registry;
pub mod sizing;
pub mod transport;
pub mod types;

pub use registry::ProviderRegistry;
pub use transport::{FixtureTransport, HostPolicy, HttpTransport, UreqTransport};
pub use types::{
    CallContext, CancelToken, CostEstimate, ErrorKind, Funding, ImageProvider, ImageRequest, JobRef, ProviderDescriptor, ProviderError,
    ProviderOutput, Reconciled,
};
