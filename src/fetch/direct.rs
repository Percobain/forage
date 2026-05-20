use super::{FetchError, FetchResult};
use std::time::Duration;

/// Fetch a URL directly using reqwest.
/// TODO: swap to rquest for TLS impersonation when crate stabilizes.
pub async fn fetch(url: &str, timeout_seconds: u64) -> Result<FetchResult, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                FetchError::Timeout(timeout_seconds)
            } else {
                FetchError::HttpError(e.to_string())
            }
        })?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    // Convert HTML to markdown if the response looks like HTML
    let content = if body.trim_start().starts_with("<!") || body.trim_start().starts_with("<html") {
        html_to_markdown(&body)
    } else {
        body
    };

    Ok(FetchResult {
        url: url.to_string(),
        status_code: status,
        content,
        method: "direct".to_string(),
    })
}

fn html_to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_else(|_| {
        // Fallback: strip tags crudely if htmd fails
        let doc = scraper::Html::parse_document(html);
        doc.root_element()
            .text()
            .collect::<Vec<_>>()
            .join(" ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_to_markdown_basic() {
        let html = "<html><body><h1>Title</h1><p>Hello world</p></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("Title"));
        assert!(md.contains("Hello world"));
    }

    #[test]
    fn test_html_to_markdown_with_links() {
        let html = r#"<html><body><a href="https://example.com">Click here</a></body></html>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("Click here"));
        assert!(md.contains("https://example.com"));
    }

    #[tokio::test]
    async fn test_fetch_example_dot_com() {
        let result = fetch("https://example.com", 10).await;
        match result {
            Ok(r) => {
                assert_eq!(r.status_code, 200);
                assert_eq!(r.method, "direct");
                assert!(r.content.contains("Example Domain"));
            }
            Err(e) => {
                // Network may be unavailable in CI
                eprintln!("Skipping network test: {e}");
            }
        }
    }
}
