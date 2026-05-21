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

    # Name from title: "Firstname Lastname | LinkedIn"
    if " | " in title and "LinkedIn" in title:
        result["name"] = title.split(" | ")[0].strip()

    # Parse visible text — LinkedIn's page structure is predictable:
    # [nav items] → Name → Headline → location → Contact info → org → school → ...
    # → About → about text → Experience → ... → Education → ...

    skip_nav = {"Home", "My Network", "Jobs", "Messaging", "Notifications", "Me",
                "For Business", "Skip to main content", "More", "Message", "Connect",
                "Follow", "He/Him", "She/Her", "They/Them", "Contact info"}

    current_section = ""
    name_found = False

    for i, line in enumerate(lines):
        # Skip nav and UI elements
        if line in skip_nav or line.startswith("0 notification") or line.startswith("Try Premium"):
            continue

        # Name is the first non-nav line (appears twice, take first)
        if not name_found and not result["name"] and len(line) > 2 and len(line) < 80:
            if not any(kw in line.lower() for kw in ["notification", "skip to", "home", "premium", "business"]):
                result["name"] = line
                name_found = True
                continue

        # Headline is right after name (may appear twice, take the longer one)
        if name_found and not result["headline"] and line != result["name"]:
            if len(line) > 10 and line not in skip_nav and not line.startswith("He/") and not line.startswith("She/") and not line.startswith("They/"):
                result["headline"] = line
                continue

        # Location: "City, State, Country" pattern
        if name_found and not result["location"]:
            if any(loc in line for loc in ["India", "United States", "UK", "Canada", "Singapore",
                                           "Mumbai", "Bangalore", "Delhi", "Pune", "Hyderabad",
                                           "Gurgaon", "Chennai", "Kolkata", "San Francisco",
                                           "New York", "London"]):
                if len(line) < 80:
                    result["location"] = line
                    continue

        # Section headers
        if line == "About":
            current_section = "about"
            continue
        elif line == "Experience":
            current_section = "experience"
            continue
        elif line == "Education":
            current_section = "education"
            continue
        elif line in ("Featured", "Activity", "Skills", "Interests",
                      "Recommendations", "Courses", "Projects", "Licenses & certifications"):
            current_section = ""
            continue

        # Parse sections
        if current_section == "about" and not result["about"]:
            if len(line) > 10:
                result["about"] = line
                current_section = ""

        elif current_section == "experience":
            if len(line) > 5 and line not in skip_nav:
                # Skip dates and durations
                if not any(kw in line for kw in ["·", "mos", "yrs", "Present"]):
                    if len(result["experience"]) < 10:
                        result["experience"].append(line)

        elif current_section == "education":
            if len(line) > 5 and line not in skip_nav:
                if not any(kw in line for kw in ["·", "Grade", "GPA"]):
                    if len(result["education"]) < 5:
                        result["education"].append(line)

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
