#!/usr/bin/env python3
"""
X/Twitter profile fetcher using Playwright + stealth.
Called as subprocess from Forage MCP server.

Usage:
    python x_fetcher.py profile <handle>

Reads cookies from ~/.forage/cookies/x.json
Outputs JSON to stdout.
"""

import json
import sys
import asyncio
from pathlib import Path

COOKIE_PATH = Path.home() / ".forage" / "cookies" / "x.json"


def load_cookies():
    if not COOKIE_PATH.exists():
        return []
    with open(COOKIE_PATH) as f:
        raw = json.load(f)
    if isinstance(raw, list):
        cookies = raw
    elif isinstance(raw, dict) and "cookies" in raw:
        cookies = raw["cookies"]
    else:
        return []

    pw_cookies = []
    for c in cookies:
        ss = c.get("sameSite", "no_restriction")
        if ss == "no_restriction": ss = "None"
        elif ss == "lax": ss = "Lax"
        elif ss == "strict": ss = "Strict"
        else: ss = "None"
        pw_cookies.append({
            "name": c["name"], "value": c["value"],
            "domain": c.get("domain", ".x.com"),
            "path": c.get("path", "/"),
            "secure": c.get("secure", True),
            "httpOnly": c.get("httpOnly", False),
            "sameSite": ss,
        })
    return pw_cookies


async def fetch_profile(handle):
    from playwright.async_api import async_playwright
    try:
        from playwright_stealth import stealth_async
        has_stealth = True
    except ImportError:
        has_stealth = False

    handle = handle.strip().lstrip("@").split("/")[-1].rstrip("/")
    url = f"https://x.com/{handle}"
    cookies = load_cookies()

    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        context = await browser.new_context(
            user_agent="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            viewport={"width": 1920, "height": 1080},
        )
        if cookies:
            await context.add_cookies(cookies)

        page = await context.new_page()
        if has_stealth:
            await stealth_async(page)

        try:
            await page.goto(url, wait_until="domcontentloaded", timeout=30000)
            await page.wait_for_timeout(5000)

            # Scroll down to load more tweets
            await page.evaluate("window.scrollBy(0, 2000)")
            await page.wait_for_timeout(2000)

            title = await page.title()
            content = await page.evaluate("document.body.innerText")

            await browser.close()

            result = parse_profile(content, title, handle, url)
            print(json.dumps(result, ensure_ascii=False))

        except Exception as e:
            await browser.close()
            print(json.dumps({"error": str(e), "handle": handle}))


def parse_profile(content, title, handle, url):
    lines = [l.strip() for l in content.split("\n") if l.strip()]

    result = {
        "handle": handle,
        "url": url,
        "name": "",
        "bio": "",
        "location": "",
        "website": "",
        "joined": "",
        "following": "",
        "followers": "",
        "posts_count": "",
        "tweets": [],
        "source": "playwright_stealth",
    }

    # Name from title: "Display Name (@handle) / X"
    if "(" in title and ")" in title:
        result["name"] = title.split("(")[0].strip()

    # Parse the structured content
    # Pattern: after @handle line, we get bio, then metadata, then tweets
    found_handle = False
    found_bio = False
    in_tweets = False
    current_tweet_author = ""
    current_tweet_lines = []

    i = 0
    while i < len(lines):
        line = lines[i]

        # Find the @handle line (the profile's, not the logged-in user's)
        if line.lower() == f"@{handle.lower()}" and not found_handle:
            found_handle = True
            i += 1
            continue

        if found_handle and not found_bio:
            # Lines after @handle until we hit metadata
            if line.startswith("Joined "):
                result["joined"] = line
                found_bio = True
            elif " Following" in line:
                result["following"] = line
                found_bio = True
            elif " Followers" in line:
                result["followers"] = line
            elif line.endswith(" posts") or line.endswith(" post"):
                result["posts_count"] = line
            elif any(line.startswith(kw) for kw in ["Posts", "Replies", "Media", "Highlights", "Articles", "Affiliates", "Not followed"]):
                pass  # Navigation elements
            elif "Following" in line and "Follower" in line:
                parts = line.split()
                for j, p in enumerate(parts):
                    if p == "Following" and j > 0:
                        result["following"] = parts[j-1] + " Following"
                    if "Follower" in p and j > 0:
                        result["followers"] = parts[j-1] + " Followers"
                found_bio = True
            elif len(line) > 5 and not result["bio"] and line not in ["Follow", "See new posts", "More"]:
                # This is likely the bio
                if not any(kw in line.lower() for kw in ["following", "followers", "joined", "posts", "see new", "not followed"]):
                    result["bio"] = line
            i += 1
            continue

        if found_bio and not in_tweets:
            # Look for metadata lines
            if " Following" in line and not result["following"]:
                result["following"] = line
            elif " Followers" in line and not result["followers"]:
                result["followers"] = line
            elif line.startswith("Joined ") and not result["joined"]:
                result["joined"] = line
            elif line.endswith("posts") or line.endswith("post"):
                if not result["posts_count"]:
                    result["posts_count"] = line

            # Detect start of tweets section
            if line.endswith("'s posts") or line == "Posts":
                in_tweets = True
            i += 1
            continue

        if in_tweets:
            # Parse tweets - pattern: author, @handle, ·, date, then tweet text
            if line.startswith("@") and len(line) < 50:
                # Save previous tweet if any
                if current_tweet_lines:
                    tweet_text = " ".join(current_tweet_lines)
                    if len(tweet_text) > 10:
                        result["tweets"].append({
                            "author": current_tweet_author,
                            "text": tweet_text,
                        })
                current_tweet_author = line
                current_tweet_lines = []
            elif line == "·":
                pass  # separator
            elif line in ["Show more", "Show", "Trending now", "What's happening", "Who to follow", "Terms of Service"]:
                # End of tweets section
                if current_tweet_lines:
                    tweet_text = " ".join(current_tweet_lines)
                    if len(tweet_text) > 10:
                        result["tweets"].append({
                            "author": current_tweet_author,
                            "text": tweet_text,
                        })
                break
            elif any(c.isalpha() for c in line) and len(line) > 3:
                # Skip pure numbers (like/retweet counts), dates
                if not line.replace(",", "").replace(".", "").isdigit():
                    if line not in ["Pinned", "Show this thread", "Replying to"]:
                        current_tweet_lines.append(line)
            i += 1
            continue

        i += 1

    # Limit tweets
    result["tweets"] = result["tweets"][:20]

    return result


async def main():
    if len(sys.argv) < 3:
        print(json.dumps({"error": "Usage: x_fetcher.py profile <handle>"}))
        sys.exit(1)

    cmd = sys.argv[1]
    target = sys.argv[2]

    if cmd == "profile":
        await fetch_profile(target)
    else:
        print(json.dumps({"error": f"Unknown command: {cmd}"}))
        sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main())
