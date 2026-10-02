from pathlib import Path

_ROOT = Path(__file__).parent.parent
_VENDOR_HTMX_SRC = _ROOT / "vendor/htmx/src/htmx.js"

HTMX_SCRIPT_TAG = '<script src="http://localhost/vendor/htmx.js"></script>'

HTMX_RESOURCE_MOUNT: dict[str, str] = {"http://localhost/vendor/": str(_VENDOR_HTMX_SRC.parent)}
