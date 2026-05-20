use url::Url;

#[derive(Debug, Clone)]
pub struct RobotsInfo {
    pub sitemaps: Vec<String>,
    pub disallowed: Vec<String>,
    pub crawl_delay: Option<u64>,
}

pub fn parse_robots_txt(content: &str, _user_agent: &str) -> RobotsInfo {
    let mut sitemaps = Vec::new();
    let mut disallowed = Vec::new();
    let mut crawl_delay = None;
    let mut in_matching_agent = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim().to_string();

            match key.as_str() {
                "sitemap" => {
                    if !value.is_empty() {
                        sitemaps.push(value);
                    }
                }
                "user-agent" => {
                    in_matching_agent = value == "*";
                }
                "disallow" if in_matching_agent => {
                    if !value.is_empty() {
                        disallowed.push(value);
                    }
                }
                "crawl-delay" if in_matching_agent => {
                    crawl_delay = value.parse().ok();
                }
                _ => {}
            }
        }
    }

    RobotsInfo {
        sitemaps,
        disallowed,
        crawl_delay,
    }
}

pub fn is_allowed(path: &str, disallowed: &[String]) -> bool {
    for rule in disallowed {
        if path.starts_with(rule.as_str()) {
            return false;
        }
    }
    true
}

/// Parse a sitemap XML (either urlset or sitemapindex) and return URLs
pub fn parse_sitemap_xml(xml: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut in_loc = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                if e.name().as_ref() == b"loc" {
                    in_loc = true;
                }
            }
            Ok(quick_xml::events::Event::Text(e)) => {
                if in_loc {
                    if let Ok(text) = e.unescape() {
                        let url = text.trim().to_string();
                        if !url.is_empty() {
                            urls.push(url);
                        }
                    }
                }
            }
            Ok(quick_xml::events::Event::End(e)) => {
                if e.name().as_ref() == b"loc" {
                    in_loc = false;
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    urls
}

/// Check if a sitemap URL points to a sitemap index
pub fn is_sitemap_index(xml: &str) -> bool {
    xml.contains("<sitemapindex") || xml.contains("<sitemapindex>")
}

/// Standard sitemap paths to try
pub fn sitemap_paths() -> Vec<&'static str> {
    vec![
        "/sitemap.xml",
        "/sitemap_index.xml",
        "/sitemap-index.xml",
        "/sitemaps/sitemap.xml",
        "/wp-sitemap.xml",
    ]
}

/// RSS feed paths to try as fallback
pub fn rss_paths() -> Vec<&'static str> {
    vec![
        "/feed",
        "/rss",
        "/atom.xml",
        "/feed.xml",
        "/index.xml",
        "/rss.xml",
    ]
}

/// Extract same-domain links from HTML content
pub fn extract_links(html: &str, base_url: &Url) -> Vec<String> {
    let document = scraper::Html::parse_document(html);
    let selector = scraper::Selector::parse("a[href]").unwrap();
    let mut links = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            if let Ok(resolved) = base_url.join(href) {
                if resolved.host_str() == base_url.host_str()
                    && (resolved.scheme() == "http" || resolved.scheme() == "https")
                {
                    let url_str = resolved.to_string();
                    // Skip anchors, mailto, tel, etc.
                    if !url_str.contains('#') && seen.insert(url_str.clone()) {
                        links.push(url_str);
                    }
                }
            }
        }
    }

    links
}

/// Extract RSS feed URLs from HTML <link> tags
pub fn extract_rss_links(html: &str, base_url: &Url) -> Vec<String> {
    let document = scraper::Html::parse_document(html);
    let selector = scraper::Selector::parse(
        r#"link[type="application/rss+xml"], link[type="application/atom+xml"]"#,
    )
    .unwrap();
    let mut feeds = Vec::new();

    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            if let Ok(resolved) = base_url.join(href) {
                feeds.push(resolved.to_string());
            }
        }
    }

    feeds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_robots_txt() {
        let robots = "User-agent: *\nDisallow: /admin\nDisallow: /private/\nCrawl-delay: 5\n\nSitemap: https://example.com/sitemap.xml\nSitemap: https://example.com/sitemap2.xml\n";
        let info = parse_robots_txt(robots, "*");
        assert_eq!(info.sitemaps.len(), 2);
        assert_eq!(info.disallowed, vec!["/admin", "/private/"]);
        assert_eq!(info.crawl_delay, Some(5));
    }

    #[test]
    fn test_is_allowed() {
        let disallowed = vec!["/admin".to_string(), "/private/".to_string()];
        assert!(is_allowed("/public/page", &disallowed));
        assert!(!is_allowed("/admin/login", &disallowed));
        assert!(!is_allowed("/private/data", &disallowed));
    }

    #[test]
    fn test_parse_sitemap_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/page1</loc></url>
  <url><loc>https://example.com/page2</loc></url>
</urlset>"#;
        let urls = parse_sitemap_xml(xml);
        assert_eq!(urls, vec!["https://example.com/page1", "https://example.com/page2"]);
    }

    #[test]
    fn test_parse_sitemap_index() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap><loc>https://example.com/sitemap1.xml</loc></sitemap>
  <sitemap><loc>https://example.com/sitemap2.xml</loc></sitemap>
</sitemapindex>"#;
        assert!(is_sitemap_index(xml));
        let urls = parse_sitemap_xml(xml);
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn test_extract_links() {
        let html = r##"<html><body>
            <a href="/page1">Page 1</a>
            <a href="https://example.com/page2">Page 2</a>
            <a href="https://other.com/external">External</a>
            <a href="#anchor">Anchor</a>
        </body></html>"##;
        let base = Url::parse("https://example.com").unwrap();
        let links = extract_links(html, &base);
        assert!(links.contains(&"https://example.com/page1".to_string()));
        assert!(links.contains(&"https://example.com/page2".to_string()));
        assert!(!links.iter().any(|l| l.contains("other.com")));
        assert!(!links.iter().any(|l| l.contains("#")));
    }
}
