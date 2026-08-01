---
icon: lucide/rocket
---

# Getting Started

## Installation

```bash
uv add miniclient
```

## Quick Tour

The API simulates a browser's user interaction with the `Page` class:

```python
from miniclient.page import Page

with Page() as page:
    page.goto("http://localhost:8000/")
    page.find("#load-more").click()
    print(page.find("#results").text)
```

Filling in a form and submitting it works the same way, through `fill()` and `requestSubmit()`:

```python
from miniclient.page import Page

with Page() as page:
    page.goto("http://localhost:8000/signup")
    page.find("input[name=name]").fill("Ada")
    page.find("input[name=email]").fill("ada@example.com")
    page.find("form").requestSubmit()
    print(page.find("#result").text)
```

For anything not covered by `Page` / `Element`, you can run arbitrary JavaScript directly
through `eval()`:

```python
from miniclient.page import Page

with Page() as page:
    page.load("<h1 id='greeting'>Hello</h1>")
    print(page.eval("document.getElementById('greeting').textContent"))
    page.eval("document.getElementById('greeting').textContent = 'Hi!'")
    print(page.find("#greeting").text)
```

Note: `Page` doesn't expose a `.runtime` property the way `AsyncPage` does — use `eval()`
instead.

## Async usage

An `AsyncPage` equivalent is available for async codebases:

```python
from miniclient.page import AsyncPage

async with AsyncPage() as page:
    await page.goto("http://localhost:8000/")
    await page.find("#load-more").click()
    print(page.find("#results").text)
```
