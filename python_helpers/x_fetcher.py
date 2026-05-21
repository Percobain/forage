#!/usr/bin/env python3
"""
X/Twitter profile fetcher using Playwright + stealth.
Called as subprocess from Forage MCP server.

Usage:
    python x_fetcher.py profile <handle>

Reads cookies from ~/.forage/cookies/x.json (optional - works without for public profiles)
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
        cookie = {
            "name": c["name"],
            "value": c["value"],
            "domain": c.get("domain", ".x.com"),
            "path": c.get("path", "/"),
            "secure": c.get("secure", True),
            "httpOnly": c.get("httpOnly", False),
        }
        ss = c.get("sameSite", "no_restriction")
        if ss == "no_restriction":
            cookie["sameSite"] = "None"
        elif ss == "lax":
            cookie["sameSite"] = "Lax"
        elif ss == "strict":
            cookie["sameSite"] = "Strict"
        else:
            cookie["sameSite"] = "None"
        pw_cookies.append(cookie)

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
            await page.wait_for_timeout(4000)

            title = await page.title()
            content = await page.evaluate("document.body.innerText")

            await browser.close()

            # Parse
            result = {
                "handle": handle,
                "url": url,
                "name": "",
                "bio": "",
                "tweets": [],
                "source": "playwright_stealth",
            }

            if " / " in title:
                parts = title.split(" / ")
                name_part = parts[0].strip()
                if " (" in name_part:
                    result["name"] = name_part.split(" (")[0].strip()
                else:
                    result["name"] = name_part

            lines = [l.strip() for l in content.split("\n") if l.strip()]
            # Bio is usually near the top, after the handle
            found_handle = False
            for l in lines:
                if f"@{handle}" in l.lower() or handle.lower() in l.lower():
                    found_handle = True
                    continue
                if found_handle and not result["bio"] and len(l) > 10 and len(l) < 300:
                    if not any(kw in l.lower() for kw in ["following", "followers", "joined", "posts", "sign up"]):
                        result["bio"] = l
                        break

            print(json.dumps(result, ensure_ascii=False))

        except Exception as e:
            await browser.close()
            print(json.dumps({"error": str(e), "handle": handle}))


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
