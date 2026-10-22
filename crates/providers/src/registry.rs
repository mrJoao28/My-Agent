use myagent_core::{AgentError, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::Read,
    path::Path,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Google,
    Ollama,
    Chatgpt,
}
impl ProviderKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Google => "google",
            Self::Ollama => "ollama",
            Self::Chatgpt => "chatgpt",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pricing {
    pub input_usd_per_million_tokens: f64,
    pub output_usd_per_million_tokens: f64,
}
impl Default for Pricing {
    fn default() -> Self {
        Self {
            input_usd_per_million_tokens: 0.0,
            output_usd_per_million_tokens: 0.0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub provider: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
}
fn valid_env(value: &str) -> bool {
    let reserved = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "MYAGENT_HOME",
        "MYAGENT_CONFIG_DIR",
        "MYAGENT_WORKSPACE_ROOTS",
        "MYAGENT_API_TOKEN",
        "MYAGENT_SANDBOX_IMAGE",
        "MYAGENT_ENABLE_CHATGPT",
    ];
    value.len() <= 200
        && !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_uppercase() || (i > 0 && b.is_ascii_digit()))
        && !reserved.contains(&value)
        && !["NODE_", "BUN_", "LD_", "DYLD_"]
            .iter()
            .any(|p| value.starts_with(p))
}
impl Model {
    pub fn api_key_env(&self) -> Option<String> {
        if let Some(custom) = &self.api_key_env {
            return Some(custom.clone());
        }
        if matches!(self.provider, ProviderKind::Ollama | ProviderKind::Chatgpt) {
            return None;
        }
        let mut normalized = String::new();
        let mut separator = false;
        for c in self.id.chars().flat_map(char::to_uppercase) {
            if c.is_ascii_alphanumeric() {
                if separator && !normalized.is_empty() {
                    normalized.push('_');
                }
                separator = false;
                normalized.push(c);
            } else {
                separator = true;
            }
        }
        Some(format!(
            "MYAGENT_{}_{}_API_KEY",
            self.provider.name().to_uppercase(),
            normalized
        ))
    }
}
#[derive(Clone, Debug)]
pub struct ModelRegistry {
    models: Vec<Model>,
    explicit_windows: HashSet<String>,
}
impl Default for ModelRegistry {
    fn default() -> Self {
        Self {
            explicit_windows: HashSet::new(),
            models: serde_json::from_str(include_str!("catalog.json"))
                .expect("embedded legacy catalog"),
        }
    }
}
impl ModelRegistry {
    pub const DEFAULT_MODEL: &'static str = "claude-opus-4-6";
    pub fn replace_chatgpt(&mut self, models: Vec<Model>) {
        self.models
            .retain(|model| model.provider != ProviderKind::Chatgpt);
        self.models.extend(
            models
                .into_iter()
                .filter(|m| m.provider == ProviderKind::Chatgpt && m.id.starts_with("chatgpt:")),
        );
    }
    pub fn models(&self) -> &[Model] {
        &self.models
    }
    pub fn find(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }
    pub fn with_custom_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(AgentError::Limit);
        }
        let mut custom: Vec<Model> =
            serde_json::from_slice(bytes).map_err(|_| AgentError::Configuration)?;
        if custom.len() > 1000 {
            return Err(AgentError::Configuration);
        }
        let mut registry = Self::default();
        let mut ids = HashSet::new();
        for model in &mut custom {
            model.id = model.id.trim().to_owned();
            if model.id.is_empty()
                || model.id.len() > 200
                || model.id.starts_with("chatgpt:")
                || model.provider == ProviderKind::Chatgpt
                || model.id.contains('\0')
                || !ids.insert(model.id.clone())
                || model.api_key_env.as_ref().is_some_and(|v| !valid_env(v))
                || model
                    .display_name
                    .as_ref()
                    .is_some_and(|s| s.trim().is_empty() || s.len() > 200)
                || model
                    .context_window
                    .is_some_and(|n| !(1024..=i32::MAX as u32).contains(&n))
                || model.pricing.as_ref().is_some_and(|p| {
                    !p.input_usd_per_million_tokens.is_finite()
                        || p.input_usd_per_million_tokens < 0.0
                        || !p.output_usd_per_million_tokens.is_finite()
                        || p.output_usd_per_million_tokens < 0.0
                })
            {
                return Err(AgentError::Configuration);
            }
            if model.context_window.is_some() {
                registry.explicit_windows.insert(model.id.clone());
            }
            if let Some(base) = registry.models.iter_mut().find(|m| m.id == model.id) {
                if model.context_window.is_none() {
                    model.context_window = base.context_window;
                }
                if model.pricing.is_none() {
                    model.pricing = base.pricing.clone();
                }
                *base = model.clone();
            } else {
                if model.context_window.is_none() {
                    model.context_window = Some(16384);
                }
                if model.pricing.is_none() {
                    model.pricing = Some(Pricing::default());
                }
                registry.models.push(model.clone());
            }
        }
        Ok(registry)
    }
    /// Caminho explícito: não enumera nem abre o home real nos testes.
    pub fn load(path: &Path) -> Result<Self> {
        if path.file_name().and_then(|s| s.to_str()) != Some("models.json") {
            return Err(AgentError::Configuration);
        }
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|_| AgentError::Configuration)?
                .join(path)
        };
        let path = absolute.as_path();
        for parent in path.ancestors() {
            match fs::symlink_metadata(parent) {
                Ok(metadata) => {
                    let linked = metadata.file_type().is_symlink();
                    #[cfg(windows)]
                    let linked = {
                        use std::os::windows::fs::MetadataExt;
                        linked || metadata.file_attributes() & 0x400 != 0
                    };
                    if linked && !myagent_platform::is_system_directory_alias(parent) {
                        return Err(AgentError::Configuration);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Self::default());
                }
                Err(_) => return Err(AgentError::Configuration),
            }
        }
        let file = File::open(path).map_err(|_| AgentError::Configuration)?;
        let metadata = file.metadata().map_err(|_| AgentError::Configuration)?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err(AgentError::Limit);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err(AgentError::Configuration);
            }
        }
        #[cfg(windows)]
        {
            if winapi_util::file::information(&file)
                .map_err(|_| AgentError::Configuration)?
                .number_of_links()
                != 1
            {
                return Err(AgentError::Configuration);
            }
        }
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| AgentError::Configuration)?;
        Self::with_custom_json(&bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextWindow {
    pub tokens: u32,
    pub estimated: bool,
    pub source: &'static str,
}
impl ModelRegistry {
    pub fn resolve_window(&self, id: &str, provider_hint: Option<u32>) -> Result<ContextWindow> {
        let model = self.find(id).ok_or(AgentError::Configuration)?;
        if model.provider == ProviderKind::Chatgpt
            && let Some(tokens) = model.context_window
        {
            return Ok(ContextWindow {
                tokens,
                estimated: false,
                source: "account",
            });
        }
        if self.explicit_windows.contains(id) {
            return Ok(ContextWindow {
                tokens: model.context_window.ok_or(AgentError::Configuration)?,
                estimated: false,
                source: "override",
            });
        }
        let catalog = Self::default();
        if let Some(base) = catalog.find(id) {
            return Ok(ContextWindow {
                tokens: base.context_window.ok_or(AgentError::Configuration)?,
                estimated: false,
                source: "catalog",
            });
        }
        let parts: Vec<_> = id.rsplitn(4, '-').collect();
        if parts.len() == 4
            && parts[0].len() == 2
            && parts[1].len() == 2
            && parts[2].len() == 4
            && parts[..3]
                .iter()
                .all(|s| s.bytes().all(|c| c.is_ascii_digit()))
            && let Some(base) = catalog.find(parts[3])
        {
            return Ok(ContextWindow {
                tokens: base.context_window.ok_or(AgentError::Configuration)?,
                estimated: false,
                source: "family",
            });
        }
        if let Some(tokens) = provider_hint {
            if !(1024..=i32::MAX as u32).contains(&tokens) {
                return Err(AgentError::Configuration);
            }
            return Ok(ContextWindow {
                tokens,
                estimated: false,
                source: "provider",
            });
        }
        Ok(ContextWindow {
            tokens: if model.provider == ProviderKind::Ollama {
                16384
            } else {
                65536
            },
            estimated: true,
            source: "default",
        })
    }
}
