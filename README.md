# miniclient

A minimal python http client that runs JavaScript, without a browser.
Meant to be used as a lightweight test client to simulate browser interactions.
Embeds a V8 Runtime, DOM, and is designed to run [htmx](https://htmx.org) especially well.

## Install

```bash
uv add miniclient
```

## Example

The snippet below opens a page, submits a search form, clicks a "load more" button and reads
the results back. All against a real in-memory DOM running JavaScript (htmx, for example).
No browser involved.

```python
from miniclient.browser import Browser

async with await Browser.create() as browser:
    await browser.goto("http://localhost:8000/")
    browser.find("input[name=q]").fill("htmx")
    await browser.find("form").requestSubmit()
    await browser.find("#load-more").click()
    print(len(browser.find_all("#results li")), "results")
    print(browser.find("#results li:first-child").text())
```

Check the full documentation: <https://marciomazza.github.io/miniclient/>
