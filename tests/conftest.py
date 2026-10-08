from pathlib import Path

import pytest

from miniclient.browser import Browser

_ROOT = Path(__file__).parent.parent
_VENDOR_HTMX_SRC = _ROOT / "vendor/htmx/src/htmx.js"

HTMX_SCRIPT_TAG = '<script src="http://localhost/vendor/htmx.js"></script>'

HTMX_RESOURCE_MOUNT: dict[str, str] = {"http://localhost/vendor/": str(_VENDOR_HTMX_SRC.parent)}


@pytest.fixture(scope="session")
def browser() -> Browser:
    return Browser()


@pytest.fixture(scope="session")
def htmx_browser() -> Browser:
    """Pages from it can reach the vendored htmx.js. Each load()/goto() is a real navigation,
    so htmx must be (re-)loaded per page via HTMX_SCRIPT_TAG, same as a real page."""
    return Browser(mounts={url: Path(directory) for url, directory in HTMX_RESOURCE_MOUNT.items()})
