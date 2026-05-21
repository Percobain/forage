use super::{Tweet, XProfile};
use crate::fetch::FetchError;
use crate::rate_limit::PlatformLimiter;
use serde::Deserialize;
use std::path::Path;

// =========================================
// Cookie-free X via Jina Reader
// =========================================

/// Fetch an X/Twitter profile via Jina Reader (no cookies needed).
/// Returns profile info parsed from the public page.
pub async fn fetch_profile_public(
    handle: &str,
    timeout_seconds: u64,
) -> Result<XProfile, FetchError> {
    let handle = handle.trim_start_matches('@').trim_start_matches("https://x.com/").trim_start_matches("https://twitter.com/").trim_end_matches('/');
    let url = format!("https://x.com/{handle}");
    let jina_url = format!("https://r.jina.ai/{url}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let response = client
        .get(&jina_url)
        .header("Accept", "text/markdown")
        .header("User-Agent", "Mozilla/5.0")
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
    if status != 200 {
        return Err(FetchError::HttpError(format!(
            "Jina returned status {status} for X profile"
        )));
    }

    let body = response.text().await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    Ok(parse_x_markdown(&body, handle))
}

/// Fetch an X profile and return raw markdown for Claude to analyze.
pub async fn fetch_profile_raw(
    handle: &str,
    timeout_seconds: u64,
) -> Result<String, FetchError> {
    let handle = handle.trim_start_matches('@').trim_start_matches("https://x.com/").trim_start_matches("https://twitter.com/").trim_end_matches('/');
    let url = format!("https://x.com/{handle}");
    let jina_url = format!("https://r.jina.ai/{url}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    let response = client
        .get(&jina_url)
        .header("Accept", "text/markdown")
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                FetchError::Timeout(timeout_seconds)
            } else {
                FetchError::HttpError(e.to_string())
            }
        })?;

    let body = response.text().await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    Ok(body)
}

fn parse_x_markdown(md: &str, handle: &str) -> XProfile {
    let lines: Vec<&str> = md.lines().collect();

    let mut bio = None;
    let mut display_name = handle.to_string();

    // Look for the display name and bio
    for line in &lines {
        let l = line.trim();

        // Display name is usually in the title
        if l.starts_with("Title: ") || l.starts_with("# ") {
            let clean = l.trim_start_matches("Title: ").trim_start_matches("# ");
            if let Some(name) = clean.split(" (@").next() {
                if let Some(name) = name.split(" / ").next() {
                    if !name.is_empty() && name != "X" {
                        display_name = name.trim().to_string();
                    }
                }
            }
        }

        // Bio is usually a short line after the handle
        if bio.is_none() && !l.is_empty() && l.len() > 10 && l.len() < 300
            && !l.starts_with("[") && !l.starts_with("*") && !l.starts_with("#")
            && !l.starts_with("Title:") && !l.starts_with("URL ")
            && !l.starts_with("Markdown") && !l.starts_with("![")
            && !l.contains("Sign in") && !l.contains("posts")
            && !l.contains("x.com") && !l.contains("twitter.com")
            && !l.contains("Square profile") {
            bio = Some(l.to_string());
        }
    }

    // Try to find recent tweets
    let mut tweets = Vec::new();
    let mut in_tweet = false;
    let mut tweet_text = String::new();

    for line in &lines {
        let l = line.trim();
        // Tweets often start after profile picture references
        if l.contains("Square profile picture") {
            if !tweet_text.is_empty() {
                tweets.push(Tweet {
                    text: tweet_text.trim().to_string(),
                    created_at: None,
                    likes: None,
                    retweets: None,
                });
                tweet_text.clear();
            }
            in_tweet = true;
            continue;
        }
        if in_tweet && !l.is_empty() && !l.starts_with("![") && !l.starts_with("[![")
            && !l.contains("x.com") && l.len() > 10 {
            if !tweet_text.is_empty() {
                tweet_text.push(' ');
            }
            tweet_text.push_str(l);
        }
    }
    if !tweet_text.is_empty() && tweets.len() < 20 {
        tweets.push(Tweet {
            text: tweet_text.trim().to_string(),
            created_at: None,
            likes: None,
            retweets: None,
        });
    }

    XProfile {
        handle: handle.to_string(),
        bio,
        followers: None,
        following: None,
        recent_tweets: tweets,
    }
}

// =========================================
// Cookie-based X via GraphQL API
// =========================================

#[derive(Debug, Deserialize)]
struct CookieFile {
    cookies: Vec<CookieEntry>,
}

#[derive(Debug, Deserialize)]
struct CookieEntry {
    name: String,
    value: String,
}

pub struct XClient {
    auth_token: String,
    ct0: String,
    client: reqwest::Client,
}

impl XClient {
    pub fn from_cookie_file(path: &Path) -> Result<Self, FetchError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| FetchError::HttpError(format!("Failed to read X cookie file: {e}")))?;

        let cookie_file: CookieFile = serde_json::from_str(&content)
            .map_err(|e| FetchError::HttpError(format!("Failed to parse X cookie file: {e}")))?;

        let auth_token = cookie_file
            .cookies
            .iter()
            .find(|c| c.name == "auth_token")
            .ok_or_else(|| FetchError::HttpError(
                "X cookie expired or missing 'auth_token'. Run: forage login x".to_string(),
            ))?
            .value
            .clone();

        let ct0 = cookie_file
            .cookies
            .iter()
            .find(|c| c.name == "ct0")
            .map(|c| c.value.clone())
            .unwrap_or_default();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        Ok(Self { auth_token, ct0, client })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_x_markdown() {
        let md = "Title: Razorpay (@Razorpay) / X\n\nURL Source: https://x.com/razorpay\n\nBacking India's Boldest Founders. Join the movement\n\n## Razorpay's posts\n";
        let profile = parse_x_markdown(md, "razorpay");
        assert_eq!(profile.handle, "razorpay");
        assert_eq!(profile.bio, Some("Backing India's Boldest Founders. Join the movement".to_string()));
    }
}
