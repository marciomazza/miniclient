---
icon: lucide/rocket
---

# Getting Started

## Installation

```bash
uv add miniclient
```

## Quick Tour

The API simulates a browser's user interaction with the `Browser` class:

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
