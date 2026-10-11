"""Runs the WPT fetch/CORS window tests inside miniclient and compares with expected.json.

    uv run python tests/wpt/run.py            # fail on regressions
    uv run python tests/wpt/run.py --update   # record what passes in 3 runs as the new baseline
    uv run python tests/wpt/run.py --extended # the larger file set (files-extended.txt)

Needs vendor/jsdom with its test/web-platform-tests submodule checked out, and a built extension
(`just develop`).
"""

import asyncio
import json
import os
import socket
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import httpx2 as httpx

from miniclient.browser import AsyncPage, Browser

HERE = Path(__file__).parent
WPT = HERE.parent.parent / "vendor" / "jsdom" / "test" / "web-platform-tests" / "tests"
FILE_TIMEOUT = 90

# Replaces WPT's reporter so the page leaves its results in `window.__wpt`.
REPORTER = (HERE / "reporter.js").read_bytes()

# wptserve has no TLS here: https origins are served by the plain http ports.
PORTS = {8000: 8000, 8001: 8001, 8443: 8000, 8444: 8001}

WAIT = """
new Promise((resolve) => {
  const timer = setInterval(() => {
    if (window.__wpt) {
      clearInterval(timer);
      resolve(window.__wpt);
    }
  }, 20);
})
"""


class Proxy(httpx.AsyncBaseTransport):
    def __init__(self):
        self.inner = httpx.AsyncHTTPTransport()

    async def handle_async_request(self, request):
        if request.url.path == "/resources/testharnessreport.js":
            return httpx.Response(
                200, headers={"content-type": "text/javascript"}, content=REPORTER
            )
        port = PORTS.get(request.url.port or 0)
        if request.url.scheme not in ("http", "https") or port is None:
            raise httpx.ConnectError(f"unmapped {request.url}")
        # A rewritten copy: mutating request.url would leak 127.0.0.1 into Response.url and
        # into redirect resolution.
        url = request.url.copy_with(scheme="http", host="127.0.0.1", port=port)
        fwd = httpx.Request(request.method, url, headers=request.headers, stream=request.stream)
        resp = await self.inner.handle_async_request(fwd)
        resp.request = request
        return resp


async def run_page(path):
    page = AsyncPage(Browser(httpx_transport=Proxy(), preboot=False))
    try:
        await asyncio.wait_for(page.goto("http://localhost:8000" + path), FILE_TIMEOUT)
        raw = await asyncio.wait_for(page.eval(WAIT), FILE_TIMEOUT)
        return json.loads(raw)
    except Exception as e:
        return {"status": "ERROR", "message": f"{type(e).__name__}: {e}"[:300], "tests": []}
    finally:
        try:
            await asyncio.wait_for(page.aclose(), 10)
        except Exception:
            pass


def run_file(path):
    return path, asyncio.run(run_page(path))


def serve():
    config = HERE / "wpt-config.json"
    server = subprocess.Popen(
        [WPT / "wpt", "serve", "--no-h2", "--config", config],
        cwd=WPT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(300):
        try:
            socket.create_connection(("127.0.0.1", 8001), timeout=1).close()
            return server
        except OSError:
            time.sleep(0.5)
    server.kill()
    sys.exit("wpt serve did not come up")


def passing(result):
    return sorted(t["name"] for t in result["tests"] if t["status"] == 0)


def main():
    update = "--update" in sys.argv
    suffix = "-extended" if "--extended" in sys.argv else ""
    expected_file = HERE / f"expected{suffix}.json"
    expected = {} if update else json.loads(expected_file.read_text())
    paths = HERE.joinpath(f"files{suffix}.txt").read_text().split()
    server = serve()
    try:
        # The baseline keeps only what passes in every one of three runs, so flaky subtests
        # are not tracked.
        with ProcessPoolExecutor(int(os.environ.get("JOBS", "8"))) as pool:
            runs = [dict(pool.map(run_file, paths)) for _ in range(3 if update else 1)]
        results = runs[0]
        now = {p: sorted(set.intersection(*(set(passing(r[p])) for r in runs))) for p in results}
        if not update:
            # Some files race against the real server (e.g. a 304 whose body leaks into the next
            # response on a reused connection): a subtest is lost only if it fails every retry.
            for p, names in expected.items():
                for _ in range(2):
                    if set(names) <= set(now[p]):
                        break
                    now[p] = sorted(set(now[p]) | set(passing(run_file(p)[1])))
    finally:
        server.terminate()

    total = sum(len(r["tests"]) for r in results.values())
    print(f"{sum(map(len, now.values()))} / {total} subtests pass in {len(paths)} files")
    if update:
        expected_file.write_text(json.dumps(now, indent=1, sort_keys=True) + "\n")
        return
    regressions = [
        f"{p}: {name}" for p, names in expected.items() for name in names if name not in now[p]
    ]
    gains = sum(len(set(now[p]) - set(names)) for p, names in expected.items())
    print(f"{gains} newly passing; run with --update to record them")
    if regressions:
        print(f"{len(regressions)} REGRESSIONS:")
        print("\n".join(regressions))
        sys.exit(1)


if __name__ == "__main__":
    main()
