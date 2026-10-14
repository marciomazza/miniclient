"""Records Chromium's computed values over the CSS corpus into tests/css_golden/chromium.json.

    uv run python scripts/record-chromium-css.py

Run by hand when the corpus or Chromium changes; tests/stylo_css.rs compares mini against it.
Each element stores only the values that differ from the property's corpus-wide default.
"""

import asyncio
import collections
import json
import re
from pathlib import Path

from playwright.async_api import Error, async_playwright

ROOT = Path(__file__).parent.parent
BASE = ROOT / "vendor" / "jsdom" / "test" / "web-platform-tests"
DIRS = [
    "to-upstream/css",
    "tests/css/css-cascade",
    "tests/css/css-variables",
    "tests/css/css-nesting",
    "tests/css/css-conditional",
    "tests/css/cssom",
]
OUT = ROOT / "tests" / "css_golden" / "chromium.json"
# mini's device: jsdom's window size, light colour scheme.
VIEWPORT = {"width": 1024, "height": 768}
COLOR_SCHEME = "light"
FILE_TIMEOUT = 20

SCRIPT = re.compile(r"(?is)<script\b.*?</script\s*>")

COMPUTE_JS = """(markup) => {
    document.documentElement.innerHTML = markup;
    return [...document.querySelectorAll("*")].map((el) => {
        const cs = getComputedStyle(el);
        const props = [...cs].filter((p) => !p.startsWith("--"));
        return [el.localName, props.map((p) => [p, cs.getPropertyValue(p)])];
    });
}"""


async def record():
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()

        async def blank_page():
            page = await browser.new_page(viewport=VIEWPORT, color_scheme=COLOR_SCHEME)
            await page.set_content("<!DOCTYPE html>")
            return page

        page = await blank_page()
        files, skipped = {}, {}
        for directory in DIRS:
            for path in sorted((BASE / directory).rglob("*.html")):
                rel = path.relative_to(BASE).as_posix()
                markup = SCRIPT.sub("", path.read_text(errors="replace"))
                # A renderer crash never settles evaluate(), so the timeout is what unblocks it.
                try:
                    files[rel] = await asyncio.wait_for(
                        page.evaluate(COMPUTE_JS, markup), FILE_TIMEOUT
                    )
                except (TimeoutError, Error) as error:
                    skipped[rel] = type(error).__name__
                    await page.close()
                    page = await blank_page()
        version = browser.version
        await browser.close()
    return version, files, skipped


def encode_and_write(version, files, skipped):
    first_element = next(element for elements in files.values() for element in elements)
    properties = [prop for prop, _ in first_element[1]]
    counts = collections.defaultdict(collections.Counter)
    for elements in files.values():
        for _, values in elements:
            assert [prop for prop, _ in values] == properties, (
                "Chromium listed a different property set"
            )
            for prop, value in values:
                counts[prop][value] += 1
    defaults = {prop: counts[prop].most_common(1)[0][0] for prop in properties}
    head = {
        "chromium": version,
        "viewport": [VIEWPORT["width"], VIEWPORT["height"]],
        "colorScheme": COLOR_SCHEME,
        "properties": properties,
        "defaults": defaults,
        "skipped": skipped,
    }
    # One file per line keeps re-recordings reviewable as diffs.
    lines = [
        f"{json.dumps(rel)}: "
        + json.dumps([
            [name, {prop: value for prop, value in values if value != defaults[prop]}]
            for name, values in elements
        ])
        for rel, elements in files.items()
    ]
    text = json.dumps(head, indent=1)[:-2] + ',\n "files": {\n' + ",\n".join(lines) + "\n}}\n"
    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text(text)
    print(f"Chromium {version}: {len(files)} files, {len(skipped)} skipped -> {OUT}")


if __name__ == "__main__":
    encode_and_write(*asyncio.run(record()))
