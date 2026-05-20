use super::LinkedInProfile;
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
    domain: String,
}

pub struct LinkedInClient {
    li_at: String,
    jsessionid: String,
    client: reqwest::Client,
}

impl LinkedInClient {
    pub fn from_cookie_file(path: &Path) -> Result<Self, FetchError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| FetchError::HttpError(format!("Failed to read LinkedIn cookie file: {e}")))?;

        let cookie_file: CookieFile = serde_json::from_str(&content)
            .map_err(|e| FetchError::HttpError(format!("Failed to parse LinkedIn cookie file: {e}")))?;

        let li_at = cookie_file
            .cookies
            .iter()
            .find(|c| c.name == "li_at")
            .ok_or_else(|| FetchError::HttpError(
                "LinkedIn cookie expired or missing 'li_at'. Run: forage login linkedin".to_string(),
            ))?
            .value
            .clone();

        let jsessionid = cookie_file
            .cookies
            .iter()
            .find(|c| c.name == "JSESSIONID")
            .map(|c| c.value.clone())
            .unwrap_or_default();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        Ok(Self { li_at, jsessionid, client })
    }

    pub async fn fetch_profile(
        &self,
        profile_url: &str,
        limiter: &PlatformLimiter,
    ) -> Result<LinkedInProfile, FetchError> {
        limiter.acquire().await.map_err(|e| FetchError::HttpError(e.to_string()))?;

        // Extract username from URL
        let username = extract_username(profile_url);

        let api_url = format!(
            "https://www.linkedin.com/voyager/api/identity/dash/profiles?q=memberIdentity&memberIdentity={username}&decorationId=com.linkedin.voyager.dash.deco.identity.profile.WebTopCardCore-20"
        );

        let response = self.client
            .get(&api_url)
            .header("cookie", format!("li_at={}; JSESSIONID={}", self.li_at, self.jsessionid))
            .header("csrf-token", self.jsessionid.trim_matches('"'))
            .header("accept", "application/vnd.linkedin.normalized+json+2.1")
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("x-li-lang", "en_US")
            .header("x-restli-protocol-version", "2.0.0")
            .send()
            .await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(FetchError::HttpError(
                "LinkedIn cookie expired. Run: forage login linkedin".to_string(),
            ));
        }

        let body = response
            .text()
            .await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        if status != 200 {
            return Err(FetchError::HttpError(format!(
                "LinkedIn API returned status {status}"
            )));
        }

        parse_profile_response(&body, &username)
    }
}

fn extract_username(url: &str) -> String {
    let url = url.trim_end_matches('/');
    if let Some(pos) = url.rfind("/in/") {
        url[pos + 4..].to_string()
    } else if url.starts_with("http") {
        url.rsplit('/').next().unwrap_or(url).to_string()
    } else {
        url.to_string()
    }
}

fn parse_profile_response(body: &str, username: &str) -> Result<LinkedInProfile, FetchError> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| FetchError::HttpError(format!("Failed to parse LinkedIn response: {e}")))?;

    // Extract from LinkedIn's normalized JSON format
    let elements = json.get("included")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .clone();

    let mut name = username.to_string();
    let mut headline = None;
    let mut about = None;
    let mut experience = Vec::new();
    let mut education = Vec::new();

    for element in &elements {
        if let Some(first_name) = element.get("firstName").and_then(|v| v.as_str()) {
            if let Some(last_name) = element.get("lastName").and_then(|v| v.as_str()) {
                name = format!("{first_name} {last_name}");
            }
        }
        if let Some(h) = element.get("headline").and_then(|v| v.as_str()) {
            headline = Some(h.to_string());
        }
        if let Some(s) = element.get("summary").and_then(|v| v.as_str()) {
            about = Some(s.to_string());
        }
        if let Some(company) = element.get("companyName").and_then(|v| v.as_str()) {
            if let Some(title) = element.get("title").and_then(|v| v.as_str()) {
                experience.push(format!("{title} at {company}"));
            }
        }
        if let Some(school) = element.get("schoolName").and_then(|v| v.as_str()) {
            education.push(school.to_string());
        }
    }

    Ok(LinkedInProfile {
        name,
        headline,
        about,
        experience,
        education,
        recent_posts: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_username() {
        assert_eq!(extract_username("https://www.linkedin.com/in/johndoe"), "johndoe");
        assert_eq!(extract_username("https://linkedin.com/in/johndoe/"), "johndoe");
        assert_eq!(extract_username("johndoe"), "johndoe");
    }
}
