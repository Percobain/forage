#!/usr/bin/env python3
"""
LinkedIn profile/company fetcher using Playwright + stealth.
Called as subprocess from Forage MCP server.

Usage:
    python linkedin_fetcher.py profile <url_or_username>
    python linkedin_fetcher.py company <url_or_slug>

Reads cookies from ~/.forage/cookies/linkedin.json
Outputs JSON to stdout.
"""

import json
import sys
import asyncio
import os
from pathlib import Path

COOKIE_PATH = Path.home() / ".forage" / "cookies" / "linkedin.json"


def load_cookies():
    if not COOKIE_PATH.exists():
        print(json.dumps({"error": f"No LinkedIn cookies at {COOKIE_PATH}"}))
        sys.exit(1)

    with open(COOKIE_PATH) as f:
        raw = json.load(f)

    # Handle both array (Cookie-Editor) and wrapped format
    if isinstance(raw, list):
        cookies = raw
    elif isinstance(raw, dict) and "cookies" in raw:
        cookies = raw["cookies"]
    else:
        print(json.dumps({"error": "Invalid cookie format"}))
        sys.exit(1)

    # Convert to Playwright format
    pw_cookies = []
    for c in cookies:
        cookie = {
            "name": c["name"],
            "value": c["value"],
            "domain": c.get("domain", ".linkedin.com"),
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


async def fetch_page(url, cookies):
    from playwright.async_api import async_playwright

    try:
        from playwright_stealth import stealth_async
        has_stealth = True
    except ImportError:
        has_stealth = False

    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        context = await browser.new_context(
            user_agent="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            viewport={"width": 1920, "height": 1080},
            locale="en-US",
        )

        await context.add_cookies(cookies)

        page = await context.new_page()
        if has_stealth:
            await stealth_async(page)

        try:
            await page.goto(url, wait_until="domcontentloaded", timeout=30000)
            await page.wait_for_timeout(3000)

            title = await page.title()
            content = await page.evaluate("document.body.innerText")
            html = await page.content()

            await browser.close()
            return {"title": title, "content": content, "html": html, "url": url}

        except Exception as e:
            await browser.close()
            return {"error": str(e), "url": url}


def parse_profile_content(data):
    """Extract structured profile info from page text content."""
    content = data.get("content", "")
    title = data.get("title", "")
    html = data.get("html", "")
    lines = [l.strip() for l in content.split("\n") if l.strip()]

    result = {
        "url": data.get("url", ""),
        "name": "",
        "headline": "",
        "location": "",
        "about": "",
        "experience": [],
        "education": [],
        "source": "playwright_stealth",
    }

    # Name from title: "Firstname Lastname - Headline | LinkedIn"
    if " - " in title and "LinkedIn" in title:
        parts = title.split(" - ", 1)
        result["name"] = parts[0].strip()
        if " | " in parts[1]:
            result["headline"] = parts[1].split(" | ")[0].strip()

    # Also try extracting from embedded JSON in HTML
    import re
    for match in re.finditer(r"<code[^>]*>(.*?)</code>", html, re.DOTALL):
        chunk = match.group(1)
        if "firstName" not in chunk or "lastName" not in chunk:
            continue
        decoded = chunk.replace("&quot;", '"').replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
        try:
            obj = json.loads(decoded)
            for item in obj.get("included", []):
                fn = item.get("firstName", "")
                ln = item.get("lastName", "")
                if fn and ln and not result["name"]:
                    result["name"] = f"{fn} {ln}"
                hl = item.get("headline", "")
                if hl and not result["headline"]:
                    result["headline"] = hl
                loc = item.get("geoLocationName", "") or item.get("locationName", "")
                if loc and not result["location"]:
                    result["location"] = loc
                summary = item.get("summary", "")
                if summary and not result["about"]:
                    result["about"] = summary
                company = item.get("companyName", "")
                ttl = item.get("title", "")
                if company and ttl:
                    result["experience"].append(f"{ttl} at {company}")
                school = item.get("schoolName", "")
                if school:
                    result["education"].append(school)
            if result["name"]:
                break
        except json.JSONDecodeError:
            pass

    # Fallback: parse from visible text content
    if not result["about"]:
        in_about = False
        for l in lines:
            if l == "About":
                in_about = True
                continue
            if in_about:
                if l in ("Experience", "Education", "Activity", "Skills"):
                    break
                if len(l) > 20:
                    result["about"] = l
                    break

    return result


def parse_company_content(data):
    """Extract structured company info from page text content."""
    content = data.get("content", "")
    title = data.get("title", "")
    lines = [l.strip() for l in content.split("\n") if l.strip()]

    result = {
        "url": data.get("url", ""),
        "name": "",
        "industry": "",
        "size": "",
        "location": "",
        "description": "",
        "website": "",
        "source": "playwright_stealth",
    }

    # Name from title
    if " | " in title:
        result["name"] = title.split(" | ")[0].strip()

    # Parse visible content
    in_about = False
    for i, l in enumerate(lines):
        if l == "About" or l == "Overview":
            in_about = True
            continue
        if in_about and not result["description"] and len(l) > 30:
            result["description"] = l
            in_about = False

        if "employees" in l.lower() and not result["size"]:
            result["size"] = l
        if "followers" in l.lower() and "on LinkedIn" in l:
            pass
        if any(loc in l for loc in ["Mumbai", "Bangalore", "Bengaluru", "Delhi", "Gurgaon", "Gurugram", "Pune", "Hyderabad", "Chennai"]):
            if not result["location"] and len(l) < 100:
                result["location"] = l

    return result


async def main():
    if len(sys.argv) < 3:
        print(json.dumps({"error": "Usage: linkedin_fetcher.py <profile|company> <url_or_username>"}))
        sys.exit(1)

    cmd = sys.argv[1]
    target = sys.argv[2]

    cookies = load_cookies()

    if cmd == "profile":
        if not target.startswith("http"):
            url = f"https://www.linkedin.com/in/{target}/"
        else:
            url = target
        data = await fetch_page(url, cookies)
        if "error" in data:
            print(json.dumps(data))
        else:
            result = parse_profile_content(data)
            print(json.dumps(result, ensure_ascii=False))

    elif cmd == "company":
        if not target.startswith("http"):
            url = f"https://www.linkedin.com/company/{target}/"
        else:
            url = target
        data = await fetch_page(url, cookies)
        if "error" in data:
            print(json.dumps(data))
        else:
            result = parse_company_content(data)
            print(json.dumps(result, ensure_ascii=False))

    else:
        print(json.dumps({"error": f"Unknown command: {cmd}"}))
        sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main())
