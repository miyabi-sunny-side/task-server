#!/usr/bin/env python3
"""Check the UI and its referenced assets against an isolated running server."""

import json
import sys
from html.parser import HTMLParser
from urllib.error import HTTPError
from urllib.request import Request, urlopen


class Assets(HTMLParser):
    def __init__(self):
        super().__init__()
        self.paths = {}

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "script" and "src" in attrs:
            self.paths[attrs["src"]] = "text/javascript"
        if tag == "link":
            mime = {
                "stylesheet": "text/css",
                "manifest": "application/manifest+json",
                "icon": "image/svg+xml",
                "apple-touch-icon": "image/png",
            }.get(attrs.get("rel"))
            if mime:
                self.paths[attrs["href"]] = mime


base = sys.argv[1].rstrip("/")


def fetch(path, method="GET", status=200):
    try:
        response = urlopen(Request(base + path, method=method), timeout=10)
    except HTTPError as error:
        response = error
    with response:
        assert response.status == status, (path, response.status, status)
        return response.headers.get_content_type(), response.read()


mime, html = fetch("/")
assert mime == "text/html" and html.lower().startswith(b"<!doctype html>")
assets = Assets()
assets.feed(html.decode())
assert {
    "text/javascript", "text/css", "application/manifest+json", "image/svg+xml", "image/png"
} <= set(assets.paths.values())
for path, expected_mime in assets.paths.items():
    actual_mime, body = fetch(path)
    assert actual_mime == expected_mime and body, (path, actual_mime)
    head_mime, head_body = fetch(path, method="HEAD")
    assert head_mime == expected_mime and not head_body, path
    if expected_mime == "application/manifest+json":
        for icon in json.loads(body)["icons"]:
            icon_mime, icon_body = fetch(icon["src"])
            assert icon_mime == icon["type"] and icon_body.startswith(b"\x89PNG\r\n\x1a\n")
for path in ["/tasks/example", "/closed", "/products", "/projects/example"]:
    assert fetch(path) == ("text/html", html), path
for path in ["/api", "/api/", "/api/missing"]:
    assert fetch(path, status=404)[0] == "application/json", path
fetch("/", method="POST", status=405)
print("UI smoke passed: HTML, JS/CSS, install assets, HEAD, deep links and API boundary")
