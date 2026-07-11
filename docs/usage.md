---
icon: lucide/book-open
---

# Usage

The examples below are adapted from the test suite (`tests/test_browser.py`,
`tests/test_htmx_integration.py`) — they're runnable patterns, not illustrations.

## Creating a browser

```python
from htmxclient.browser import Browser

async with await Browser.create() as browser:
    ...
```

`Browser.create()` accepts:

- `url` — the initial page URL (default `"http://localhost/"`).
- `httpx_transport` — an `httpx2.AsyncBaseTransport`, useful to test an ASGI/WSGI app in-process
  with no real HTTP server (see below).
- `mounts` — a `dict[str, Path]` mapping a URL prefix to a local directory, so `<script src="...">`
  can load local files (e.g. htmx itself) without a real server.

### Testing an ASGI/WSGI app in-process

```python
import httpx2
from htmxclient.browser import Browser
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

### Loading external scripts via `mounts`

```python
b = await Browser.create(mounts={"http://localhost/ext/": tmp_path})
b.runtime.eval(
    'document.head.innerHTML = \'<script src="http://localhost/ext/external-script.js"></script>\''
)
```

## Loading pages

```python
# Fetch a URL, load the full document, and process htmx (real request via httpx_transport/network)
await browser.goto("http://localhost/page")

# Load raw HTML directly into the document body, no request involved
await browser.load("<p id='msg'>hello</p>")
```

Each call to `load()` replaces the previous body entirely.

## Finding elements

```python
el = browser.find("#msg")          # first match, or None
items = browser.find_all("li")     # all matches, possibly empty
```

## Reading elements

```python
el.html()        # outerHTML
el.innerHTML()   # innerHTML
el.text()        # textContent
el.attr("href")  # attribute value, or None if absent
```

## Filling inputs

```python
el = browser.find("#inp")
el.fill("new value")   # works for input, textarea, select
```

For `<select>`, this only takes effect if the value matches an existing `<option>`'s value —
same as in a real browser.

## Clicking and triggering events

```python
await browser.find("button").click()

# Any DOM event, e.g. for hx-trigger="my-event"
await browser.find("button").trigger("my-event")
```

Both wait for htmx to settle if the event fires an htmx request.

## Submitting forms

`browser.find(...)` returns a `FormElement` when the match is a `<form>`, which adds
`requestSubmit()`:

```python
form = browser.find("form")
await form.requestSubmit()
```

If the form is htmx-wired (`hx-post`, `hx-get`, ...), this waits for htmx to settle. If not, it
performs a plain fetch and reloads the page — matching what a real browser does for a normal form
submit. Clicking a `<button type="submit">` or `<input type="submit">` inside the form works the
same way, through `.click()`.

## Raw JavaScript escape hatch

For anything not covered by `Browser` / `Element`, evaluate JavaScript directly against the
happy-dom `document`:

```python
browser.runtime.eval("document.title")
await browser.runtime.eval_async("some async js expression")
```
