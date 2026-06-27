from pathlib import Path

import pytest
import pytest_asyncio
from hypothesis import Phase, settings
from jsrun import SnapshotBuilder

from htmxclient.runtime import _populate_builder, build_runtime

settings.register_profile(
    "noshrink",
    phases=[Phase.explicit, Phase.reuse, Phase.generate, Phase.target],
)


_ROOT = Path(__file__).parent.parent
_HTMX_TEST = _ROOT / "vendor/htmx/test"
_CHAI_JS = _ROOT / "node_modules/chai/chai.js"
_RUNNER_JS = Path(__file__).parent / "runner.js"
_FETCH_MOCK_BRIDGE_JS = Path(__file__).parent / "htmx_fetch_mock_bridge.js"


@pytest.fixture(scope="session")
def browser_snapshot() -> bytes:
    builder = SnapshotBuilder()
    _populate_builder(builder)
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
    r = await build_runtime(snapshot=browser_snapshot)
    try:
        yield r
    finally:
        r.close()
