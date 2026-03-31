use std::collections::HashMap;

pub trait KeyResolver: Send + Sync {
    fn resolve_api_key(&self, provider: &str) -> Option<String>;
}

#[derive(Debug, Clone, Default)]
pub struct StaticKeyResolver {
    keys: HashMap<String, String>,
}

impl StaticKeyResolver {
    pub fn new() -> Self {
        Self {
            keys: HashMap::new(),
        }
    }

    pub fn with_key(mut self, provider: impl Into<String>, api_key: impl Into<String>) -> Self {
        self.keys.insert(provider.into(), api_key.into());
        self
    }

    pub fn insert(&mut self, provider: impl Into<String>, api_key: impl Into<String>) {
        self.keys.insert(provider.into(), api_key.into());
    }
}

impl KeyResolver for StaticKeyResolver {
    fn resolve_api_key(&self, provider: &str) -> Option<String> {
        self.keys.get(provider).cloned()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EnvKeyResolver;

impl KeyResolver for EnvKeyResolver {
    fn resolve_api_key(&self, provider: &str) -> Option<String> {
        let env_name = format!(
            "{}_API_KEY",
            provider.trim().to_ascii_uppercase().replace('-', "_")
        );
        std::env::var(env_name)
            .ok()
            .filter(|v| !v.trim().is_empty())
    }
}
