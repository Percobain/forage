use super::SearchResult;
use crate::fetch::FetchError;

/// Search DuckDuckGo via lite.duckduckgo.com (POST, no API key needed).
/// The lite endpoint is more reliable than html.duckduckgo.com for scraping.
pub async fn search(query: &str, limit: usize, timeout_seconds: u64) -> Result<Vec<SearchResult>, FetchError> {
    let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .cookie_provider(jar)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    // First, visit the lite page to get any required cookies
    let _ = client
        .get("https://lite.duckduckgo.com/lite/")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await;

    let response = client
        .post("https://lite.duckduckgo.com/lite/")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Referer", "https://lite.duckduckgo.com/")
        .header("Origin", "https://lite.duckduckgo.com")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("q={}", urlencoding::encode(query)))
        .send()
        .await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    tracing::debug!("DDG lite response: status={status}, body_len={}, has_result_link={}", body.len(), body.contains("result-link"));

    if body.contains("anomaly") || !body.contains("result-link") {
        tracing::warn!("DDG returned anomaly/no results page (status={status}, len={})", body.len());
        // Return error with diagnostic info so the user knows what happened
        if body.contains("anomaly") {
            return Err(FetchError::HttpError(
                "DuckDuckGo returned an anomaly challenge page. Search may be rate-limited. Try again in a minute.".to_string()
            ));
        }
    }

    let results = parse_ddg_lite_html(&body, limit);
    Ok(results)
}

fn parse_ddg_lite_html(html: &str, limit: usize) -> Vec<SearchResult> {
    let document = scraper::Html::parse_document(html);
    let link_selector = scraper::Selector::parse("a.result-link").unwrap();
    let snippet_selector = scraper::Selector::parse("td.result-snippet").unwrap();

    let links: Vec<_> = document.select(&link_selector).collect();
    let snippets: Vec<_> = document.select(&snippet_selector).collect();

    let mut results = Vec::new();

    for (i, link_el) in links.iter().enumerate() {
        if results.len() >= limit {
            break;
        }

        let title = link_el.text().collect::<String>().trim().to_string();
        let url = link_el.value().attr("href").unwrap_or("").to_string();

        // Skip ads (DDG ads link through duckduckgo.com/y.js)
        if url.contains("duckduckgo.com/y.js") || url.is_empty() || title.is_empty() {
            continue;
        }

        // Skip the "more info" ad disclaimer links
        if title == "more info" {
            continue;
        }

        let snippet = snippets
            .get(i)
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string();

        results.push(SearchResult { title, url, snippet });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ddg_lite_html() {
        let html = r##"<html><body>
            <table>
                <tr>
                    <td>
                        <a rel="nofollow" href="https://example.com/page1" class='result-link'>Example Page 1</a>
                    </td>
                </tr>
                <tr>
                    <td class='result-snippet'>This is the first result snippet.</td>
                </tr>
                <tr>
                    <td>
                        <a rel="nofollow" href="https://example.com/page2" class='result-link'>Example Page 2</a>
                    </td>
                </tr>
                <tr>
                    <td class='result-snippet'>Second result snippet here.</td>
                </tr>
                <tr>
                    <td>
                        <a rel="nofollow" href="https://duckduckgo.com/y.js?ad_domain=spam.com" class='result-link'>Sponsored Ad</a>
                    </td>
                </tr>
            </table>
        </body></html>"##;

        let results = parse_ddg_lite_html(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Page 1");
        assert_eq!(results[0].url, "https://example.com/page1");
        assert_eq!(results[0].snippet, "This is the first result snippet.");
        assert_eq!(results[1].title, "Example Page 2");
    }
}
