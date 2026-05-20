# Forage

An MCP server built in Rust that gives LLMs the ability to actually fetch, crawl, and research the real web — including sites they normally can't access.

## The Problem

LLMs are blind to the live web. When you ask Claude or ChatGPT to "check this company's website" or "find me startups in this space," they either hallucinate, give stale training data, or hit a wall. Specifically:

- **Cloudflare and bot protection block LLM fetches.** Most company websites, landing pages, and blogs sit behind Cloudflare, Akamai, or similar WAFs. LLMs get 403'd. The page might as well not exist.
- **No web search built in.** LLMs can't Google things. They can't go to DuckDuckGo and search for "fintech startups in Mumbai." They're stuck with what's in their training data.
- **No crawling ability.** Even if an LLM could fetch one page, it can't discover and crawl an entire site — follow sitemaps, parse RSS feeds, BFS through links.
- **Social platforms are walled gardens.** LinkedIn profiles, X/Twitter bios, company data on Apollo — all behind auth walls that LLMs have zero access to.
- **No caching or rate limiting.** Even with tool use, naive fetch implementations hit rate limits, re-fetch the same pages, and waste time.

The result: any research task that requires looking at real, live websites becomes a manual grind. You end up doing the tab-hopping yourself.

## The Solution

Forage is an MCP server that runs locally on your machine. It gives Claude (or any MCP-compatible LLM) a set of research tools that actually work against the real web:

- **Tiered fetch with Cloudflare bypass.** Direct fetch first, automatic fallback to Jina Reader (which handles JS rendering and bot protection) for blocked sites.
- **Full site crawling.** Discovers pages via sitemap.xml, RSS feeds, or BFS link crawling. Fetches everything in parallel. Respects robots.txt.
- **Web search without API keys.** Searches DuckDuckGo's lite endpoint — no keys, no billing, just results.
- **Social platform access.** LinkedIn and X profiles via cookie-based auth. Apollo.io for company search. All rate-limited to protect accounts.
- **Aggressive SQLite caching.** 24h cache on fetches, 6h on search. No redundant requests. Rate limit counters persist across restarts.
- **Built in Rust.** Async, parallel, fast. Crawl a 50-page site in seconds, not minutes.

One prompt replaces a week of research:

> Find me 30 fintech companies in Mumbai sized 11-50 employees. For each, crawl their website, pull a description, and find the founder's LinkedIn. Rank by relevance. Markdown table.

Claude calls `search_web` → `crawl_site` (x30) → `fetch_profile_linkedin` → returns a sourced table. That's it.

## Tools

Forage exposes 7 tools via the Model Context Protocol:

| Tool | What it does |
|------|-------------|
| `fetch_url` | Fetch any URL → clean markdown. Tiered: direct fetch → Jina Reader fallback for Cloudflare sites. Cached 24h. |
| `crawl_site` | Discover pages via sitemap/RSS/BFS, fetch all in parallel, return markdown per page. Respects robots.txt. |
| `search_web` | DuckDuckGo search → structured results (title, URL, snippet). No API key. |
| `fetch_archive` | Wayback Machine historical snapshots. |
| `find_companies` | Apollo.io company search by keywords, size, location. |
| `fetch_profile_linkedin` | LinkedIn profile via cookie auth. Rate-limited (8-15s delay, 80/day cap). |
| `fetch_profile_x` | X/Twitter profile via cookie auth. Rate-limited (6-12s delay, 200/day cap). |

## Architecture

```
┌──────────────────────────────────────────────┐
│  MCP Server (rmcp, stdio transport)          │
│  7 tools exposed to any MCP client           │
└─────────────────┬────────────────────────────┘
                  │
┌─────────────────▼────────────────────────────┐
│  Core Service (Rust, async tokio)            │
│                                              │
│  ┌────────────┐ ┌────────────┐ ┌──────────┐ │
│  │ Open Web   │ │ Social     │ │ Search   │ │
│  │ - Direct   │ │ - LinkedIn │ │ - DDG    │ │
│  │   fetch    │ │   (cookie) │ │ - Wayback│ │
│  │ - Jina     │ │ - X/Twitter│ │          │ │
│  │   fallback │ │   (cookie) │ │          │ │
│  │ - Sitemap  │ │ - Apollo   │ │          │ │
│  │ - Crawler  │ │   (API key)│ │          │ │
│  └────────────┘ └────────────┘ └──────────┘ │
│                                              │
│  Cross-cutting:                              │
│   - SQLite cache (24h TTL, configurable)     │
│   - Per-platform rate limiter with jitter    │
│   - Cookie vault (~/.forage/cookies/)        │
│   - Structured file logging (tracing)        │
└──────────────────────────────────────────────┘
```

