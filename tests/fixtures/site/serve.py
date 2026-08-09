#!/usr/bin/env python3
"""Serve the fixture site.

Static files cover most of it, but the cases that actually break crawlers are behaviours, not
documents: redirect chains, rate limiting, timeouts, and an infinitely deep link graph. Those
need a server, so this is a small one rather than a directory listing.

Python's standard library only — this runs in CI and on a laptop, and a fixture server that needs
its own dependency tree is a fixture server nobody starts.

    ./tests/fixtures/site/serve.py [--port 8099]
"""

import argparse
import http.server
import os
import socketserver
import sys
import time
from pathlib import Path

ROOT = Path(__file__).parent


ROBOTS_HITS = {"n": 0}


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def log_message(self, fmt, *args):
        # One line per request, on stderr, without the timestamp noise. Tests read this to
        # assert that a disallowed path was never requested.
        sys.stderr.write("fixture %s %s\n" % (self.command, self.path))

    def do_HEAD(self):
        # The dynamic paths must answer HEAD the same way they answer GET. A fetcher that probes
        # with HEAD before deciding to download would otherwise see a 404 where the real site
        # sends a 429, and the politeness logic would never be exercised.
        self.do_GET(body=False)

    def do_GET(self, body=True):
        path = self.path.split("?", 1)[0]

        # --- redirect chain -------------------------------------------------------------
        # Four hops. Real news sites chain http → https → www → canonical routinely, and each
        # hop has to be revalidated against the SSRF guard rather than followed blindly.
        if path.startswith("/redirect/"):
            tail = path[len("/redirect/"):].strip("/")
            if tail == "loop":
                # A cycle. The only correct outcome is to give up, not to follow it forever.
                self.send_response(302)
                self.send_header("Location", "/redirect/loop")
                self.end_headers()
                return
            try:
                hop = int(tail)
            except ValueError:
                self.send_error(404)
                return
            if hop >= 4:
                self.send_response(302)
                self.send_header("Location", "/articles/normal.html")
            else:
                self.send_response(301 if hop % 2 else 302)
                self.send_header("Location", f"/redirect/{hop + 1}")
            self.end_headers()
            return

        # --- robots.txt request counter -------------------------------------------------
        # So a test can assert what the *site* saw, rather than what our own cache thinks. An
        # in-process cache would satisfy a state-based check while still sending two requests,
        # and the request count is the thing a site operator actually reacts to.
        if path == "/robots-count":
            data = str(ROBOTS_HITS["n"]).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if body:
                self.wfile.write(data)
            return
        if path == "/robots-count/reset":
            ROBOTS_HITS["n"] = 0
            self.send_response(204)
            self.end_headers()
            return

        # --- robots.txt failure modes ---------------------------------------------------
        # An unreachable robots.txt is not permission. These routes let the crawler's handling of
        # each status be asserted rather than assumed, because the failure is silent: a crawler
        # that reads a 403 as "no restrictions" behaves impeccably in testing and crawls a site
        # that refused it in production.
        if path.startswith("/robots-status/"):
            try:
                code = int(path.rsplit("/", 1)[-1])
            except ValueError:
                self.send_error(404)
                return
            self.send_response(code)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        # --- X-Robots-Tag ---------------------------------------------------------------
        # A page that allows crawling and refuses indexing. The two are separate permissions, and
        # this header is the only way a document without a <head> — a PDF, an image, a JSON feed —
        # can express the second.
        #
        # Emitted as two header lines rather than one comma-joined value, because that is what
        # real servers do and a parser that reads only the first sees half the directive.
        if path == "/noindex-header.html":
            data = (ROOT / "articles" / "normal.html").read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("X-Robots-Tag", "noindex")
            self.send_header("X-Robots-Tag", "nofollow")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if body:
                self.wfile.write(data)
            return

        # Addressed to a different crawler. Obeying it would drop a document the site was happy
        # for us to keep.
        if path == "/noindex-other-agent.html":
            data = (ROOT / "articles" / "normal.html").read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("X-Robots-Tag", "googlebot: noindex")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if body:
                self.wfile.write(data)
            return

        # --- robots.txt -----------------------------------------------------------------
        # Served dynamically so the Sitemap directive names the host and port we are actually
        # listening on. A hardcoded absolute URL in the static file points somewhere else the
        # moment the port changes, and sitemap discovery then silently finds nothing.
        if path == "/robots.txt":
            ROBOTS_HITS["n"] += 1
            text = (ROOT / "robots.txt").read_text()
            host = self.headers.get("Host", f"127.0.0.1:{self.server.server_address[1]}")
            text = text.replace("http://localhost:8099", f"http://{host}")
            data = text.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if body:
                self.wfile.write(data)
            return

        # --- rate limiting --------------------------------------------------------------
        if path == "/429":
            self.send_response(429)
            self.send_header("Retry-After", "2")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        if path == "/500":
            self.send_error(500, "deliberate")
            return

        # --- slow response --------------------------------------------------------------
        # Five seconds, longer than any per-request budget. Tests that this aborts rather than
        # holding a connection open until the whole crawl stalls behind it.
        if path == "/slow":
            time.sleep(5)
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            if body:
                self.wfile.write(b"<html><body><p>eventually</p></body></html>")
            return

        # --- crawler trap ---------------------------------------------------------------
        # Every URL under /trap/ exists and links to three more, forever. Disallowed in
        # robots.txt, so a correct crawler never gets here; the depth limit is the backstop for
        # sites that do not warn you.
        if path.startswith("/trap/") and path != "/trap/":
            depth = path.strip("/").count("/")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            links = "".join(
                f'<a href="{path.rstrip("/")}/{i}/">deeper</a> ' for i in range(3)
            )
            if body:
                self.wfile.write(
                    f"<html><body><h1>depth {depth}</h1>{links}</body></html>".encode()
                )
            return

        # --- static -----------------------------------------------------------------------
        if body:
            super().do_GET()
        else:
            super().do_HEAD()

    def end_headers(self):
        # windows-1256 is declared in the document's meta tag only. Sending a charset here too
        # would make the hard case easy and hide the bug this fixture exists to catch.
        super().end_headers()

    def guess_type(self, path):
        if str(path).endswith("windows-1256.html"):
            return "text/html"
        return super().guess_type(path)


class Server(socketserver.ThreadingTCPServer):
    # Threaded so /slow does not block every other request, and reusable so restarting the
    # fixture during a test run does not wait out TIME_WAIT.
    allow_reuse_address = True
    daemon_threads = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=int(os.environ.get("FIXTURE_PORT", 8099)))
    args = parser.parse_args()

    # `--port 0` asks the OS for a free one and prints it on stdout. Tests use this: a fixed
    # port collides with a server left over from an earlier run, and the symptom is a test
    # talking to stale code rather than an obvious "address in use".
    with Server(("127.0.0.1", args.port), Handler) as httpd:
        port = httpd.server_address[1]
        print(port, flush=True)
        print(f"fixture site on http://127.0.0.1:{port} (Ctrl-C to stop)", file=sys.stderr)
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
