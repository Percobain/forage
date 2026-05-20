use crate::cache::Cache;
use crate::config::Config;
use crate::fetch;
use crate::fetch::crawler;
use crate::rate_limit::{PlatformLimiter, RateLimiterConfig};
use crate::search;
use crate::social;
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct ForageServer {
    config: Config,
    cache: Arc<Cache>,
}

// === Parameter types ===

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FetchUrlParams {
    /// The URL to fetch
    pub url: String,
    /// Whether to use cache (default: true)
    #[serde(default = "default_true")]
    pub use_cache: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CrawlSiteParams {
    /// Domain to crawl (e.g. "example.com" or "https://example.com")
    pub domain: String,
    /// Maximum number of pages to fetch (default: 50)
    #[serde(default = "default_max_pages")]
    pub max_pages: usize,
    /// Maximum crawl depth for BFS link discovery (default: 3)
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchWebParams {
    /// Search query string
    pub query: String,
    /// Maximum results to return (default: 20)
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FetchArchiveParams {
    /// URL to look up in the Wayback Machine
    pub url: String,
    /// Preferred snapshot date (YYYYMMDD format, optional)
    pub prefer_date: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindCompaniesParams {
    /// Keywords to search for companies
    pub keywords: String,
    /// Company size: A=1, B=2-10, C=11-50, D=51-200, E=201-500, F=501-1000, G=1001-5000
    pub size: Option<String>,
    /// Location filter (e.g. "United States")
    pub location: Option<String>,
    /// Max results (default: 50)
    #[serde(default = "default_company_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FetchLinkedInParams {
    /// LinkedIn profile URL (e.g. "https://linkedin.com/in/johndoe")
    pub profile_url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FetchXParams {
    /// X/Twitter handle (with or without @)
    pub handle: String,
    /// Whether to include recent tweets (default: true)
    #[serde(default = "default_true")]
    pub include_tweets: bool,
    /// Number of tweets to fetch (default: 40)
    #[serde(default = "default_tweet_count")]
    pub tweet_count: usize,
}

fn default_true() -> bool { true }
fn default_max_pages() -> usize { 50 }
fn default_max_depth() -> u32 { 3 }
fn default_search_limit() -> usize { 20 }
fn default_company_limit() -> usize { 50 }
fn default_tweet_count() -> usize { 40 }

fn schema_for<T: JsonSchema>() -> serde_json::Map<String, serde_json::Value> {
    let schema = schemars::schema_for!(T);
    let val = serde_json::to_value(schema).unwrap();
    match val {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

// === Tool implementations ===

impl ForageServer {
    pub fn new(config: Config, cache: Arc<Cache>) -> Self {
        Self { config, cache }
    }

    fn tool_list() -> Vec<Tool> {
        vec![
            Tool {
                name: "fetch_url".into(),
                description: "Fetch any URL and return clean markdown content. Uses tiered fallback: direct fetch, then Jina Reader for Cloudflare-protected sites. Results are cached for 24 hours by default.".into(),
                input_schema: schema_for::<FetchUrlParams>().into(),
            },
            Tool {
                name: "crawl_site".into(),
                description: "Crawl a website by discovering pages via sitemap.xml, RSS feeds, or link crawling, then fetch all pages in parallel. Returns markdown content for each page. Respects robots.txt.".into(),
                input_schema: schema_for::<CrawlSiteParams>().into(),
            },
            Tool {
                name: "search_web".into(),
                description: "Search the web using DuckDuckGo. Returns titles, URLs, and snippets as JSON.".into(),
                input_schema: schema_for::<SearchWebParams>().into(),
            },
            Tool {
                name: "fetch_archive".into(),
                description: "Fetch a historical snapshot of a URL from the Internet Archive Wayback Machine.".into(),
                input_schema: schema_for::<FetchArchiveParams>().into(),
            },
            Tool {
                name: "find_companies".into(),
                description: "Find companies matching criteria using Apollo.io API. Requires apollo.api_key in config. Size codes: A=1, B=2-10, C=11-50, D=51-200, E=201-500, F=501-1000, G=1001-5000.".into(),
                input_schema: schema_for::<FindCompaniesParams>().into(),
            },
            Tool {
                name: "fetch_profile_linkedin".into(),
                description: "Fetch a LinkedIn profile. Requires cookies in ~/.forage/cookies/linkedin.json. Rate-limited: 8-15s between requests, max 80/day.".into(),
                input_schema: schema_for::<FetchLinkedInParams>().into(),
            },
            Tool {
                name: "fetch_profile_x".into(),
                description: "Fetch an X/Twitter profile. Requires cookies in ~/.forage/cookies/x.json. Rate-limited: 6-12s between requests, max 200/day.".into(),
                input_schema: schema_for::<FetchXParams>().into(),
            },
        ]
    }

    async fn dispatch_tool(&self, name: &str, args: serde_json::Map<String, serde_json::Value>) -> String {
        let args_val = serde_json::Value::Object(args);

        match name {
            "fetch_url" => {
                let params: FetchUrlParams = match serde_json::from_value(args_val) {
                    Ok(p) => p,
                    Err(e) => return format!("Invalid parameters: {e}"),
                };
                self.handle_fetch_url(params).await
            }
            "crawl_site" => {
                let params: CrawlSiteParams = match serde_json::from_value(args_val) {
                    Ok(p) => p,
                    Err(e) => return format!("Invalid parameters: {e}"),
                };
                self.handle_crawl_site(params).await
            }
            "search_web" => {
                let params: SearchWebParams = match serde_json::from_value(args_val) {
                    Ok(p) => p,
                    Err(e) => return format!("Invalid parameters: {e}"),
                };
                self.handle_search_web(params).await
            }
            "fetch_archive" => {
                let params: FetchArchiveParams = match serde_json::from_value(args_val) {
                    Ok(p) => p,
                    Err(e) => return format!("Invalid parameters: {e}"),
                };
                self.handle_fetch_archive(params).await
            }
            "find_companies" => {
                let params: FindCompaniesParams = match serde_json::from_value(args_val) {
                    Ok(p) => p,
                    Err(e) => return format!("Invalid parameters: {e}"),
                };
                self.handle_find_companies(params).await
            }
            "fetch_profile_linkedin" => {
                let params: FetchLinkedInParams = match serde_json::from_value(args_val) {
                    Ok(p) => p,
                    Err(e) => return format!("Invalid parameters: {e}"),
                };
                self.handle_fetch_profile_linkedin(params).await
            }
            "fetch_profile_x" => {
                let params: FetchXParams = match serde_json::from_value(args_val) {
                    Ok(p) => p,
                    Err(e) => return format!("Invalid parameters: {e}"),
                };
                self.handle_fetch_profile_x(params).await
            }
            _ => format!("Unknown tool: {name}"),
        }
    }

    async fn handle_fetch_url(&self, params: FetchUrlParams) -> String {
        let url = &params.url;

        if params.use_cache {
            let cache_key = Cache::cache_key("fetch", url, "");
            if let Some(entry) = self.cache.get(&cache_key) {
                return entry.content;
            }
        }

        let jina_key = if self.config.fetch.jina_api_key.is_empty() {
            None
        } else {
            Some(self.config.fetch.jina_api_key.as_str())
        };

        match fetch::fetch_with_fallback(url, self.config.fetch.default_timeout_seconds, jina_key).await {
            Ok(result) => {
                let cache_key = Cache::cache_key("fetch", url, "");
                self.cache.put(
                    &cache_key, url, &result.method, &result.content,
                    Some(result.status_code as i32),
                    self.config.general.default_cache_ttl_seconds,
                );
                result.content
            }
            Err(e) => format!("Error fetching {url}: {e}"),
        }
    }

    async fn handle_crawl_site(&self, params: CrawlSiteParams) -> String {
        let jina_key = if self.config.fetch.jina_api_key.is_empty() {
            None
        } else {
            Some(self.config.fetch.jina_api_key.as_str())
        };

        match crawler::crawl_site(
            &params.domain, params.max_pages, params.max_depth,
            self.config.fetch.default_timeout_seconds,
            self.config.fetch.parallel_concurrency, jina_key,
        ).await {
            Ok(result) => {
                let mut output = format!(
                    "# Crawl Results for {}\n\nDiscovery method: {}\nPages found: {}\n\n",
                    result.domain, result.discovery_method, result.pages.len()
                );
                for page in &result.pages {
                    let title = page.title.as_deref().unwrap_or("Untitled");
                    output.push_str(&format!("---\n## {} ({})\n\n{}\n\n", title, page.url, page.content));
                }
                output
            }
            Err(e) => format!("Error crawling {}: {e}", params.domain),
        }
    }

    async fn handle_search_web(&self, params: SearchWebParams) -> String {
        let cache_key = Cache::cache_key("search", &params.query, "");
        if let Some(entry) = self.cache.get(&cache_key) {
            return entry.content;
        }

        match search::duckduckgo::search(&params.query, params.limit, self.config.fetch.default_timeout_seconds).await {
            Ok(results) => {
                let output = serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string());
                self.cache.put(&cache_key, &params.query, "duckduckgo", &output, Some(200), 21600);
                output
            }
            Err(e) => format!("Search error: {e}"),
        }
    }

    async fn handle_fetch_archive(&self, params: FetchArchiveParams) -> String {
        match search::wayback::fetch_archive(
            &params.url, params.prefer_date.as_deref(),
            self.config.fetch.default_timeout_seconds,
        ).await {
            Ok(result) => {
                format!(
                    "# Archived Snapshot\n\nOriginal URL: {}\nSnapshot URL: {}\nCapture Date: {}\n\n---\n\n{}",
                    result.original_url, result.snapshot_url, result.capture_date, result.content
                )
            }
            Err(e) => format!("Archive fetch error: {e}"),
        }
    }

    async fn handle_find_companies(&self, params: FindCompaniesParams) -> String {
        let apollo = match social::apollo::ApolloClient::new(&self.config.apollo.api_key) {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };

        match apollo.find_companies(&params.keywords, params.size.as_deref(), params.location.as_deref(), params.limit).await {
            Ok(companies) => serde_json::to_string_pretty(&companies).unwrap_or_else(|_| "[]".to_string()),
            Err(e) => format!("Error finding companies: {e}"),
        }
    }

    async fn handle_fetch_profile_linkedin(&self, params: FetchLinkedInParams) -> String {
        let cookies_path = self.config.cookies_dir_path().join("linkedin.json");
        let client = match social::linkedin::LinkedInClient::from_cookie_file(&cookies_path) {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };

        let limiter = PlatformLimiter::new(
            RateLimiterConfig {
                platform: "linkedin".to_string(),
                min_delay: Duration::from_millis(self.config.rate_limits.linkedin_min_delay_ms),
                max_delay: Duration::from_millis(self.config.rate_limits.linkedin_max_delay_ms),
                daily_cap: self.config.rate_limits.linkedin_daily_cap,
            },
            self.cache.clone(),
        );

        match client.fetch_profile(&params.profile_url, &limiter).await {
            Ok(profile) => serde_json::to_string_pretty(&profile).unwrap_or_else(|_| "{}".to_string()),
            Err(e) => format!("Error fetching LinkedIn profile: {e}"),
        }
    }

    async fn handle_fetch_profile_x(&self, params: FetchXParams) -> String {
        let cookies_path = self.config.cookies_dir_path().join("x.json");
        let client = match social::x::XClient::from_cookie_file(&cookies_path) {
            Ok(c) => c,
            Err(e) => return format!("Error: {e}"),
        };

        let limiter = PlatformLimiter::new(
            RateLimiterConfig {
                platform: "x".to_string(),
                min_delay: Duration::from_millis(self.config.rate_limits.x_min_delay_ms),
                max_delay: Duration::from_millis(self.config.rate_limits.x_max_delay_ms),
                daily_cap: self.config.rate_limits.x_daily_cap,
            },
            self.cache.clone(),
        );

        match client.fetch_profile(&params.handle, params.include_tweets, params.tweet_count, &limiter).await {
            Ok(profile) => serde_json::to_string_pretty(&profile).unwrap_or_else(|_| "{}".to_string()),
            Err(e) => format!("Error fetching X profile: {e}"),
        }
    }
}

impl ServerHandler for ForageServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "forage".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: None,
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, rmcp::Error>> + Send + '_ {
        async {
            Ok(ListToolsResult {
                next_cursor: None,
                tools: Self::tool_list(),
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::Error>> + Send + '_ {
        async move {
            let args = request.arguments.unwrap_or_default();
            let result_text = self.dispatch_tool(&request.name, args).await;
            Ok(CallToolResult {
                content: vec![Content::text(result_text)],
                is_error: None,
            })
        }
    }
}