## How the Crawler Works

The site crawler follows a discovery chain for maximum coverage:

1. Fetch `robots.txt` → parse `Sitemap:` directives + `Disallow:` rules
2. Try sitemap URLs from robots.txt
3. Try standard paths: `/sitemap.xml`, `/sitemap_index.xml`, `/wp-sitemap.xml`
4. If sitemap index → recursively fetch child sitemaps
5. Fallback to RSS: `/feed`, `/rss`, `/atom.xml`, `<link rel="alternate">` tags
6. Final fallback: BFS link crawl from homepage (same-domain, max depth 3)
7. Parallel fetch all discovered URLs (10 concurrent by default)
8. Each fetch uses the tiered fallback: direct → Jina Reader

## Tech Stack

| Layer | Choice |
|---|---|
| Language | Rust (2021 edition) |
| Async | tokio |
| HTTP | reqwest (rustls-tls) |
| HTML to Markdown | htmd |
| HTML parsing | scraper |
| XML (sitemaps) | quick-xml |
| MCP SDK | rmcp |
| Cache | rusqlite (SQLite, bundled) |
| Config | serde + toml |
| Logging | tracing + tracing-appender |

## Project Structure

```
forage/
├── Cargo.toml
├── config.toml.example
├── src/
│   ├── main.rs              # Entry: MCP stdio server, test-fetch, doctor
│   ├── tools.rs             # 7 MCP tool definitions + dispatch
│   ├── config.rs            # TOML config with defaults
│   ├── cache.rs             # SQLite cache + rate limit persistence
│   ├── rate_limit.rs        # Per-platform throttle with jitter + daily caps
│   ├── fetch/
│   │   ├── mod.rs           # Tiered fallback: direct → Jina
│   │   ├── direct.rs        # reqwest fetch + HTML→Markdown
│   │   ├── jina.rs          # Jina Reader API client
│   │   ├── sitemap.rs       # robots.txt, sitemap.xml, RSS, link extraction
│   │   └── crawler.rs       # Discovery chain → parallel fetch
│   ├── search/
│   │   ├── mod.rs
│   │   ├── duckduckgo.rs    # DDG lite HTML scraping (POST, no key needed)
│   │   └── wayback.rs       # Internet Archive Wayback Machine
│   └── social/
│       ├── mod.rs
│       ├── linkedin.rs      # Voyager API client (cookie auth)
│       ├── x.rs             # GraphQL client (cookie auth)
│       └── apollo.rs        # Apollo.io REST API (key auth)
├── python_helpers/
│   └── instagram_helper.py  # instagrapi subprocess wrapper
└── cookies/                 # GITIGNORED
```

## Quick Start

### Build

```bash
git clone git@github.com:Percobain/forage.git
cd forage
cargo build --release
```

Binary: `target/release/forage` (or `forage.exe` on Windows).

### Configure (optional)

```bash
mkdir -p ~/.forage
cp config.toml.example ~/.forage/config.toml
```

Everything works out of the box without a config file. You only need config for:
- `find_companies` → `apollo.api_key`
- `fetch_profile_linkedin` → cookie file at `~/.forage/cookies/linkedin.json`
- `fetch_profile_x` → cookie file at `~/.forage/cookies/x.json`

### Connect to Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "forage": {
      "command": "/absolute/path/to/forage"
    }
  }
}
```

Restart Claude Desktop.

### Connect to Claude Code

```bash
claude mcp add forage /absolute/path/to/forage
```

### CLI Testing (no Claude needed)

```bash
# Smoke test a URL fetch
forage test-fetch https://example.com

# Check config, cookies, API keys
forage doctor

# Test MCP protocol directly
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | forage
```

### Cookie Setup (LinkedIn / X)

Use a **burner account** — never your main:

1. Log in via browser
2. Install "Cookie-Editor" extension
3. Export cookies as JSON
4. Save to `~/.forage/cookies/linkedin.json` or `~/.forage/cookies/x.json`

```json
{
  "platform": "linkedin",
  "cookies": [
    {"name": "li_at", "value": "AQED...", "domain": ".linkedin.com"},
    {"name": "JSESSIONID", "value": "ajax:...", "domain": ".linkedin.com"}
  ]
}
```

## Caching

SQLite-based, automatic:
- Web fetches: 24h TTL
- Search results: 6h TTL
- All tools support `use_cache: false` to force refresh
- Rate limit counters persist across restarts

## License

MIT
