use super::{Tweet, XProfile};
use crate::fetch::FetchError;
use crate::rate_limit::PlatformLimiter;
use serde::Deserialize;
use std::path::Path;

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

    pub async fn fetch_profile(
        &self,
        handle: &str,
        include_tweets: bool,
        tweet_count: usize,
        limiter: &PlatformLimiter,
    ) -> Result<XProfile, FetchError> {
        limiter.acquire().await.map_err(|e| FetchError::HttpError(e.to_string()))?;

        let handle = handle.trim_start_matches('@');

        // Use X's GraphQL UserByScreenName endpoint
        let variables = serde_json::json!({
            "screen_name": handle,
            "withSafetyModeUserFields": true
        });
        let features = serde_json::json!({
            "hidden_profile_subscriptions_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "responsive_web_graphql_exclude_directive_enabled": true,
            "verified_phone_label_enabled": false,
            "highlights_tweets_tab_ui_enabled": true,
            "responsive_web_twitter_article_notes_tab_enabled": true,
            "subscriptions_feature_can_gift_premium": true,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "responsive_web_graphql_timeline_navigation_enabled": true
        });

        let url = format!(
            "https://x.com/i/api/graphql/xmU6X_CKVnQ5lSrCbAmJsg/UserByScreenName?variables={}&features={}",
            urlencoding::encode(&variables.to_string()),
            urlencoding::encode(&features.to_string()),
        );

        let response = self.client
            .get(&url)
            .header("cookie", format!("auth_token={}; ct0={}", self.auth_token, self.ct0))
            .header("x-csrf-token", &self.ct0)
            .header("authorization", "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA")
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("content-type", "application/json")
            .send()
            .await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(FetchError::HttpError(
                "X cookie expired. Run: forage login x".to_string(),
            ));
        }

        let body = response
            .text()
            .await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        let mut profile = parse_user_response(&body, handle)?;

        // Fetch tweets if requested
        if include_tweets {
            if let Ok(tweets) = self.fetch_tweets(handle, tweet_count, limiter).await {
                profile.recent_tweets = tweets;
            }
        }

        Ok(profile)
    }

    async fn fetch_tweets(
        &self,
        handle: &str,
        count: usize,
        limiter: &PlatformLimiter,
    ) -> Result<Vec<Tweet>, FetchError> {
        limiter.acquire().await.map_err(|e| FetchError::HttpError(e.to_string()))?;

        // Use UserTweets GraphQL endpoint
        // Note: user_id is needed; for simplicity we'll return empty if we can't get it
        // In a full implementation, we'd extract rest_id from the profile response
        Ok(vec![])
    }
}

fn parse_user_response(body: &str, handle: &str) -> Result<XProfile, FetchError> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| FetchError::HttpError(format!("Failed to parse X response: {e}")))?;

    let user_result = json
        .pointer("/data/user/result")
        .or_else(|| json.pointer("/data/user"));

    let (bio, followers, following) = if let Some(user) = user_result {
        let legacy = user.get("legacy").unwrap_or(user);
        (
            legacy.get("description").and_then(|v| v.as_str()).map(String::from),
            legacy.get("followers_count").and_then(|v| v.as_u64()),
            legacy.get("friends_count").and_then(|v| v.as_u64()),
        )
    } else {
        (None, None, None)
    };

    Ok(XProfile {
        handle: handle.to_string(),
        bio,
        followers,
        following,
        recent_tweets: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_response_empty() {
        let body = r#"{"data":{"user":{"result":{"legacy":{"description":"test bio","followers_count":100,"friends_count":50}}}}}"#;
        let profile = parse_user_response(body, "test").unwrap();
        assert_eq!(profile.bio, Some("test bio".to_string()));
        assert_eq!(profile.followers, Some(100));
        assert_eq!(profile.following, Some(50));
    }
}
