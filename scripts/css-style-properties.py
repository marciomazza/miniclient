#!/usr/bin/env python3
"""Regenerates the jsdom fork's CSSStyleProperties attributes from stylo's property table.

Keeps the properties whose pref is unset or one src/css.rs enables, so `element.style` exposes
exactly what the style engine parses. Writes the attribute-to-property map that the impl and
js/polyfills/npm-jsdom-css-style-properties.js read. Needs a prior `cargo build`.
"""

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORK = ROOT / "vendor/jsdom"
CSS_DIR = FORK / "lib/jsdom/living/css"


def idl_attribute(prop: str, drop_leading_dash: bool) -> str:
    """The CSSOM "CSS property to IDL attribute" algorithm."""
    if drop_leading_dash and prop.startswith("-"):
        prop = prop[1:]
    return re.sub(r"-(.)", lambda m: m[1].upper(), prop)


def attributes(prop: str) -> list[str]:
    names = [idl_attribute(prop, False)]
    if prop.startswith("-webkit-"):
        names.append(idl_attribute(prop, True))
    if "-" in prop:
        names.append(prop)
    return names


def main() -> None:
    # The recipe's `cargo build` writes under target/debug; older builds leave stale tables.
    tables = ROOT.glob("target/debug/build/stylo/*/out/css-properties.json")
    table = max(tables, key=lambda p: p.stat().st_mtime)
    data = json.loads(table.read_text())
    enabled = set(re.findall(r'set_pref!\("([^"]+)", true\)', (ROOT / "src/css.rs").read_text()))
    props = sorted(
        name
        for kind in ("longhands", "shorthands")
        for name, info in data[kind].items()
        # Chrome exposes no -moz- aliases, and pages feature-detect on them.
        if (info["pref"] is None or info["pref"] in enabled) and not name.startswith("-moz-")
    )
    names = {"cssFloat": "float"} | {attr: prop for prop in props for attr in attributes(prop)}

    (CSS_DIR / "style-property-names.json").write_text(json.dumps(names, indent=1) + "\n")


if __name__ == "__main__":
    main()
