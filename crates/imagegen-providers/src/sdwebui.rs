//! Local Stable Diffusion WebUI servers (`automatic1111`, `forge`): the `/sdapi/v1/txt2img` and `img2img` API on a loopback endpoint.

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource};

use crate::registry::ProviderRegistry;
use crate::transport::HttpTransport;

/// Register this module's providers unless the project config disables them.
pub fn register(_reg: &mut ProviderRegistry, _config: &Config, _env: &dyn EnvSource, _transport: Arc<dyn HttpTransport>) {}
