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

const SOCIAL_TIMEOUT: u64 = 60; // LinkedIn/X via Jina need more time

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
pub struct LinkedInCompanyParams {
    /// LinkedIn company URL, slug, or name. Examples: "razorpay", "https://linkedin.com/company/razorpay"
    pub company: String,
    /// Whether to use cache (default: true)
    #[serde(default = "default_true")]
    pub use_cache: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkedInPersonParams {
    /// LinkedIn profile URL or username. Examples: "johndoe", "https://linkedin.com/in/johndoe"
    pub profile: String,
    /// Whether to use cache (default: true)
    #[serde(default = "default_true")]
    pub use_cache: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkedInSearchParams {
    /// Search query to find companies or people on LinkedIn via web search
    pub query: String,
    /// What to search for: "companies" or "people" (default: "companies")
    #[serde(default = "default_companies")]
    pub search_type: String,
    /// Maximum results (default: 10)
    #[serde(default = "default_ten")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct XProfileParams {
    /// X/Twitter handle (with or without @) or URL. Examples: "elonmusk", "@razorpay", "https://x.com/razorpay"
    pub handle: String,
    /// Whether to use cache (default: true)
    #[serde(default = "default_true")]
    pub use_cache: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct XSearchParams {
    /// Search query to find tweets or users on X via web search
    pub query: String,
    /// Maximum results (default: 10)
    #[serde(default = "default_ten")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchFetchParams {
    /// List of URLs to fetch in parallel
    pub urls: Vec<String>,
}

fn default_true() -> bool { true }
fn default_max_pages() -> usize { 50 }
fn default_max_depth() -> u32 { 3 }
fn default_search_limit() -> usize { 20 }
fn default_company_limit() -> usize { 50 }
fn default_companies() -> String { "companies".to_string() }
fn default_ten() -> usize { 10 }

fn schema_for<T: JsonSchema>() -> serde_json::Map<String, serde_json::Value> {
    let schema = schemars::schema_for!(T);
    let val = serde_json::to_value(schema).unwrap();
    match val {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

impl ForageServer {
    pub fn new(config: Config, cache: Arc<Cache>) -> Self {
        Self { config, cache }
    }

    fn tool_list() -> Vec<Tool> {
        vec![
            Tool {
                name: "fetch_url".into(),
                description: "Fetch any URL and return clean markdown. Handles Cloudflare-protected sites via Jina Reader fallback. Cached 24h.".into(),
                input_schema: schema_for::<FetchUrlParams>().into(),
            },
            Tool {
                name: "crawl_site".into(),
                description: "Crawl a website: discover pages via sitemap/RSS/link crawling, fetch all in parallel, return markdown per page. Respects robots.txt.".into(),
                input_schema: schema_for::<CrawlSiteParams>().into(),
            },
            Tool {
                name: "search_web".into(),
                description: "Search the web via DuckDuckGo. Returns JSON array of {title, url, snippet}. No API key needed.".into(),
                input_schema: schema_for::<SearchWebParams>().into(),
            },
            Tool {
                name: "fetch_archive".into(),
                description: "Fetch a historical snapshot from the Internet Archive Wayback Machine.".into(),
                input_schema: schema_for::<FetchArchiveParams>().into(),
            },
            Tool {
                name: "find_companies".into(),
                description: "Find companies by keywords, size, location via Apollo.io API. Needs apollo.api_key in config. Size: A=1, B=2-10, C=11-50, D=51-200, E=201-500.".into(),
                input_schema: schema_for::<FindCompaniesParams>().into(),
            },
            // === LinkedIn tools (no cookies needed for public pages) ===
            Tool {
                name: "linkedin_company".into(),
                description: "Fetch a LinkedIn company page. Returns company name, industry, size, location, description. Works WITHOUT cookies — fetches public page via Jina Reader. Pass a company slug (e.g. 'razorpay'), name, or full URL.".into(),
                input_schema: schema_for::<LinkedInCompanyParams>().into(),
            },
            Tool {
                name: "linkedin_person".into(),
                description: "Fetch a LinkedIn person profile. Returns name, headline, about, experience, education. Works WITHOUT cookies — fetches public page via Jina Reader. Pass a username (e.g. 'johndoe') or full URL.".into(),
                input_schema: schema_for::<LinkedInPersonParams>().into(),
            },
            Tool {
                name: "linkedin_search".into(),
                description: "Search for LinkedIn companies or people via web search. Returns LinkedIn profile URLs you can then fetch with linkedin_company or linkedin_person. Use search_type='companies' or 'people'.".into(),
                input_schema: schema_for::<LinkedInSearchParams>().into(),
            },
            // === X/Twitter tools (no cookies needed for public pages) ===
            Tool {
                name: "x_profile".into(),
                description: "Fetch an X/Twitter profile. Returns handle, bio, and recent tweets. Works WITHOUT cookies — fetches public page via Jina Reader. Pass handle with or without @.".into(),
                input_schema: schema_for::<XProfileParams>().into(),
            },
            Tool {
                name: "x_search".into(),
                description: "Search for tweets or X users via web search. Returns URLs and snippets. Use to find accounts, mentions, or discussions about a topic on X.".into(),
                input_schema: schema_for::<XSearchParams>().into(),
            },
            // === Batch ===
            Tool {
                name: "batch_fetch".into(),
                description: "Fetch multiple URLs in parallel. Returns JSON object mapping each URL to its content. Use for enriching lists of companies/profiles at scale.".into(),
                input_schema: schema_for::<BatchFetchParams>().into(),
            },
        ]
    }

    async fn dispatch_tool(&self, name: &str, args: serde_json::Map<String, serde_json::Value>) -> String {
        let v = serde_json::Value::Object(args);
        macro_rules! dispatch {
            ($t:ty, $handler:ident) => {
                match serde_json::from_value::<$t>(v) {
                    Ok(p) => self.$handler(p).await,
                    Err(e) => format!("Invalid parameters: {e}"),
                }
            };
        }
        match name {
            "fetch_url" => dispatch!(FetchUrlParams, handle_fetch_url),
            "crawl_site" => dispatch!(CrawlSiteParams, handle_crawl_site),
            "search_web" => dispatch!(SearchWebParams, handle_search_web),
            "fetch_archive" => dispatch!(FetchArchiveParams, handle_fetch_archive),
            "find_companies" => dispatch!(FindCompaniesParams, handle_find_companies),
            "linkedin_company" => dispatch!(LinkedInCompanyParams, handle_linkedin_company),
            "linkedin_person" => dispatch!(LinkedInPersonParams, handle_linkedin_person),
            "linkedin_search" => dispatch!(LinkedInSearchParams, handle_linkedin_search),
            "x_profile" => dispatch!(XProfileParams, handle_x_profile),
            "x_search" => dispatch!(XSearchParams, handle_x_search),
            "batch_fetch" => dispatch!(BatchFetchParams, handle_batch_fetch),
            _ => format!("Unknown tool: {name}"),
        }
    }

    // === Open Web ===

    async fn handle_fetch_url(&self, params: FetchUrlParams) -> String {
        let url = &params.url;
        if params.use_cache {
            let cache_key = Cache::cache_key("fetch", url, "");
            if let Some(entry) = self.cache.get(&cache_key) {
                return entry.content;
            }
        }
        let jina_key = if self.config.fetch.jina_api_key.is_empty() { None } else { Some(self.config.fetch.jina_api_key.as_str()) };
        match fetch::fetch_with_fallback(url, self.config.fetch.default_timeout_seconds, jina_key).await {
            Ok(result) => {
                let cache_key = Cache::cache_key("fetch", url, "");
                self.cache.put(&cache_key, url, &result.method, &result.content, Some(result.status_code as i32), self.config.general.default_cache_ttl_seconds);
                result.content
            }
            Err(e) => format!("Error fetching {url}: {e}"),
        }
    }

    async fn handle_crawl_site(&self, params: CrawlSiteParams) -> String {
        let jina_key = if self.config.fetch.jina_api_key.is_empty() { None } else { Some(self.config.fetch.jina_api_key.as_str()) };
        match crawler::crawl_site(&params.domain, params.max_pages, params.max_depth, self.config.fetch.default_timeout_seconds, self.config.fetch.parallel_concurrency, jina_key).await {
            Ok(result) => {
                let mut output = format!("# Crawl Results for {}\n\nDiscovery method: {}\nPages found: {}\n\n", result.domain, result.discovery_method, result.pages.len());
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
        if let Some(entry) = self.cache.get(&cache_key) { return entry.content; }
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
        match search::wayback::fetch_archive(&params.url, params.prefer_date.as_deref(), self.config.fetch.default_timeout_seconds).await {
            Ok(result) => format!("# Archived Snapshot\n\nOriginal URL: {}\nSnapshot URL: {}\nCapture Date: {}\n\n---\n\n{}", result.original_url, result.snapshot_url, result.capture_date, result.content),
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

    // === LinkedIn ===

    async fn handle_linkedin_company(&self, params: LinkedInCompanyParams) -> String {
        let cache_key = Cache::cache_key("linkedin_company", &params.company, "");
        if params.use_cache {
            if let Some(entry) = self.cache.get(&cache_key) { return entry.content; }
        }

        // Try public page via Jina first (no cookies needed)
        match social::linkedin::fetch_company_raw(&params.company, SOCIAL_TIMEOUT).await {
            Ok(content) => {
                self.cache.put(&cache_key, &params.company, "linkedin_jina", &content, Some(200), 3600);
                content
            }
            Err(e) => format!("Error fetching LinkedIn company: {e}"),
        }
    }

    async fn handle_linkedin_person(&self, params: LinkedInPersonParams) -> String {
        let cache_key = Cache::cache_key("linkedin_person", &params.profile, "");
        if params.use_cache {
            if let Some(entry) = self.cache.get(&cache_key) { return entry.content; }
        }

        // Try cookie-based Voyager API first (richer data)
        let cookies_path = self.config.cookies_dir_path().join("linkedin.json");
        if cookies_path.exists() {
            if let Ok(client) = social::linkedin::LinkedInClient::from_cookie_file(&cookies_path) {
                let limiter = PlatformLimiter::new(
                    RateLimiterConfig {
                        platform: "linkedin".to_string(),
                        min_delay: Duration::from_millis(self.config.rate_limits.linkedin_min_delay_ms),
                        max_delay: Duration::from_millis(self.config.rate_limits.linkedin_max_delay_ms),
                        daily_cap: self.config.rate_limits.linkedin_daily_cap,
                    },
                    self.cache.clone(),
                );
                if let Ok(profile) = client.fetch_profile(&params.profile, &limiter).await {
                    let output = serde_json::to_string_pretty(&profile).unwrap_or_else(|_| "{}".to_string());
                    self.cache.put(&cache_key, &params.profile, "linkedin_voyager", &output, Some(200), 3600);
                    return output;
                }
            }
        }

        // Fallback: public page via Jina (no cookies needed)
        match social::linkedin::fetch_person_public(&params.profile, SOCIAL_TIMEOUT).await {
            Ok(profile) => {
                let output = serde_json::to_string_pretty(&profile).unwrap_or_else(|_| "{}".to_string());
                self.cache.put(&cache_key, &params.profile, "linkedin_jina", &output, Some(200), 3600);
                output
            }
            Err(e) => format!("Error fetching LinkedIn profile: {e}"),
        }
    }

    async fn handle_linkedin_search(&self, params: LinkedInSearchParams) -> String {
        let path_filter = match params.search_type.as_str() {
            "people" | "person" => "/in/",
            _ => "/company/",
        };
        // Search for the query + LinkedIn, then filter results to LinkedIn URLs
        let query = format!("{} LinkedIn", params.query);
        let cache_key = Cache::cache_key("linkedin_search", &format!("{}{}", query, path_filter), "");
        if let Some(entry) = self.cache.get(&cache_key) { return entry.content; }

        // Fetch more results than needed, then filter to LinkedIn URLs
        let fetch_limit = params.limit * 4;
        match search::duckduckgo::search(&query, fetch_limit, self.config.fetch.default_timeout_seconds).await {
            Ok(results) => {
                let filtered: Vec<_> = results.into_iter()
                    .filter(|r| r.url.contains("linkedin.com") && r.url.contains(path_filter))
                    .take(params.limit)
                    .collect();
                let output = serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "[]".to_string());
                self.cache.put(&cache_key, &query, "linkedin_search", &output, Some(200), 21600);
                output
            }
            Err(e) => format!("LinkedIn search error: {e}"),
        }
    }

    // === X/Twitter ===

    async fn handle_x_profile(&self, params: XProfileParams) -> String {
        let cache_key = Cache::cache_key("x_profile", &params.handle, "");
        if params.use_cache {
            if let Some(entry) = self.cache.get(&cache_key) { return entry.content; }
        }

        match social::x::fetch_profile_raw(&params.handle, SOCIAL_TIMEOUT).await {
            Ok(content) => {
                self.cache.put(&cache_key, &params.handle, "x_jina", &content, Some(200), 3600);
                content
            }
            Err(e) => format!("Error fetching X profile: {e}"),
        }
    }

    async fn handle_x_search(&self, params: XSearchParams) -> String {
        let query = format!("site:x.com {}", params.query);
        let cache_key = Cache::cache_key("x_search", &query, "");
        if let Some(entry) = self.cache.get(&cache_key) { return entry.content; }

        match search::duckduckgo::search(&query, params.limit, self.config.fetch.default_timeout_seconds).await {
            Ok(results) => {
                let output = serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string());
                self.cache.put(&cache_key, &query, "x_search", &output, Some(200), 21600);
                output
            }
            Err(e) => format!("X search error: {e}"),
        }
    }

    // === Batch ===

    async fn handle_batch_fetch(&self, params: BatchFetchParams) -> String {
        let jina_key = if self.config.fetch.jina_api_key.is_empty() { None } else { Some(self.config.fetch.jina_api_key.clone()) };
        let timeout = self.config.fetch.default_timeout_seconds;
        let cache = self.cache.clone();
        let ttl = self.config.general.default_cache_ttl_seconds;

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.fetch.parallel_concurrency));
        let mut handles = Vec::new();

        for url in params.urls {
            let sem = semaphore.clone();
            let jk = jina_key.clone();
            let cache = cache.clone();
            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                // Check cache
                let cache_key = Cache::cache_key("fetch", &url, "");
                if let Some(entry) = cache.get(&cache_key) {
                    return (url, entry.content);
                }

                let jk_ref = jk.as_deref();
                match fetch::fetch_with_fallback(&url, timeout, jk_ref).await {
                    Ok(result) => {
                        cache.put(&cache_key, &url, &result.method, &result.content, Some(result.status_code as i32), ttl);
                        (url, result.content)
                    }
                    Err(e) => (url, format!("Error: {e}")),
                }
            });
            handles.push(handle);
        }

        let mut results = serde_json::Map::new();
        for handle in handles {
            if let Ok((url, content)) = handle.await {
                // Truncate long content to avoid overwhelming Claude
                let content = if content.len() > 5000 {
                    format!("{}...\n\n[Truncated, {} total chars. Use fetch_url for full content.]", &content[..5000], content.len())
                } else {
                    content
                };
                results.insert(url, serde_json::Value::String(content));
            }
        }

        serde_json::to_string_pretty(&results).unwrap_or_else(|_| "{}".to_string())
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
                tools: Some(ToolsCapability { list_changed: None }),
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
        async { Ok(ListToolsResult { next_cursor: None, tools: Self::tool_list() }) }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::Error>> + Send + '_ {
        async move {
            let args = request.arguments.unwrap_or_default();
            let result_text = self.dispatch_tool(&request.name, args).await;
            Ok(CallToolResult { content: vec![Content::text(result_text)], is_error: None })
        }
    }
}
