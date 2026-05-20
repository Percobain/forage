use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub fetch: FetchConfig,
    #[serde(default)]
    pub rate_limits: RateLimitConfig,
    #[serde(default)]
    pub apollo: ApolloConfig,
    #[serde(default)]
    pub bing: BingConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GeneralConfig {
    #[serde(default = "default_cache_db")]
    pub cache_db: String,
    #[serde(default = "default_cookies_dir")]
    pub cookies_dir: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_cache_ttl")]
    pub default_cache_ttl_seconds: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FetchConfig {
    #[serde(default = "default_user_agent_profile")]
    pub direct_user_agent_profile: String,
    #[serde(default = "default_timeout")]
    pub default_timeout_seconds: u64,
    #[serde(default = "default_concurrency")]
    pub parallel_concurrency: usize,
    #[serde(default)]
    pub jina_api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RateLimitConfig {
    #[serde(default = "default_linkedin_min_delay")]
    pub linkedin_min_delay_ms: u64,
    #[serde(default = "default_linkedin_max_delay")]
    pub linkedin_max_delay_ms: u64,
    #[serde(default = "default_linkedin_daily_cap")]
    pub linkedin_daily_cap: usize,
    #[serde(default = "default_x_min_delay")]
    pub x_min_delay_ms: u64,
    #[serde(default = "default_x_max_delay")]
    pub x_max_delay_ms: u64,
    #[serde(default = "default_x_daily_cap")]
    pub x_daily_cap: usize,
    #[serde(default = "default_ig_min_delay")]
    pub instagram_min_delay_ms: u64,
    #[serde(default = "default_ig_max_delay")]
    pub instagram_max_delay_ms: u64,
    #[serde(default = "default_ig_daily_cap")]
    pub instagram_daily_cap: usize,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ApolloConfig {
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct BingConfig {
    #[serde(default)]
    pub api_key: String,
}

fn default_cache_db() -> String { "~/.forage/cache.db".to_string() }
fn default_cookies_dir() -> String { "~/.forage/cookies".to_string() }
fn default_log_level() -> String { "info".to_string() }
fn default_cache_ttl() -> u64 { 86400 }
fn default_user_agent_profile() -> String { "chrome_120".to_string() }
fn default_timeout() -> u64 { 30 }
fn default_concurrency() -> usize { 10 }
fn default_linkedin_min_delay() -> u64 { 8000 }
fn default_linkedin_max_delay() -> u64 { 15000 }
fn default_linkedin_daily_cap() -> usize { 80 }
fn default_x_min_delay() -> u64 { 6000 }
fn default_x_max_delay() -> u64 { 12000 }
fn default_x_daily_cap() -> usize { 200 }
fn default_ig_min_delay() -> u64 { 12000 }
fn default_ig_max_delay() -> u64 { 20000 }
fn default_ig_daily_cap() -> usize { 60 }

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            cache_db: default_cache_db(),
            cookies_dir: default_cookies_dir(),
            log_level: default_log_level(),
            default_cache_ttl_seconds: default_cache_ttl(),
        }
    }
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            direct_user_agent_profile: default_user_agent_profile(),
            default_timeout_seconds: default_timeout(),
            parallel_concurrency: default_concurrency(),
            jina_api_key: String::new(),
        }
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            linkedin_min_delay_ms: default_linkedin_min_delay(),
            linkedin_max_delay_ms: default_linkedin_max_delay(),
            linkedin_daily_cap: default_linkedin_daily_cap(),
            x_min_delay_ms: default_x_min_delay(),
            x_max_delay_ms: default_x_max_delay(),
            x_daily_cap: default_x_daily_cap(),
            instagram_min_delay_ms: default_ig_min_delay(),
            instagram_max_delay_ms: default_ig_max_delay(),
            instagram_daily_cap: default_ig_daily_cap(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            fetch: FetchConfig::default(),
            rate_limits: RateLimitConfig::default(),
            apollo: ApolloConfig::default(),
            bing: BingConfig::default(),
        }
    }
}

/// Expand ~ to home directory
pub fn expand_tilde(path: &str) -> PathBuf {
    if path.starts_with("~/") || path.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&path[2..]);
        }
    }
    PathBuf::from(path)
}

impl Config {
    pub fn load() -> Self {
        let config_path = expand_tilde("~/.forage/config.toml");
        Self::load_from(&config_path)
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse config: {e}. Using defaults.");
                Config::default()
            }),
            Err(_) => {
                tracing::info!("No config file found at {path:?}. Using defaults.");
                Config::default()
            }
        }
    }

    pub fn cache_db_path(&self) -> PathBuf {
        expand_tilde(&self.general.cache_db)
    }

    pub fn cookies_dir_path(&self) -> PathBuf {
        expand_tilde(&self.general.cookies_dir)
    }
}
