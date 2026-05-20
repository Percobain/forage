use crate::fetch::FetchError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct WaybackAvailability {
    archived_snapshots: ArchivedSnapshots,
}

#[derive(Debug, Deserialize)]
struct ArchivedSnapshots {
    closest: Option<Snapshot>,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    url: String,
    timestamp: String,
    status: String,
}

/// Fetch a URL from the Wayback Machine
pub async fn fetch_archive(
    url: &str,
    prefer_date: Option<&str>,
    timeout_seconds: u64,
) -> Result<ArchiveResult, FetchError> {
    let timestamp = prefer_date.unwrap_or("");
    let api_url = format!(
        "https://archive.org/wayback/available?url={}&timestamp={timestamp}",
        urlencoding::encode(url)
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let response = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let body = response
        .text()
        .await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let availability: WaybackAvailability = serde_json::from_str(&body)
        .map_err(|e| FetchError::HttpError(format!("Failed to parse Wayback API response: {e}")))?;

    let snapshot = availability
        .archived_snapshots
        .closest
        .ok_or_else(|| FetchError::HttpError(format!("No archived snapshot found for {url}")))?;

    // Fetch the actual snapshot content
    let snapshot_result = crate::fetch::fetch_with_fallback(&snapshot.url, timeout_seconds, None).await?;

    Ok(ArchiveResult {
        original_url: url.to_string(),
        snapshot_url: snapshot.url,
        capture_date: snapshot.timestamp,
        content: snapshot_result.content,
    })
}

#[derive(Debug, Clone)]
pub struct ArchiveResult {
    pub original_url: String,
    pub snapshot_url: String,
    pub capture_date: String,
    pub content: String,
}
