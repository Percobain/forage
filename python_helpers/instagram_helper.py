#!/usr/bin/env python3
"""
Instagram profile fetcher using instagrapi.
Called as a subprocess from the Rust MCP server.

Usage:
    python instagram_helper.py fetch_profile <username>

Output: JSON to stdout

Requirements:
    pip install instagrapi

Setup:
    Login once with: python instagram_helper.py login <username> <password>
    Session is saved to ~/.forage/cookies/instagram_session.json
"""

import sys
import json
import os
from pathlib import Path

SESSION_PATH = Path.home() / ".research-mcp" / "cookies" / "instagram_session.json"


def get_client():
    try:
        from instagrapi import Client
    except ImportError:
        print(json.dumps({"error": "instagrapi not installed. Run: pip install instagrapi"}))
        sys.exit(1)

    cl = Client()
    if SESSION_PATH.exists():
        cl.load_settings(str(SESSION_PATH))
        cl.login_by_sessionid(cl.settings.get("sessionid", ""))
    else:
        print(json.dumps({"error": f"No Instagram session found at {SESSION_PATH}. Run: python instagram_helper.py login <username> <password>"}))
        sys.exit(1)

    return cl


def fetch_profile(username):
    cl = get_client()
    try:
        user = cl.user_info_by_username(username)
        result = {
            "username": user.username,
            "full_name": user.full_name,
            "biography": user.biography,
            "followers": user.follower_count,
            "following": user.following_count,
            "posts_count": user.media_count,
            "is_verified": user.is_verified,
            "external_url": user.external_url,
        }
        print(json.dumps(result))
    except Exception as e:
        print(json.dumps({"error": str(e)}))
        sys.exit(1)


def login(username, password):
    try:
        from instagrapi import Client
    except ImportError:
        print(json.dumps({"error": "instagrapi not installed. Run: pip install instagrapi"}))
        sys.exit(1)

    cl = Client()
    cl.login(username, password)
    SESSION_PATH.parent.mkdir(parents=True, exist_ok=True)
    cl.dump_settings(str(SESSION_PATH))
    print(json.dumps({"status": "ok", "session_path": str(SESSION_PATH)}))


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: instagram_helper.py <command> [args...]")
        print("Commands: fetch_profile <username>, login <username> <password>")
        sys.exit(1)

    command = sys.argv[1]
    if command == "fetch_profile" and len(sys.argv) >= 3:
        fetch_profile(sys.argv[2])
    elif command == "login" and len(sys.argv) >= 4:
        login(sys.argv[2], sys.argv[3])
    else:
        print(f"Unknown command or missing args: {command}")
        sys.exit(1)
