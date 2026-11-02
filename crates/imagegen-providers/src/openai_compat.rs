//! OpenAI-compatible image APIs: a generic `openai_compatible` provider (any `/v1/images/generations` server) and presets for xAI (`xai`), Together AI (`together`), OpenRouter (`openrouter`), Microsoft Foundry / Azure OpenAI (`foundry`) and Hugging Face Inference Providers (`huggingface`).

use std::sync::Arc;

use imagegen_core::config::{Config, EnvSource};

use crate::registry::ProviderRegistry;
use crate::transport::HttpTransport;

/// Register this module's providers unless the project config disables them.
pub fn register(_reg: &mut ProviderRegistry, _config: &Config, _env: &dyn EnvSource, _transport: Arc<dyn HttpTransport>) {}
