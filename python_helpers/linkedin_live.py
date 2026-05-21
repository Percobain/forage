"""
LinkedIn live browser session for Forage MCP server.

Commands via stdin (JSON per line):
  {"cmd": "search_people", "query": "Stackby CTO"}
  {"cmd": "get_profile", "username": "shreyanstatiya"}
  {"cmd": "search_company_people", "company": "Stackby", "title": "CTO"}
  {"cmd": "quit"}

Outputs JSON per line to stdout.

Requires: User must have logged into LinkedIn at least once
via `python linkedin_live.py login` which saves a persistent browser profile.
"""

import sys
import json
import asyncio
import random
import os

PROFILE_DIR = os.path.join(os.path.expanduser("~"), ".forage", "linkedin_browser")


async def main():
    from playwright.async_api import async_playwright

    mode = sys.argv[1] if len(sys.argv) > 1 else "serve"

    async with async_playwright() as p:
        headless = mode == "serve"
        context = await p.chromium.launch_persistent_context(
            PROFILE_DIR,
            headless=headless,
            viewport={"width": 1280, "height": 900},
            locale="en-US",
        )
        page = context.pages[0] if context.pages else await context.new_page()

        # Check login state
        await page.goto("https://www.linkedin.com/feed/", timeout=30000)
        await page.wait_for_timeout(3000)

        url = page.url
        if "login" in url or "authwall" in url or "signup" in url or "Sign Up" in (await page.title()):
            if mode == "login":
                print("Log into LinkedIn in the browser window.", file=sys.stderr)
                print("Waiting for login...", file=sys.stderr)
                for _ in range(120):  # wait up to 4 mins
                    await page.wait_for_timeout(2000)
                    if "/feed" in page.url:
                        break
                if "/feed" not in page.url:
                    print(json.dumps({"status": "error", "message": "Login timeout"}), flush=True)
                    await context.close()
                    return
                print(json.dumps({"status": "logged_in"}), flush=True)
                print("Login saved! You can now close this window.", file=sys.stderr)
                await page.wait_for_timeout(2000)
                await context.close()
                return
            else:
                print(json.dumps({"status": "not_logged_in", "message": "Run: python linkedin_live.py login"}), flush=True)
                await context.close()
                return

        print(json.dumps({"status": "ready"}), flush=True)

        if mode == "login":
            print("Already logged in!", file=sys.stderr)
            await context.close()
            return

        # Serve mode: read commands from stdin
        loop = asyncio.get_event_loop()
        while True:
            line = await loop.run_in_executor(None, sys.stdin.readline)
            if not line:
                break
            line = line.strip()
            if not line:
                continue

            try:
                cmd = json.loads(line)
            except json.JSONDecodeError:
                # Treat as simple username
                cmd = {"cmd": "get_profile", "username": line}

            action = cmd.get("cmd", "")

            if action == "quit":
                break

            elif action == "get_profile":
                username = cmd["username"].strip("/").split("/")[-1]
                result = await fetch_profile(page, username)
                print(json.dumps(result, ensure_ascii=False), flush=True)

            elif action == "search_people":
                query = cmd["query"]
                limit = cmd.get("limit", 10)
                result = await search_people(page, query, limit)
                print(json.dumps(result, ensure_ascii=False), flush=True)

            elif action == "search_company_people":
                company = cmd["company"]
                title = cmd.get("title", "")
                limit = cmd.get("limit", 5)
                query = f"{company} {title}".strip()
                result = await search_people(page, query, limit)
                print(json.dumps(result, ensure_ascii=False), flush=True)

            else:
                print(json.dumps({"error": f"Unknown command: {action}"}), flush=True)

            # Random delay between requests
            await page.wait_for_timeout(random.randint(1500, 3000))

        await context.close()


async def fetch_profile(page, username):
    """Fetch a single LinkedIn profile."""
    url = f"https://www.linkedin.com/in/{username}/"
    try:
        await page.goto(url, wait_until="domcontentloaded", timeout=30000)
        await page.wait_for_timeout(3000 + random.randint(0, 2000))

        title = await page.title()
        if "authwall" in page.url or "Sign Up" in title:
            return {"error": "session_expired", "username": username}

        content = await page.evaluate("document.body.innerText")
        return parse_profile(content, title, username, url)

    except Exception as e:
        return {"error": str(e), "username": username}


