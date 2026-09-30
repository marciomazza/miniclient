from collections.abc import AsyncIterator
from pathlib import Path

import pytest_asyncio

from miniclient.runtime import Page, open_runtime

_ROOT = Path(__file__).parent.parent
_VENDOR_HTMX_SRC = _ROOT / "vendor/htmx/src/htmx.js"

HTMX_SCRIPT_TAG = '<script src="http://localhost/vendor/htmx.js"></script>'

HTMX_RESOURCE_MOUNT: dict[str, str] = {"http://localhost/vendor/": str(_VENDOR_HTMX_SRC.parent)}


@pytest_asyncio.fixture
async def runtime() -> AsyncIterator[Page]:
    async with open_runtime() as r:
        yield r
