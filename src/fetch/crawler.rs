use super::sitemap;
use super::FetchError;
use std::collections::HashSet;
use url::Url;

#[derive(Debug, Clone)]
pub struct CrawlResult {
    pub domain: String,
    pub pages: Vec<PageResult>,
    pub discovery_method: String,
}

#[derive(Debug, Clone)]
pub struct PageResult {
    pub url: String,
    pub title: Option<String>,
    pub content: String,
    pub status_code: u16,
}

/// Crawl a site: discover URLs via sitemap/RSS/BFS, then fetch in parallel.
pub async fn crawl_site(
    domain: &str,
    max_pages: usize,
    max_depth: u32,
    timeout_seconds: u64,
    concurrency: usize,
    jina_api_key: Option<&str>,
) -> Result<CrawlResult, FetchError> {
    let base_url = if domain.starts_with("http://") || domain.starts_with("https://") {
        Url::parse(domain).map_err(|e| FetchError::UrlError(e.to_string()))?
    } else {
        Url::parse(&format!("https://{domain}"))
            .map_err(|e| FetchError::UrlError(e.to_string()))?
    };

    let base_str = base_url.as_str().trim_end_matches('/');

    // Step 1: Fetch robots.txt
    let robots_url = format!("{base_str}/robots.txt");
    let robots_info = match super::direct::fetch(&robots_url, timeout_seconds).await {
        Ok(r) if r.status_code == 200 => sitemap::parse_robots_txt(&r.content, "*"),
        _ => sitemap::RobotsInfo {
            sitemaps: vec![],
            disallowed: vec![],
            crawl_delay: None,
        },
    };

    // Step 2: Try sitemap discovery
    let (urls, method) = discover_urls(&base_url, &robots_info, timeout_seconds, max_pages).await;

    if urls.is_empty() {
        // Final fallback: just fetch the homepage
        let homepage = super::fetch_with_fallback(base_str, timeout_seconds, jina_api_key).await?;
        let title = extract_title_from_markdown(&homepage.content);
        return Ok(CrawlResult {
            domain: domain.to_string(),
            pages: vec![PageResult {
                url: homepage.url,
                title,
                content: homepage.content,
                status_code: homepage.status_code,
            }],
            discovery_method: "homepage_only".to_string(),
        });
    }

    // Step 3: Filter URLs against robots.txt disallow rules
    let filtered_urls: Vec<String> = urls
        .into_iter()
        .filter(|u| {
            if let Ok(parsed) = Url::parse(u) {
                sitemap::is_allowed(parsed.path(), &robots_info.disallowed)
            } else {
                false
            }
        })
        .take(max_pages)
        .collect();

    // Step 4: Parallel fetch
    let pages = parallel_fetch(&filtered_urls, timeout_seconds, concurrency, jina_api_key).await;

    Ok(CrawlResult {
        domain: domain.to_string(),
        pages,
        discovery_method: method,
    })
}

async fn discover_urls(
    base_url: &Url,
    robots_info: &sitemap::RobotsInfo,
    timeout_seconds: u64,
    max_pages: usize,
) -> (Vec<String>, String) {
    let base_str = base_url.as_str().trim_end_matches('/');

    // Try sitemaps from robots.txt first
    for sitemap_url in &robots_info.sitemaps {
        if let Some(urls) = try_fetch_sitemap(sitemap_url, timeout_seconds, max_pages).await {
            if !urls.is_empty() {
                return (urls, "robots_sitemap".to_string());
            }
        }
    }

    // Try standard sitemap paths
    for path in sitemap::sitemap_paths() {
        let url = format!("{base_str}{path}");
        if let Some(urls) = try_fetch_sitemap(&url, timeout_seconds, max_pages).await {
            if !urls.is_empty() {
                return (urls, format!("sitemap:{path}"));
            }
        }
    }

    // Try RSS feeds
    for path in sitemap::rss_paths() {
        let url = format!("{base_str}{path}");
        if let Ok(result) = super::direct::fetch(&url, timeout_seconds).await {
            if result.status_code == 200 && (result.content.contains("<item") || result.content.contains("<entry")) {
                let urls = sitemap::parse_sitemap_xml(&result.content);
                if !urls.is_empty() {
                    return (urls.into_iter().take(max_pages).collect(), format!("rss:{path}"));
                }
            }
        }
    }

    // Try extracting RSS links from homepage HTML
    if let Ok(result) = super::direct::fetch(base_str, timeout_seconds).await {
        if result.status_code == 200 {
            let rss_links = sitemap::extract_rss_links(&result.content, base_url);
            for rss_url in &rss_links {
                if let Ok(rss_result) = super::direct::fetch(rss_url, timeout_seconds).await {
                    if rss_result.status_code == 200 {
                        let urls = sitemap::parse_sitemap_xml(&rss_result.content);
                        if !urls.is_empty() {
                            return (urls.into_iter().take(max_pages).collect(), "rss:html_link".to_string());
                        }
                    }
                }
            }

            // BFS link crawl as last resort
            let links = bfs_crawl(base_url, &result.content, timeout_seconds, max_pages, 3).await;
            if !links.is_empty() {
                return (links, "bfs_crawl".to_string());
            }
        }
    }

    (vec![], "none".to_string())
}

