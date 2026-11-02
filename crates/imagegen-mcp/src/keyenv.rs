//! API keys saved by HOI4 Mod Setup, read from the operating system credential store.
//!
//! An AI client starts `hoi4-imagegen-mcp` itself, so Setup cannot hand it a key through the environment without
//! writing the secret into a project file, which it never does. Setup keeps the keys in the user's credential store
//! (Windows Credential Manager, macOS Keychain, Linux Secret Service) under its own service name, and this module
//! reads them when the matching environment variable is not set. A variable that is set always wins. Nothing is
//! written, cached on disk or logged.

use imagegen_core::config::EnvSource;

/// Setup's credential-store service name.
pub const SETUP_SERVICE: &str = "com.klimpaskov.hoi4-mod-setup";

/// The Setup credential field that holds the key for an environment variable, if Setup manages one.
pub fn setup_field(var: &str) -> Option<&'static str> {
    match var {
        "OPENAI_API_KEY" => Some("openai_api_key"),
        "GEMINI_API_KEY" | "GOOGLE_API_KEY" => Some("gemini_api_key"),
        "FAL_KEY" => Some("fal_key"),
        "COMFYUI_API_KEY" => Some("comfyui_api_key"),
        "REPLICATE_API_TOKEN" => Some("replicate_api_token"),
        "BFL_API_KEY" => Some("bfl_api_key"),
        "STABILITY_API_KEY" => Some("stability_api_key"),
        "IDEOGRAM_API_KEY" => Some("ideogram_api_key"),
        "RECRAFT_API_KEY" => Some("recraft_api_key"),
        "LEONARDO_API_KEY" => Some("leonardo_api_key"),
        "XAI_API_KEY" => Some("xai_api_key"),
        "TOGETHER_API_KEY" => Some("together_api_key"),
        "OPENROUTER_API_KEY" => Some("openrouter_api_key"),
        "OPENAI_COMPATIBLE_API_KEY" => Some("openai_compatible_api_key"),
        "HF_TOKEN" => Some("hf_token"),
        _ => None,
    }
}

/// The credential-store account Setup writes a field under (its `image_generation_credential_reference`).
pub fn setup_account(field: &str) -> String {
    format!("credential://image_generation_api_key/hoi4_imagegen_{field}")
}

/// An environment that falls back to Setup's credential store for the provider keys.
pub struct SetupKeyEnv<E: EnvSource> {
    inner: E,
}

impl<E: EnvSource> SetupKeyEnv<E> {
    /// Wrap an environment.
    pub fn new(inner: E) -> Self {
        SetupKeyEnv { inner }
    }
}

impl<E: EnvSource> EnvSource for SetupKeyEnv<E> {
    fn get(&self, key: &str) -> Option<String> {
        if let Some(v) = self.inner.get(key) {
            return Some(v);
        }
        let field = setup_field(key)?;
        keyring::Entry::new(SETUP_SERVICE, &setup_account(field)).ok()?.get_password().ok().filter(|v| !v.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::config::MapEnv;

    #[test]
    fn only_provider_keys_map_to_setup_fields() {
        assert_eq!(setup_field("OPENAI_API_KEY"), Some("openai_api_key"));
        assert_eq!(setup_field("GOOGLE_API_KEY"), Some("gemini_api_key"));
        assert_eq!(setup_field("FAL_KEY"), Some("fal_key"));
        assert_eq!(setup_field("COMFYUI_API_KEY"), Some("comfyui_api_key"));
        assert_eq!(setup_field("REPLICATE_API_TOKEN"), Some("replicate_api_token"));
        assert_eq!(setup_field("AWS_SECRET_ACCESS_KEY"), None, "Bedrock is deferred");
        assert_eq!(setup_field("HF_TOKEN"), Some("hf_token"));
        assert_eq!(setup_field("HOME"), None);
        assert_eq!(setup_account("fal_key"), "credential://image_generation_api_key/hoi4_imagegen_fal_key");
    }

    #[test]
    fn a_set_variable_wins_and_unrelated_names_never_touch_the_store() {
        let env = SetupKeyEnv::new(MapEnv::from_pairs(&[("OPENAI_API_KEY", "sk-synthetic"), ("HOME", "/h")]));
        assert_eq!(env.get("OPENAI_API_KEY").as_deref(), Some("sk-synthetic"));
        assert_eq!(env.get("HOME").as_deref(), Some("/h"));
        assert_eq!(env.get("PATH_NOT_SET"), None);
    }
}
