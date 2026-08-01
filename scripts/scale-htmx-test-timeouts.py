#!/usr/bin/env python3
"""Divides every real-time constant in a vendored htmx test file by up to MAX_DIVISOR.

Vendored htmx tests wait on real setTimeout-backed delays (htmx.timeout() calls, raw
setTimeout(), hx-trigger delay modifiers, and hx-ws's reconnect/TTL config values),
which makes some of them dominate test_htmx.py's runtime (e.g. hx-ws.js was ~10s).
Scaling every constant by the same factor preserves their relative ratios (so
ordering-dependent assertions still hold) while cutting wall time proportionally.

The divisor is capped at the file's smallest nonzero timing constant, so nothing ever
floors to 0 — htmx.timeout(0) and setTimeout(fn, 0) skip the real timer entirely (see
htmx.js's timeout()), which silently breaks tests relying on a real macrotask tick
(e.g. MutationObserver flush timing) rather than just a microtask. If that cap is 1,
the file's numbers are already as low as they can go — it's left untouched.

Run again after any vendor/htmx refresh (scripts/setup-vendor-htmx.sh re-clones from
upstream and wipes in-place edits to vendored files).

Usage:
    scripts/scale-htmx-test-timeouts.py <path-to-js-file> [max-divisor]
"""

import re
import sys
from pathlib import Path

DEFAULT_MAX_DIVISOR = 10

# Each branch's lookbehind/lookahead is fixed-width (Python's re requirement), so the
# match is just the digits — no need to capture and re-emit the surrounding syntax.
_TIMING_NUMBER = re.compile(
    r"(?<=htmx\.timeout\()\d+(?=\))"
    r"|(?<=reconnectDelay:\ )\d+"
    r"|(?<=reconnectMaxDelay:\ )\d+"
    r"|(?<=pendingRequestTTL:\ )\d+"
    r"|(?<=ws\.reconnectDelay:)\d+"
    r'|(?<="reconnectDelay":\ )\d+'
    r"|(?<=delay:)\d+(?=ms)"
)

# setTimeout's first argument is a callback of arbitrary length, so its delay sits
# behind a variable-width prefix — can't be reached with a fixed-width lookbehind.
_SET_TIMEOUT = re.compile(r"(setTimeout\(.*?,\s*)(\d+)(\))", re.DOTALL)


def _find_values(text: str) -> list[int]:
    values = [int(m.group(0)) for m in _TIMING_NUMBER.finditer(text)]
    values += [int(m.group(2)) for m in _SET_TIMEOUT.finditer(text)]
    return values


def scale_file(target: Path, max_divisor: int) -> None:
    text = target.read_text()
    nonzero = [v for v in _find_values(text) if v > 0]
    divisor = min(max_divisor, min(nonzero)) if nonzero else max_divisor

    if divisor <= 1:
        return

    text = _TIMING_NUMBER.sub(lambda m: str(int(m.group(0)) // divisor), text)
    text = _SET_TIMEOUT.sub(lambda m: f"{m.group(1)}{int(m.group(2)) // divisor}{m.group(3)}", text)
    target.write_text(text)
    print(f"Scaled timing constants in {target} by /{divisor}")  # ruff: ignore[print]


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit(f"usage: {sys.argv[0]} <path-to-js-file> [max-divisor]")
    target = Path(sys.argv[1])
    max_divisor = int(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_MAX_DIVISOR
    scale_file(target, max_divisor)


if __name__ == "__main__":
    main()
