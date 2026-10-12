---
icon: lucide/book-open
---

# Usage

The `Page` class simulates a browser page for testing.
It runs a real V8 engine (via `deno_core`) with jsdom providing the DOM,
and lets you drive it from Python — load pages, query and interact with DOM elements, fill forms,
click and trigger events.
If the page includes htmx, it runs normally and htmx requests are awaited automatically.

## Creating a page

Every page comes from a `Browser`:

```python
from miniclient.browser import Browser, Page

browser = Browser()
page = Page(browser)
...
page.close()
```

`Page` can also be used as a context manager (`with Page(browser) as page:`) to close it on exit.

`Browser(...)` accepts:

- `httpx_transport` — an `httpx2.AsyncBaseTransport`, useful to test an ASGI/WSGI app in-process
  without an HTTP server (see below).
- `mounts` — a `dict[str, Path]` mapping a URL prefix to a local directory, so `<script>` tags can
  load local files (e.g. htmx itself or a bundle) without going through an HTTP request.
- `preboot` — `True` by default. `False` skips the background boot: each page boots its runtime
  when created, which is slower but holds one isolate less in memory.

The `Browser` boots the next page's runtime in the background, so creating a page per test is
nearly free. Pages are independent (own DOM, cookies, event loop and httpx client). The `Browser`
needs no closing: drop it when you are done. To add helpers, subclass `Page` and create it the
same way. With pytest:

```python
@pytest.fixture(scope="session")
def browser():
    return Browser(httpx_transport=WSGITransport(app=app))

@pytest.fixture
def page(browser):
    with Page(browser) as page:
        yield page
```

### Async

An `AsyncPage` with the same constructor is available for async codebases:

```python
from miniclient.browser import AsyncPage, Browser

async with AsyncPage(Browser()) as page:
    ...
```

## Testing a WSGI/ASGI app in-process

You can test your WSGI/ASGI app directly (Django, Flask, FastAPI, etc)
with no HTTP server or network involved, by passing a `miniclient.wsgi.WSGITransport` to `Browser(...)`.

An example with [nanodjango](https://nanodjango.dev/)
_(and [`htmx.min.js`](https://four.htmx.org/docs#download) in the folder `"path/to/htmx/dist"`)_:

```python
from pathlib import Path

from miniclient.browser import Browser, Page
from miniclient.wsgi import WSGITransport
from nanodjango import Django

app = Django()

@app.route("/")
def index(request):
    return """
    <html>
      <head>
        <script src="http://localhost/static/htmx.min.js"></script>
      </head>
      <body>
        <button hx-get="/hello" hx-target="#result">Say hi</button>
        <div id="result"></div>
      </body>
    </html>
    """

@app.route("/hello")
def hello(request):
    return "Hello from Django!"

browser = Browser(
    httpx_transport=WSGITransport(app=app.wsgi),
    mounts={"http://localhost/static/": Path("path/to/htmx/dist")},
)
with Page(browser) as page:
    page.goto("/")
    page.find("button").click()
    print(page.find("#result").text)  # prints "Hello from Django!"
```

For an ASGI app instead, pass an `httpx2.ASGITransport(app=app.asgi)` — see
[httpx's documentation](https://www.python-httpx.org/advanced/transports/#asgi-transport).

## Loading external scripts via `mounts`

Serve local files through `mounts` so a `<script>` tag can load them without a real server:

```python
page = Page(Browser(mounts={"http://localhost/ext/": tmp_path}))
page.eval(
    'document.head.innerHTML = \'<script src="http://localhost/ext/external-script.js"></script>\''
)
```

## Loading pages

Load content either via a real request, or directly as raw HTML:

```python
# Fetch a URL, load the full document, and process htmx (real request via httpx_transport/network)
page.goto("http://localhost/page")

# Load raw HTML directly into the document body, no request involved
page.load("<p id='msg'>hello</p>")
```

Each call to `load()` replaces the previous body entirely.

A new page starts blank at `http://localhost/`. Its first `goto()` reuses that page's JavaScript
realm instead of booting another, so anything `eval()` defined on `window` before it (globals,
listeners) is still there afterwards; every later `goto()` starts a fresh realm. Cookies carry
over either way.

## Finding elements

Locate elements by CSS selector, returning `Element` wrappers:

```python
el = page.find("#msg")          # the first match, or None
items = page.find_all("li")     # a list of all matches, possibly empty
```

Pass `text` to also filter by contained text (a substring match against `textContent`):

```python
el = page.find("li", text="Buy milk")        # first <li> containing this text, or None
items = page.find_all("li", text="urgent")   # all <li>s containing this text
```

`Element` also exposes `find()`/`find_all()`, scoped to that element instead of the whole
document:

```python
row = page.find("#results li")
label = row.find(".label")                      # searches only within `row`
badges = row.find_all(".badge", text="new")
```

## Reading elements

`Element` exposes the usual ways to read content and attributes:

```python
el.html          # outerHTML — the element's tag plus its content
el.innerHTML     # innerHTML — the element's content, without its own tag
el.text          # textContent — all text inside, with tags stripped
el.attr("href")  # value of the "href" attribute, or None if absent
el.parent        # the parent Element, or None if there is no parent
```

## Filling inputs

Set an input's value directly:

```python
input = page.find("#input-id")
input.fill("new value")
```

This works for `<input>`, `<textarea>` and `<select>` elements.
For `<select>`, this only takes effect if the value matches an existing `<option>`'s value,
just like in a real browser.

## Clicking and triggering events

Simulate a click, or dispatch any DOM event:

```python
page.find("button").click()
page.find("div").trigger("my-event")  # any DOM event, e.g. for hx-trigger="my-event"
```

Both wait for the page to settle (any pending timers/fetches, e.g. from an htmx request, a
plain `fetch()`, or another framework's async work).

## Submitting forms

`page.find(...)` returns a `FormElement` when the match is a `<form>`, which adds
`request_submit()`:

```python
form = page.find("form")
form.request_submit()
```

If a script (e.g. htmx via `hx-post`, `hx-get`, ...) intercepts the submit, this waits for the
page to settle. If not, it performs the form's native GET/POST navigation and reloads the page.
Clicking a `<button type="submit">` or `<input type="submit">` inside the form works the same
way, through `.click()`.

## Executing JavaScript

For anything not covered by `Page` / `Element`, evaluate JavaScript directly.

With sync `Page`, use `eval()`:

```python
title = page.eval("document.title")
```

`Element` has its own `eval()`, with `this` bound to that element:

```python
height = page.find("#panel").eval("this.scrollHeight")
```

On an `AsyncPage`, elements also have `eval_async()`:

```python
async with AsyncPage(browser) as page:
    data = await page.find("#panel").eval_async("this.loadData()")  # this.loadData() returns a promise
```

`AsyncPage` also has `eval_async()`, which awaits the result when the JavaScript returns a
promise:

```python
async with AsyncPage(browser) as page:
    title = page.eval("document.title")
    status = await page.eval_async("fetch('/api/status').then(r => r.json())")
```