async def search_people(page, query, limit=10):
    """Search LinkedIn for people matching a query."""
    import urllib.parse
    search_url = f"https://www.linkedin.com/search/results/people/?keywords={urllib.parse.quote(query)}&origin=GLOBAL_SEARCH_HEADER"

    try:
        await page.goto(search_url, wait_until="domcontentloaded", timeout=30000)
        await page.wait_for_timeout(3000 + random.randint(0, 2000))

        title = await page.title()
        if "authwall" in page.url or "Sign Up" in title:
            return {"error": "session_expired", "query": query}

        # Scroll to load more results
        await page.evaluate("window.scrollBy(0, 1500)")
        await page.wait_for_timeout(1500)

        # Extract profile links and snippets
        results = await page.evaluate("""() => {
            const cards = document.querySelectorAll('.reusable-search__result-container, li.reusable-search-simple-insight, div[data-view-name="search-entity-result-universal-template"]');
            const results = [];

            // Get all profile links with their surrounding text
            const links = document.querySelectorAll('a[href*="/in/"]');
            const seen = new Set();

            for (const a of links) {
                const href = a.href.split('?')[0];
                const text = a.innerText.trim();

                // Skip nav links, empty, duplicates
                if (!text || text.length < 3 || text.length > 100 || seen.has(href)) continue;
                if (href.includes('/in/AC') || text.includes('Search') || text.includes('LinkedIn')) continue;

                seen.add(href);

                // Try to get the parent card's full text for headline
                let cardText = '';
                let parent = a.closest('li') || a.closest('div[data-view-name]');
                if (parent) {
                    cardText = parent.innerText.trim();
                }

                results.push({
                    name: text,
                    linkedin_url: href,
                    card_text: cardText.substring(0, 300),
                });
            }
            return results;
        }""")

        # Dedupe and limit
        seen = set()
        unique = []
        for r in results:
            if r["linkedin_url"] not in seen and len(unique) < limit:
                seen.add(r["linkedin_url"])
                # Parse headline from card text
                card_lines = [l.strip() for l in r["card_text"].split("\n") if l.strip()]
                headline = ""
                for i, l in enumerate(card_lines):
                    if l == r["name"] and i + 1 < len(card_lines):
                        # Next non-empty line after name is usually headline
                        for j in range(i + 1, min(i + 3, len(card_lines))):
                            candidate = card_lines[j]
                            if candidate not in ("Connect", "Message", "Follow", "Pending") and len(candidate) > 5:
                                headline = candidate
                                break
                        break

                unique.append({
                    "name": r["name"],
                    "headline": headline,
                    "linkedin_url": r["linkedin_url"],
                })

        return {"query": query, "results": unique}

    except Exception as e:
        return {"error": str(e), "query": query}


def parse_profile(content, title, username, url):
    """Parse profile data from page text."""
    lines = [l.strip() for l in content.split("\n") if l.strip()]

    result = {
        "username": username,
        "url": url,
        "name": "",
        "headline": "",
        "location": "",
        "about": "",
        "experience": [],
        "education": [],
    }

    # Name from title
    if " | " in title:
        result["name"] = title.split(" | ")[0].strip()

    skip = {"Home", "My Network", "Jobs", "Messaging", "Notifications", "Me",
            "For Business", "Skip to main content", "More", "Message", "Connect",
            "Follow", "He/Him", "She/Her", "They/Them", "Contact info"}

    section = ""
    name_found = False

    for line in lines:
        if line in skip or "notification" in line.lower() or line.startswith("Try Premium"):
            continue

        if not name_found and not result["name"] and len(line) > 2 and len(line) < 80:
            result["name"] = line
            name_found = True
            continue

        if name_found and not result["headline"] and line != result["name"]:
            if len(line) > 10 and line not in skip:
                result["headline"] = line
                continue

        if not result["location"] and any(loc in line for loc in
            ["India", "Mumbai", "Bangalore", "Delhi", "Pune", "Hyderabad",
             "Gurgaon", "Chennai", "United States", "London", "Singapore"]):
            if len(line) < 80:
                result["location"] = line
                continue

        if line == "About":
            section = "about"
            continue
        elif line == "Experience":
            section = "experience"
            continue
        elif line == "Education":
            section = "education"
            continue
        elif line in ("Featured", "Activity", "Skills", "Interests",
                      "Recommendations", "Licenses & certifications"):
            section = ""
            continue

        if section == "about" and not result["about"] and len(line) > 10:
            result["about"] = line
            section = ""
        elif section == "experience" and len(line) > 5 and line not in skip:
            if not any(kw in line for kw in ["·", "mos", "yrs", "Present"]):
                if len(result["experience"]) < 10:
                    result["experience"].append(line)
        elif section == "education" and len(line) > 5 and line not in skip:
            if not any(kw in line for kw in ["·", "Grade", "GPA"]):
                if len(result["education"]) < 5:
                    result["education"].append(line)

    return result


