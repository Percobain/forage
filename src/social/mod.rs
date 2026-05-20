pub mod linkedin;
pub mod x;
pub mod apollo;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CompanyInfo {
    pub name: String,
    pub domain: Option<String>,
    pub industry: Option<String>,
    pub size: Option<String>,
    pub hq: Option<String>,
    pub description: Option<String>,
    pub linkedin_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkedInProfile {
    pub name: String,
    pub headline: Option<String>,
    pub about: Option<String>,
    pub experience: Vec<String>,
    pub education: Vec<String>,
    pub recent_posts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct XProfile {
    pub handle: String,
    pub bio: Option<String>,
    pub followers: Option<u64>,
    pub following: Option<u64>,
    pub recent_tweets: Vec<Tweet>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tweet {
    pub text: String,
    pub created_at: Option<String>,
    pub likes: Option<u64>,
    pub retweets: Option<u64>,
}
