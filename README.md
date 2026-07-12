# miniclient

[![CI](https://github.com/marciomazza/miniclient/actions/workflows/ci.yml/badge.svg)](https://github.com/marciomazza/miniclient/actions/workflows/ci.yml)
[![Docs](https://github.com/marciomazza/miniclient/actions/workflows/docs.yml/badge.svg)](https://marciomazza.github.io/miniclient/)
[![PyPI](https://img.shields.io/pypi/v/miniclient)](https://pypi.org/project/miniclient/)
[![Python versions](https://img.shields.io/pypi/pyversions/miniclient)](https://pypi.org/project/miniclient/)
[![License](https://img.shields.io/github/license/marciomazza/miniclient)](LICENSE)

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

with Browser() as browser:
    browser.goto("http://localhost:8000/")
    browser.find("input[name=q]").fill("htmx")
    browser.find("form").requestSubmit()
    browser.find("#load-more").click()
    print(len(browser.find_all("#results li")), "results")
    print(browser.find("#results li:first-child").text())
```

Check the full documentation: <https://marciomazza.github.io/miniclient/>
