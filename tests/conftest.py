from collections.abc import AsyncIterator
from pathlib import Path

import pytest
import pytest_asyncio
from hypothesis import Phase, settings
from jsrun import Runtime

from miniclient.runtime import VirtualServer, get_snapshot_builder, open_runtime

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

HTMX_BASE_HTML = """\
    <!DOCTYPE html>
    <html>
      <head>
        <script src="http://localhost/vendor/htmx.js"></script>
      </head>
      <body>
        <div id="test-playground"></div>
      </body>
    </html>
"""


HTMX_VIRTUAL_SERVER: VirtualServer = {
    "url": "http://localhost/vendor/",
    "directory": str(_VENDOR_HTMX_SRC.parent),
}


@pytest.fixture(scope="session")
def snapshot() -> bytes:
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
async def runtime(snapshot: bytes) -> AsyncIterator[Runtime]:
    async with open_runtime(snapshot=snapshot, virtual_servers=[HTMX_VIRTUAL_SERVER]) as r:
        yield r