async fn try_fetch_sitemap(url: &str, timeout_seconds: u64, max_pages: usize) -> Option<Vec<String>> {
    let result = super::direct::fetch(url, timeout_seconds).await.ok()?;
    if result.status_code != 200 {
        return None;
    }

    if sitemap::is_sitemap_index(&result.content) {
        // Recursively fetch child sitemaps
        let child_urls = sitemap::parse_sitemap_xml(&result.content);
        let mut all_urls = Vec::new();
        for child_url in child_urls {
            if all_urls.len() >= max_pages {
                break;
            }
            if let Ok(child_result) = super::direct::fetch(&child_url, timeout_seconds).await {
                if child_result.status_code == 200 {
                    let urls = sitemap::parse_sitemap_xml(&child_result.content);
                    all_urls.extend(urls);
                }
            }
        }
        if all_urls.is_empty() {
            None
        } else {
            Some(all_urls.into_iter().take(max_pages).collect())
        }
    } else {
        let urls = sitemap::parse_sitemap_xml(&result.content);
        if urls.is_empty() {
            None
        } else {
            Some(urls.into_iter().take(max_pages).collect())
        }
    }
}

async fn bfs_crawl(
    base_url: &Url,
    homepage_html: &str,
    timeout_seconds: u64,
    max_pages: usize,
    max_depth: u32,
) -> Vec<String> {
    let mut visited = HashSet::new();
    let mut queue: Vec<(String, u32)> = Vec::new();
    let mut result_urls = Vec::new();

    // Seed with homepage links
    let homepage_str = base_url.as_str().trim_end_matches('/').to_string();
    visited.insert(homepage_str.clone());
    result_urls.push(homepage_str);

    let links = sitemap::extract_links(homepage_html, base_url);
    for link in links {
        if visited.insert(link.clone()) {
            queue.push((link, 1));
        }
    }

    while let Some((url, depth)) = queue.pop() {
        if result_urls.len() >= max_pages {
            break;
        }

        result_urls.push(url.clone());

        if depth < max_depth {
            if let Ok(result) = super::direct::fetch(&url, timeout_seconds).await {
                if result.status_code == 200 {
                    if let Ok(page_url) = Url::parse(&url) {
                        let new_links = sitemap::extract_links(&result.content, &page_url);
                        for link in new_links {
                            if visited.insert(link.clone()) && result_urls.len() + queue.len() < max_pages {
                                queue.push((link, depth + 1));
                            }
                        }
                    }
                }
            }
        }
    }

    result_urls
}

async fn parallel_fetch(
    urls: &[String],
    timeout_seconds: u64,
    concurrency: usize,
    jina_api_key: Option<&str>,
) -> Vec<PageResult> {
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let jina_key = jina_api_key.map(|s| s.to_string());

    let mut handles = Vec::new();
    for url in urls {
        let url = url.clone();
        let sem = semaphore.clone();
        let jk = jina_key.clone();

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let jk_ref = jk.as_deref();
            match super::fetch_with_fallback(&url, timeout_seconds, jk_ref).await {
                Ok(result) => {
                    let title = extract_title_from_markdown(&result.content);
                    Some(PageResult {
                        url: result.url,
                        title,
                        content: result.content,
                        status_code: result.status_code,
                    })
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch {url}: {e}");
                    None
                }
            }
        });
        handles.push(handle);
    }

    let mut pages = Vec::new();
    for handle in handles {
        if let Ok(Some(page)) = handle.await {
            pages.push(page);
        }
    }

    pages
}

fn extract_title_from_markdown(md: &str) -> Option<String> {
    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            return Some(title.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_title() {
        assert_eq!(
            extract_title_from_markdown("# My Title\n\nSome content"),
            Some("My Title".to_string())
        );
        assert_eq!(
            extract_title_from_markdown("No title here"),
            None
        );
    }
}
