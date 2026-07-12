---
icon: lucide/rocket
---

# miniclient

A python HTTP client for testing [htmx](https://htmx.org)-powered server applications.

It's a lightweight client that simulates browser interactions running htmx and
[happy-dom](https://github.com/capricorn86/happy-dom) inside a V8 JavaScript runtime.

This way requests, DOM swaps and event handling all go through htmx's actual code.
HTTP is done with [httpx2](https://httpx2.pydantic.dev/). V8 is embedded by
[jsrun](https://imfing.github.io/jsrun), which enables running JavaScript code directly from python.

## Why?

Testing against a real browser is simply too slow. And mostly unnecessary.

## Quick tour

The API simulates a browser's general interaction with the `Browser` and `Element` classes:

```python
from miniclient.browser import Browser

async with await Browser.create() as browser:
    await browser.goto("http://localhost:8000/")
    await browser.find("#load-more").click()
    print(browser.find("#results").text())
```

Filling in a form and submitting it works the same way, through `fill()` and `requestSubmit()`:

```python
from miniclient.browser import Browser

async with await Browser.create() as browser:
    await browser.goto("http://localhost:8000/signup")
    browser.find("input[name=name]").fill("Ada")
    browser.find("input[name=email]").fill("ada@example.com")
    await browser.find("form").requestSubmit()
    print(browser.find("#result").text())
```

For anything not covered by `Browser` / `Element`, you can run arbitrary JavaScript directly
through `browser.runtime`:

```python
from miniclient.browser import Browser

async with await Browser.create() as browser:
    await browser.load("<h1 id='greeting'>Hello</h1>")
    print(browser.runtime.eval("document.getElementById('greeting').textContent"))
    browser.runtime.eval("document.getElementById('greeting').textContent = 'Hi!'")
    print(browser.find("#greeting").text())
```

See [Getting started](getting-started.md) to install, [Usage](usage.md) for the full API guide, and
[Testing](testing.md) for how the test suite (including the browser crosscheck) works.
