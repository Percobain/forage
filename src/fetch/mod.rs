pub mod crawler;
pub mod direct;
pub mod jina;
pub mod sitemap;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP request failed: {0}")]
    HttpError(String),
    #[error("Request timed out after {0}s")]
    Timeout(u64),
    #[error("Blocked by site protection (status {0})")]
    Blocked(u16),
    #[error("URL parse error: {0}")]
    UrlError(String),
    #[error("Content extraction failed: {0}")]
    ExtractionError(String),
}

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub url: String,
    pub status_code: u16,
    pub content: String,
    pub method: String,
}

/// Fetch a URL with the tiered fallback chain: direct -> Jina -> error
pub async fn fetch_with_fallback(
    url: &str,
    timeout_seconds: u64,
    jina_api_key: Option<&str>,
) -> Result<FetchResult, FetchError> {
    // Tier 1: direct fetch with TLS impersonation
    match direct::fetch(url, timeout_seconds).await {
        Ok(result) if result.status_code == 200 => return Ok(result),
        Ok(result) if result.status_code == 403 || result.status_code == 503 => {
            tracing::info!(
                "Direct fetch got {} for {url}, falling back to Jina",
                result.status_code
            );
        }
        Ok(result) => {
            // Non-200 but not a block — still return it
            return Ok(result);
        }
        Err(e) => {
            tracing::info!("Direct fetch failed for {url}: {e}, falling back to Jina");
        }
    }

    // Tier 2: Jina Reader
    match jina::fetch(url, jina_api_key, timeout_seconds).await {
        Ok(result) => return Ok(result),
        Err(e) => {
            tracing::info!("Jina fetch failed for {url}: {e}");
        }
    }

    Err(FetchError::Blocked(403))
}
