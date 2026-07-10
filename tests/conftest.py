from pathlib import Path

import pytest
import pytest_asyncio
from hypothesis import Phase, settings

from htmxclient.runtime import VirtualServer, build_runtime, get_snapshot_builder

settings.register_profile(
    "noshrink",
    phases=[Phase.explicit, Phase.reuse, Phase.generate, Phase.target],
)


_ROOT = Path(__file__).parent.parent
_VENDOR_HTMX_TEST = _ROOT / "vendor/htmx/test"
_VENDOR_HTMX_SRC = _ROOT / "vendor/htmx/src/htmx.js"
_CHAI_JS = _ROOT / "node_modules/chai/chai.js"
_RUNNER_JS = Path(__file__).parent / "runner.js"
_FETCH_MOCK_BRIDGE_JS = Path(__file__).parent / "htmx_fetch_mock_bridge.js"


def htmx_virtual_server(base_url: str = "http://localhost/") -> VirtualServer:
    """Mount the vendored htmx source so `<script src="{base_url}vendor/htmx.js">` resolves."""
    return {"url": f"{base_url}vendor/", "directory": str(_VENDOR_HTMX_SRC.parent)}


def htmx_script_tag(base_url: str = "http://localhost/") -> str:
    return f'<script src="{base_url}vendor/htmx.js"></script>'


@pytest.fixture(scope="session")
def browser_snapshot() -> bytes:
    builder = get_snapshot_builder()
    builder.execute_script(
        "chai",
        f"""{_CHAI_JS.read_text()}
            globalThis.assert = globalThis.chai.assert;
            globalThis.should = globalThis.chai.should();""",
    )
    builder.execute_script("fetch-mock-bridge", _FETCH_MOCK_BRIDGE_JS.read_text())
    builder.execute_script("runner", _RUNNER_JS.read_text())
    return builder.build()


@pytest_asyncio.fixture
async def runtime(browser_snapshot):
    r = await build_runtime(snapshot=browser_snapshot, virtual_servers=[htmx_virtual_server()])
    try:
        yield r
    finally:
        r.close()
