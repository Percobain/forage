use super::{FetchError, FetchResult};
use std::time::Duration;

const JINA_READER_BASE: &str = "https://r.jina.ai/";

/// Fetch a URL via Jina Reader API, which returns clean markdown.
/// Jina handles JS rendering and Cloudflare challenges.
pub async fn fetch(
    url: &str,
    api_key: Option<&str>,
    timeout_seconds: u64,
) -> Result<FetchResult, FetchError> {
    let jina_url = format!("{JINA_READER_BASE}{url}");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let mut request = client.get(&jina_url);

    if let Some(key) = api_key {
        if !key.is_empty() {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
    }

    // Ask Jina to return markdown
    request = request.header("Accept", "text/markdown");

    let response = request.send().await.map_err(|e| {
        if e.is_timeout() {
            FetchError::Timeout(timeout_seconds)
        } else {
            FetchError::HttpError(e.to_string())
        }
    })?;

    let status = response.status().as_u16();

    if status == 402 {
        return Err(FetchError::HttpError(
            "Jina Reader rate limit exceeded. Add a jina_api_key to config.toml for higher limits."
                .to_string(),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    if status != 200 {
        return Err(FetchError::HttpError(format!(
            "Jina returned status {status}"
        )));
    }

    Ok(FetchResult {
        url: url.to_string(),
        status_code: 200,
        content: body,
        method: "jina".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_jina_fetch_example() {
        let result = fetch("https://example.com", None, 15).await;
        match result {
            Ok(r) => {
                assert_eq!(r.method, "jina");
                assert!(r.content.contains("Example Domain") || r.content.contains("example"));
            }
            Err(e) => {
                // Jina may rate-limit or network may be unavailable
                eprintln!("Skipping Jina network test: {e}");
            }
        }
    }
}
