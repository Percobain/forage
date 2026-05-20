use super::CompanyInfo;
use crate::fetch::FetchError;

pub struct ApolloClient {
    api_key: String,
    client: reqwest::Client,
}

impl ApolloClient {
    pub fn new(api_key: &str) -> Result<Self, FetchError> {
        if api_key.is_empty() {
            return Err(FetchError::HttpError(
                "Apollo API key not configured. Add apollo.api_key to config.toml".to_string(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        Ok(Self {
            api_key: api_key.to_string(),
            client,
        })
    }

    pub async fn find_companies(
        &self,
        keywords: &str,
        size: Option<&str>,
        location: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CompanyInfo>, FetchError> {
        let size_ranges = size.map(|s| match s.to_uppercase().as_str() {
            "A" => vec!["1"],
            "B" => vec!["2,10"],
            "C" => vec!["11,50"],
            "D" => vec!["51,200"],
            "E" => vec!["201,500"],
            "F" => vec!["501,1000"],
            "G" => vec!["1001,5000"],
            _ => vec![s],
        });

        let mut body = serde_json::json!({
            "q_organization_keyword_tags": [keywords],
            "page": 1,
            "per_page": limit.min(100),
        });

        if let Some(ranges) = &size_ranges {
            body["organization_num_employees_ranges"] = serde_json::json!(ranges);
        }

        if let Some(loc) = location {
            body["organization_locations"] = serde_json::json!([loc]);
        }

        let response = self.client
            .post("https://api.apollo.io/v1/mixed_companies/search")
            .header("Content-Type", "application/json")
            .header("Cache-Control", "no-cache")
            .query(&[("api_key", &self.api_key)])
            .json(&body)
            .send()
            .await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        let status = response.status().as_u16();
        let response_body = response
            .text()
            .await
            .map_err(|e| FetchError::HttpError(e.to_string()))?;

        if status != 200 {
            return Err(FetchError::HttpError(format!(
                "Apollo API returned status {status}: {response_body}"
            )));
        }

        let json: serde_json::Value = serde_json::from_str(&response_body)
            .map_err(|e| FetchError::HttpError(format!("Failed to parse Apollo response: {e}")))?;

        let organizations = json
            .get("organizations")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let companies: Vec<CompanyInfo> = organizations
            .iter()
            .map(|org| CompanyInfo {
                name: org.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                domain: org.get("primary_domain").and_then(|v| v.as_str()).map(String::from),
                industry: org.get("industry").and_then(|v| v.as_str()).map(String::from),
                size: org.get("estimated_num_employees").and_then(|v| v.as_u64()).map(|n| n.to_string()),
                hq: extract_hq(org),
                description: org.get("short_description").and_then(|v| v.as_str()).map(String::from),
                linkedin_url: org.get("linkedin_url").and_then(|v| v.as_str()).map(String::from),
            })
            .take(limit)
            .collect();

        Ok(companies)
    }
}

fn extract_hq(org: &serde_json::Value) -> Option<String> {
    let city = org.get("city").and_then(|v| v.as_str()).unwrap_or("");
    let state = org.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let country = org.get("country").and_then(|v| v.as_str()).unwrap_or("");

    let parts: Vec<&str> = [city, state, country]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect();

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}
