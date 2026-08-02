from collections.abc import AsyncIterator
from pathlib import Path

import pytest_asyncio
from jsrun import Runtime

from miniclient.runtime import VirtualServer, open_runtime

_ROOT = Path(__file__).parent.parent
_VENDOR_HTMX_SRC = _ROOT / "vendor/htmx/src/htmx.js"

HTMX_SCRIPT_TAG = '<script src="http://localhost/vendor/htmx.js"></script>'

HTMX_BASE_HTML = f"""\
    <!DOCTYPE html>
    <html>
      <head>
        {HTMX_SCRIPT_TAG}
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


@pytest_asyncio.fixture
async def runtime() -> AsyncIterator[Runtime]:
    async with open_runtime() as r:
        yield r
