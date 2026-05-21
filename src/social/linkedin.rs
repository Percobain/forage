use super::{CompanyInfo, LinkedInProfile};
use crate::fetch::FetchError;
use crate::rate_limit::PlatformLimiter;
use serde::Deserialize;
use std::path::Path;

// =========================================
// Cookie-free LinkedIn via Jina Reader
// =========================================

/// Fetch a LinkedIn company page via Jina Reader (no cookies needed).
/// Returns structured company info parsed from the public page.
pub async fn fetch_company_public(
    company_url: &str,
    timeout_seconds: u64,
) -> Result<CompanyInfo, FetchError> {
    let url = normalize_company_url(company_url);
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
            "Jina returned status {status} for LinkedIn company page"
        )));
    }

    let body = response.text().await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    Ok(parse_company_markdown(&body, &url))
}

/// Fetch a LinkedIn person profile via Jina Reader (no cookies needed).
/// Returns structured profile info from the public page.
pub async fn fetch_person_public(
    profile_url: &str,
    timeout_seconds: u64,
) -> Result<LinkedInProfile, FetchError> {
    let url = normalize_profile_url(profile_url);
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
            "Jina returned status {status} for LinkedIn profile"
        )));
    }

    let body = response.text().await
        .map_err(|e| FetchError::HttpError(e.to_string()))?;

    Ok(parse_profile_markdown(&body, &url))
}

/// Fetch a LinkedIn company page and return the raw markdown for Claude to analyze.
pub async fn fetch_company_raw(
    company_url: &str,
    timeout_seconds: u64,
) -> Result<String, FetchError> {
    let url = normalize_company_url(company_url);
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

// =========================================
// Cookie-based LinkedIn via Voyager API
// =========================================

#[derive(Debug, Deserialize)]
struct CookieEntry {
    name: String,
    value: String,
    #[serde(default)]
    #[allow(dead_code)]
    domain: String,
}

#[derive(Debug, Deserialize)]
struct WrappedCookieFile {
    cookies: Vec<CookieEntry>,
}

/// Parse cookie file in either format:
/// - Raw Cookie-Editor export: `[{name, value, domain, ...}, ...]`
/// - Wrapped format: `{platform: "...", cookies: [{name, value}, ...]}`
fn parse_cookie_file(content: &str) -> Result<Vec<CookieEntry>, FetchError> {
    // Try raw array first (Cookie-Editor export)
    if let Ok(cookies) = serde_json::from_str::<Vec<CookieEntry>>(content) {
        return Ok(cookies);
    }
    // Try wrapped format
    if let Ok(wrapped) = serde_json::from_str::<WrappedCookieFile>(content) {
        return Ok(wrapped.cookies);
    }
    Err(FetchError::HttpError(
        "Failed to parse cookie file. Expected either a JSON array from Cookie-Editor export or {\"cookies\": [...]} format.".to_string()
    ))
}

pub struct LinkedInClient {
    cookie_header: String,
    csrf_token: String,
    client: reqwest::Client,
}

impl LinkedInClient {
    pub fn from_cookie_file(path: &Path) -> Result<Self, FetchError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| FetchError::HttpError(format!("Failed to read LinkedIn cookie file: {e}")))?;

        let cookies = parse_cookie_file(&content)?;

        // Verify li_at exists
        if !cookies.iter().any(|c| c.name == "li_at") {
            return Err(FetchError::HttpError(
                "LinkedIn cookie missing 'li_at'. Re-export cookies from browser.".to_string(),
            ));
        }

        let csrf_token = cookies
            .iter()
            .find(|c| c.name == "JSESSIONID")
            .map(|c| c.value.trim_matches('"').to_string())
            .unwrap_or_default();

        // Send ALL cookies — LinkedIn validates more than just li_at
        let cookie_header = cookies
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        Ok(Self { cookie_header, csrf_token, client })
    }

    /// Fetch a LinkedIn profile by loading the web page with cookies
    /// (like a real browser) and extracting data from embedded JSON.
    pub async fn fetch_profile(
        &self,
        profile_url: &str,
        limiter: &PlatformLimiter,
    ) -> Result<LinkedInProfile, FetchError> {
        limiter.acquire().await.map_err(|e| FetchError::HttpError(e.to_string()))?;

        let username = extract_username(profile_url);
        let page_url = format!("https://www.linkedin.com/in/{username}/");

        let response = self.client
            .get(&page_url)
            .header("cookie", &self.cookie_header)
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("accept-language", "en-US,en;q=0.9")
            .header("sec-fetch-dest", "document")
            .header("sec-fetch-mode", "navigate")
            .header("sec-fetch-site", "none")
            .header("sec-fetch-user", "?1")
            .header("upgrade-insecure-requests", "1")
            .send()
            .await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        let status = response.status().as_u16();
        if status == 302 || status == 301 {
            return Err(FetchError::HttpError(
                "LinkedIn session expired (302 redirect). Re-export cookies from browser.".to_string(),
            ));
        }
        if status == 999 {
            return Err(FetchError::HttpError(
                "LinkedIn blocked request (999). Account may need warming up.".to_string(),
            ));
        }
        if status != 200 {
            return Err(FetchError::HttpError(format!(
                "LinkedIn returned status {status}"
            )));
        }

        let body = response.text().await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        if body.len() < 5000 || body.contains("\"authwall\"") {
            return Err(FetchError::HttpError(
                "LinkedIn returned auth wall. Re-export cookies from browser.".to_string(),
            ));
        }

        Ok(parse_html_profile(&body, &username))
    }
}