async def oneshot_search(query, limit=5):
    """One-shot search: launch browser, search, return results, close."""
    from playwright.async_api import async_playwright
    import urllib.parse

    profile_dir = PROFILE_DIR
    async with async_playwright() as p:
        context = await p.chromium.launch_persistent_context(
            profile_dir, headless=False,
            viewport={"width": 1280, "height": 900},
        )
        page = context.pages[0] if context.pages else await context.new_page()
        await page.goto("https://www.linkedin.com/feed/", timeout=30000)
        await page.wait_for_timeout(3000)

        if "login" in page.url or "authwall" in page.url:
            print(json.dumps({"error": "not_logged_in", "message": "Run: python linkedin_live.py login"}), flush=True)
            await context.close()
            return

        search_url = f"https://www.linkedin.com/search/results/people/?keywords={urllib.parse.quote(query)}"
        await page.goto(search_url, wait_until="domcontentloaded", timeout=30000)
        await page.wait_for_timeout(4000)
        await page.evaluate("window.scrollBy(0, 1500)")
        await page.wait_for_timeout(1500)

        results = await page.evaluate("""() => {
            const links = document.querySelectorAll('a[href*="/in/"]');
            const seen = new Set(); const out = [];
            for (const a of links) {
                const href = a.href.split('?')[0]; const text = a.innerText.trim();
                if (!text || text.length < 3 || text.length > 80 || seen.has(href)) continue;
                if (href.includes('/in/AC') || text.includes('LinkedIn')) continue;
                seen.add(href);
                let p = a.closest('li'); let card = p ? p.innerText.substring(0, 500) : '';
                out.push({name: text, url: href, card: card});
            }
            return out;
        }""")

        final = []
        for r in results[:limit]:
            card_lines = [l.strip() for l in r["card"].split("\n") if l.strip()]
            headline = ""
            for i, l in enumerate(card_lines):
                if l == r["name"] and i + 1 < len(card_lines):
                    for j in range(i + 1, min(i + 5, len(card_lines))):
                        c = card_lines[j]
                        if c not in ("Connect", "Message", "Follow", "Pending", "3rd+", "2nd", "1st", "3rd", "• 3rd+", "• 2nd", "• 1st") and len(c) > 5 and not c.startswith("•"):
                            headline = c
                            break
                    break
            final.append({"name": r["name"], "headline": headline, "linkedin_url": r["url"]})

        print(json.dumps({"query": query, "people": final}, ensure_ascii=False), flush=True)
        await context.close()


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "login":
        asyncio.run(main())
    elif len(sys.argv) > 1 and sys.argv[1] == "search":
        query = sys.argv[2] if len(sys.argv) > 2 else ""
        limit = int(sys.argv[3]) if len(sys.argv) > 3 else 5
        asyncio.run(oneshot_search(query, limit))
    elif len(sys.argv) > 1 and sys.argv[1] == "profile":
        # One-shot profile fetch
        async def oneshot_profile():
            from playwright.async_api import async_playwright
            profile_dir = PROFILE_DIR
            async with async_playwright() as p:
                context = await p.chromium.launch_persistent_context(
                    profile_dir, headless=False,
                    viewport={"width": 1280, "height": 900},
                )
                page = context.pages[0] if context.pages else await context.new_page()
                await page.goto("https://www.linkedin.com/feed/", timeout=30000)
                await page.wait_for_timeout(3000)
                if "login" in page.url or "authwall" in page.url:
                    print(json.dumps({"error": "not_logged_in"}), flush=True)
                    await context.close()
                    return
                result = await fetch_profile(page, sys.argv[2])
                print(json.dumps(result, ensure_ascii=False), flush=True)
                await context.close()
        asyncio.run(oneshot_profile())
    else:
        asyncio.run(main())
