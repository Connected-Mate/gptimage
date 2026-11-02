//! Stability AI REST API (`stability`): synchronous Stable Image generation.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource};

use crate::registry::ProviderRegistry;
use crate::transport::HttpTransport;

/// Register this module's providers unless the project config disables them.
pub fn register(_reg: &mut ProviderRegistry, _config: &Config, _env: &dyn EnvSource, _transport: Arc<dyn HttpTransport>) {}
