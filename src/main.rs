mod cache;
mod config;
mod fetch;
mod rate_limit;
mod search;
mod social;
mod tools;

use cache::Cache;
use config::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let config = Config::load();

    // Initialize logging to file (NOT stdout — stdout is for MCP transport)
    let log_dir = config::expand_tilde("~/.forage/logs");
    std::fs::create_dir_all(&log_dir).ok();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "forage.log");
    tracing_subscriber::fmt()
        .with_writer(file_appender)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.general.log_level.parse().unwrap_or_default()),
        )
        .init();

    tracing::info!("forage starting up");

    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && args[1] == "test-fetch" {
        run_test_fetch(&config, args.get(2).map(|s| s.as_str())).await;
    } else if args.len() > 1 && args[1] == "doctor" {
        run_doctor(&config);
    } else {
        // Default: run MCP server over stdio
        run_mcp_server(config).await;
    }
}

async fn run_mcp_server(config: Config) {
    let cache_path = config.cache_db_path();
    let cache = Arc::new(Cache::open(&cache_path).expect("Failed to open cache database"));

    let server = tools::ForageServer::new(config, cache);

    tracing::info!("Starting MCP server on stdio");

    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport)
        .await
        .expect("Failed to start MCP server");

    service.waiting().await.expect("MCP server error");
}

async fn run_test_fetch(config: &Config, url: Option<&str>) {
    let url = url.unwrap_or("https://example.com");
    eprintln!("Fetching {url}...");

    let cache_path = config.cache_db_path();
    let cache = Cache::open(&cache_path).expect("Failed to open cache");

    let cache_key = Cache::cache_key("fetch", url, "");
    if let Some(entry) = cache.get(&cache_key) {
        eprintln!("--- CACHED (fetched at {}) ---", entry.fetched_at);
        eprintln!("{}", &entry.content[..entry.content.len().min(500)]);
        return;
    }

    let jina_key = if config.fetch.jina_api_key.is_empty() {
        None
    } else {
        Some(config.fetch.jina_api_key.as_str())
    };

    match fetch::fetch_with_fallback(url, config.fetch.default_timeout_seconds, jina_key).await {
        Ok(result) => {
            eprintln!(
                "--- {} via {} (status {}) ---",
                result.url, result.method, result.status_code
            );
            eprintln!("{}", &result.content[..result.content.len().min(500)]);

            cache.put(
                &cache_key,
                url,
                &result.method,
                &result.content,
                Some(result.status_code as i32),
                config.general.default_cache_ttl_seconds,
            );
            eprintln!("\n[Cached for {}s]", config.general.default_cache_ttl_seconds);
        }
        Err(e) => {
            eprintln!("Fetch failed: {e}");
            std::process::exit(1);
        }
    }
}

fn run_doctor(config: &Config) {
    eprintln!("forage doctor v0.1.0\n");

    // Check config
    let config_path = config::expand_tilde("~/.forage/config.toml");
    if config_path.exists() {
        eprintln!("[OK] Config file: {}", config_path.display());
    } else {
        eprintln!("[WARN] No config file at {}. Using defaults.", config_path.display());
    }

    // Check cache DB
    let cache_path = config.cache_db_path();
    match Cache::open(&cache_path) {
        Ok(_) => eprintln!("[OK] Cache DB: {}", cache_path.display()),
        Err(e) => eprintln!("[ERR] Cache DB: {e}"),
    }

    // Check cookies
    let cookies_dir = config.cookies_dir_path();
    if cookies_dir.exists() {
        for platform in &["linkedin", "x", "instagram"] {
            let cookie_file = cookies_dir.join(format!("{platform}.json"));
            if cookie_file.exists() {
                eprintln!("[OK] {platform} cookies: {}", cookie_file.display());
            } else {
                eprintln!("[--] {platform} cookies: not configured");
            }
        }
    } else {
        eprintln!("[WARN] Cookies directory does not exist: {}", cookies_dir.display());
    }

    // Check Apollo
    if !config.apollo.api_key.is_empty() {
        eprintln!("[OK] Apollo API key: configured");
    } else {
        eprintln!("[--] Apollo API key: not configured");
    }

    // Check Jina
    if !config.fetch.jina_api_key.is_empty() {
        eprintln!("[OK] Jina API key: configured (higher rate limits)");
    } else {
        eprintln!("[--] Jina API key: not configured (using free tier)");
    }

    eprintln!("\nDone.");
}
