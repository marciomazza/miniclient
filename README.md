# miniclient

## What is this?

A python HTTP client for testing [htmx](https://htmx.org)-powered server applications.

This is a lightweight client that simulates browser interactions
running htmx and [happy-dom](https://github.com/capricorn86/happy-dom) inside a V8 JavaScript runtime.

This way requests, DOM swaps and event handling all go through htmx actual code.
HTTP is done with [httpx2](https://httpx2.pydantic.dev/). V8 is embedded by [jsrun](https://imfing.github.io/jsrun),
that enables running JavaScript code directly from python.

## Usage

The api simulates a browser general interaction with the `Browser` and `Element` classes:

```python
from miniclient.browser import Browser

async with await Browser.create() as browser:
    await browser.goto("http://localhost:8000/")
    await browser.find("#load-more").click()
    print(browser.find("#results").text())
```

This lets you assert on real htmx behavior (swaps, OOB updates, events, morphing, ...) against
your server's actual HTML responses, without browser automation tools like Playwright.

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

For anything not covered by `Browser` / `Element`,
you can run arbitrary JavaScript directly through `browser.runtime`:

```python
from miniclient.browser import Browser

async with await Browser.create() as browser:
    await browser.load("<h1 id='greeting'>Hello</h1>")
    print(browser.runtime.eval("document.getElementById('greeting').textContent"))
    browser.runtime.eval("document.getElementById('greeting').textContent = 'Hi!'")
    print(browser.find("#greeting").text())
```

For other possible uses of the JavaScript runtime, check the [jsrun](https://imfing.github.io/jsrun) documentation.

## Why?

Testing against a real browser is simply too slow. And mostly unnecessary.

## Testing an ASGI/WSGI app in-process

You can test your ASGI/WSGI app directly (Django, Flask, FastAPI, etc)
with no HTTP server or network involved, by passing an `httpx2.ASGITransport` to `Browser.create()`.

An example with [nanodjango](https://nanodjango.dev/):

```python
import httpx2
from miniclient.browser import Browser
from nanodjango import Django

app = Django()

@app.route("/")
def index(request):
    return '<button hx-get="/hello" hx-target="#result">Say hi</button><div id="result"></div>'

@app.route("/hello")
def hello(request):
    return "Hello from Django!"

async with await Browser.create(
    "http://testserver/",
    httpx_transport=httpx2.ASGITransport(app=app.asgi),
) as browser:
    await browser.goto("http://testserver/")
    await browser.find("button").click()
    print(browser.find("#result").text())  # prints "Hello from Django!"
```

## Development

To bootstrap this project environment, run:

```bash
uv sync
npm install
./scripts/setup-vendor-htmx.sh   # clones htmx source into vendor/htmx, required by the tests
```

The test suite includes some hypothesis property based tests that crosscheck the behavior of this
client against a real browser in many different scenarios.
This part of the tests can be rather slow so they're disabled by default.

To run all tests but the crosscheck, simply use:

```bash
pytest
```
To run only the hypothesis crosscheck tests:

```bash
pytest -m "cross"
```

To run the full suite, including the crosscheck tests:

```bash
pytest -m ""
```

Tests can run in parallel by adding the `pytest` option `-n auto`.
