import asyncio
from pathlib import Path

import pytest
import pytest_asyncio
from jsrun import SnapshotBuilder

from htmxclient.browser import Browser, _populate_builder, build_browser

_ROOT = Path(__file__).parent.parent
_HTMX_TEST = _ROOT / "vendor/htmx/test"
_CHAI_JS = _ROOT / "node_modules/chai/chai.js"
_RUNNER_JS = Path(__file__).parent / "runner.js"
_FETCH_MOCK_JS = _HTMX_TEST / "lib/fetch-mock.js"


@pytest.fixture(scope="session")
def browser_snapshot() -> bytes:
    builder = SnapshotBuilder()
    _populate_builder(builder)
    builder.execute_script(
        "chai",
        _CHAI_JS.read_text()
        + "\nglobalThis.assert = globalThis.chai.assert;"
        + "\nglobalThis.should = globalThis.chai.should();",
    )
    builder.execute_script("fetch-mock", _FETCH_MOCK_JS.read_text())
    builder.execute_script("runner", _RUNNER_JS.read_text())
    return builder.build()


@pytest.fixture()
def browser(browser_snapshot):
    async def _build():
        return await build_browser(snapshot=browser_snapshot)

    with asyncio.run(_build()) as r:
        yield r


@pytest_asyncio.fixture
async def browser_async(browser_snapshot):
    r = await build_browser(snapshot=browser_snapshot)
    try:
        yield r
    finally:
        r.close()


@pytest_asyncio.fixture
async def app_browser(browser_snapshot):
    r = await build_browser("http://app.example.com/", snapshot=browser_snapshot)
    b = Browser(r)
    try:
        yield b
    finally:
        b.close()