// =========================================
// Parsing helpers
// =========================================

fn normalize_company_url(input: &str) -> String {
    let input = input.trim().trim_end_matches('/');
    if input.starts_with("http") {
        input.to_string()
    } else if input.contains("linkedin.com") {
        format!("https://{input}")
    } else {
        // Assume it's a slug like "razorpay"
        format!("https://www.linkedin.com/company/{input}")
    }
}

fn normalize_profile_url(input: &str) -> String {
    let input = input.trim().trim_end_matches('/');
    if input.starts_with("http") {
        input.to_string()
    } else if input.contains("linkedin.com") {
        format!("https://{input}")
    } else {
        format!("https://www.linkedin.com/in/{input}")
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

fn parse_company_markdown(md: &str, url: &str) -> CompanyInfo {
    let lines: Vec<&str> = md.lines().collect();

    // Extract name from title line (usually "# Company Name | LinkedIn")
    let name = lines.iter()
        .find(|l| l.starts_with("# ") || l.starts_with("Title: "))
        .map(|l| {
            l.trim_start_matches("# ")
                .trim_start_matches("Title: ")
                .split(" | ")
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();

    // Look for industry, size, description patterns in the markdown
    let mut industry = None;
    let mut size = None;
    let mut description = None;
    let mut hq = None;

    for (i, line) in lines.iter().enumerate() {
        let l = line.trim();

        // LinkedIn shows "X followers" near the top
        if l.contains("followers") && size.is_none() {
            // Look for employee count nearby
            for j in i.saturating_sub(3)..=(i + 5).min(lines.len() - 1) {
                let nearby = lines[j].trim();
                if nearby.contains("employees") || nearby.contains("people") {
                    size = Some(nearby.to_string());
                    break;
                }
            }
        }

        // Industry is often a standalone line near the top
        if industry.is_none() && (l.contains("Information Technology") || l.contains("Financial Services")
            || l.contains("Software") || l.contains("Banking") || l.contains("Internet")
            || l.contains("Computer") || l.contains("Technology") || l.contains("Legal")
            || l.contains("Fintech") || l.contains("Services")) && l.len() < 80 && !l.contains("http") {
            industry = Some(l.to_string());
        }

        // Description / About section
        if (l == "About" || l == "## About" || l.starts_with("About us")) && description.is_none() {
            // Next non-empty line(s) are the description
            let mut desc_lines = Vec::new();
            for j in (i + 1)..lines.len().min(i + 10) {
                let dl = lines[j].trim();
                if dl.is_empty() || dl.starts_with("#") || dl.starts_with("*") || dl.starts_with("[") {
                    break;
                }
                desc_lines.push(dl);
            }
            if !desc_lines.is_empty() {
                description = Some(desc_lines.join(" "));
            }
        }

        // Location
        if hq.is_none() && (l.contains(", India") || l.contains(", Maharashtra")
            || l.contains(", Karnataka") || l.contains(", Haryana")
            || l.contains(", Telangana") || l.contains(", US")
            || l.contains("Mumbai") || l.contains("Bangalore") || l.contains("Bengaluru")
            || l.contains("Gurgaon") || l.contains("Gurugram") || l.contains("Hyderabad")
            || l.contains("Pune") || l.contains("Delhi"))
            && l.len() < 100 && !l.contains("http") {
            hq = Some(l.to_string());
        }
    }

    CompanyInfo {
        name,
        domain: None,
        industry,
        size,
        hq,
        description,
        linkedin_url: Some(url.to_string()),
    }
}

fn parse_profile_markdown(md: &str, url: &str) -> LinkedInProfile {
    let lines: Vec<&str> = md.lines().collect();

    let name = lines.iter()
        .find(|l| l.starts_with("# ") || l.starts_with("Title: "))
        .map(|l| {
            l.trim_start_matches("# ")
                .trim_start_matches("Title: ")
                .split(" - ")
                .next()
                .unwrap_or("")
                .split(" | ")
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();

    let mut headline = None;
    let mut about = None;
    let mut experience = Vec::new();
    let mut education = Vec::new();

    let mut current_section = "";
    for line in &lines {
        let l = line.trim();

        if l == "Experience" || l == "## Experience" {
            current_section = "experience";
            continue;
        }
        if l == "Education" || l == "## Education" {
            current_section = "education";
            continue;
        }
        if l == "About" || l == "## About" {
            current_section = "about";
            continue;
        }
        if l.starts_with("## ") || l.starts_with("# ") {
            current_section = "";
            continue;
        }

        // Headline is usually right after the name
        if headline.is_none() && !l.is_empty() && l.len() > 10 && l.len() < 200
            && !l.starts_with("[") && !l.starts_with("*") && !l.starts_with("#")
            && !l.contains("Sign in") && !l.contains("Join now") && !l.contains("LinkedIn")
            && !l.contains("http") {
            headline = Some(l.to_string());
        }

        match current_section {
            "about" if !l.is_empty() && about.is_none() => {
                about = Some(l.to_string());
            }
            "experience" if !l.is_empty() && l.len() > 5 && !l.starts_with("[") && !l.starts_with("*") => {
                if experience.len() < 10 {
                    experience.push(l.to_string());
                }
            }
            "education" if !l.is_empty() && l.len() > 5 && !l.starts_with("[") && !l.starts_with("*") => {
                if education.len() < 5 {
                    education.push(l.to_string());
                }
            }
            _ => {}
        }
    }

    LinkedInProfile {
        name,
        headline,
        about,
        experience,
        education,
        recent_posts: vec![],
    }
}

/// Parse profile data from LinkedIn's HTML page.
/// LinkedIn embeds profile JSON in <code> tags as serialized data.
fn parse_html_profile(html: &str, username: &str) -> LinkedInProfile {
    let mut name = String::new();
    let mut headline = None;
    let mut about = None;
    let mut experience = Vec::new();
    let mut education = Vec::new();

    // LinkedIn embeds data in <code> tags with JSON containing "included" arrays
    let doc = scraper::Html::parse_document(html);
    let code_sel = scraper::Selector::parse("code").unwrap();

    for code_el in doc.select(&code_sel) {
        let text = code_el.text().collect::<String>();
        if !text.contains("firstName") || !text.contains("lastName") {
            continue;
        }

        // Decode HTML entities
        let decoded = text
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">");

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&decoded) {
            if let Some(included) = json.get("included").and_then(|v| v.as_array()) {
                for item in included {
                    // Extract name and headline
                    if let (Some(fn_val), Some(ln_val)) = (
                        item.get("firstName").and_then(|v| v.as_str()),
                        item.get("lastName").and_then(|v| v.as_str()),
                    ) {
                        if !fn_val.is_empty() && !ln_val.is_empty() && name.is_empty() {
                            name = format!("{fn_val} {ln_val}");
                        }
                    }

                    if headline.is_none() {
                        if let Some(h) = item.get("headline").and_then(|v| v.as_str()) {
                            if !h.is_empty() {
                                headline = Some(h.to_string());
                            }
                        }
                    }

                    if about.is_none() {
                        if let Some(s) = item.get("summary").and_then(|v| v.as_str()) {
                            if !s.is_empty() {
                                about = Some(s.to_string());
                            }
                        }
                    }

                    // Experience entries
                    if let Some(company) = item.get("companyName").and_then(|v| v.as_str()) {
                        if let Some(title) = item.get("title").and_then(|v| v.as_str()) {
                            experience.push(format!("{title} at {company}"));
                        }
                    }

                    // Education entries
                    if let Some(school) = item.get("schoolName").and_then(|v| v.as_str()) {
                        if !school.is_empty() {
                            education.push(school.to_string());
                        }
                    }
                }
            }

            if !name.is_empty() {
                break; // Found what we need
            }
        }
    }

    // Fallback: try to extract from meta tags / title
    if name.is_empty() {
        let title_sel = scraper::Selector::parse("title").unwrap();
        if let Some(title_el) = doc.select(&title_sel).next() {
            let title_text = title_el.text().collect::<String>();
            // LinkedIn titles are like "Shreyans Tatiya - Something | LinkedIn"
            if let Some(dash_pos) = title_text.find(" - ") {
                name = title_text[..dash_pos].trim().to_string();
                if headline.is_none() {
                    let rest = &title_text[dash_pos + 3..];
                    if let Some(pipe_pos) = rest.find(" | ") {
                        headline = Some(rest[..pipe_pos].trim().to_string());
                    }
                }
            }
        }
    }

    if name.is_empty() {
        name = username.to_string();
    }

    LinkedInProfile {
        name,
        headline,
        about,
        experience,
        education,
        recent_posts: vec![],
    }
}

fn parse_voyager_profile(body: &str, username: &str) -> Result<LinkedInProfile, FetchError> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| FetchError::HttpError(format!("Failed to parse LinkedIn response: {e}")))?;

    let elements = json.get("included")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

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

    #[test]
    fn test_normalize_company_url() {
        assert_eq!(normalize_company_url("razorpay"), "https://www.linkedin.com/company/razorpay");
        assert_eq!(normalize_company_url("https://linkedin.com/company/razorpay"), "https://linkedin.com/company/razorpay");
    }

    #[test]
    fn test_normalize_profile_url() {
        assert_eq!(normalize_profile_url("johndoe"), "https://www.linkedin.com/in/johndoe");
        assert_eq!(normalize_profile_url("https://linkedin.com/in/johndoe"), "https://linkedin.com/in/johndoe");
    }
}
