---
icon: lucide/book-open
---

# Usage

The `Page` class simulates a browser page for testing.
It runs a real V8 engine (via `deno_core`) with happy-dom providing the DOM,
and lets you drive it from Python — load pages, query and interact with DOM elements, fill forms,
click and trigger events.
If the page includes htmx, it runs normally and htmx requests are awaited automatically.

## Creating a page

Create a page with a context manager:

```python
from miniclient.page import Page

with Page() as page:
    ...
```

`Page(...)` accepts:

- `httpx_transport` — an `httpx2.AsyncBaseTransport`, useful to test an ASGI/WSGI app in-process
  with no real HTTP server (see below).
- `mounts` — a `dict[str, Path]` mapping a URL prefix to a local directory, so `<script>` tags can
  load local files (e.g. htmx itself) without a real server.

### Async

An `AsyncPage` with the same constructor is available for async codebases:

```python
from miniclient.page import AsyncPage

async with AsyncPage() as page:
    ...
```

## Testing a WSGI/ASGI app in-process

You can test your WSGI/ASGI app directly (Django, Flask, FastAPI, etc)
with no HTTP server or network involved, by passing a `miniclient.wsgi.WSGITransport` to `Page(...)`.

An example with [nanodjango](https://nanodjango.dev/)
_(and [`htmx.min.js`](https://four.htmx.org/docs#download) in the folder `"path/to/htmx/dist"`)_:

```python
from pathlib import Path

from miniclient.page import Page
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

with Page(
    httpx_transport=WSGITransport(app=app.wsgi),
    mounts={"http://localhost/static/": Path("path/to/htmx/dist")},
) as page:
    page.goto("/")
    page.find("button").click()
    print(page.find("#result").text)  # prints "Hello from Django!"
```

For an ASGI app instead, pass an `httpx2.ASGITransport(app=app.asgi)` — see
[httpx's documentation](https://www.python-httpx.org/advanced/transports/#asgi-transport).

## Loading external scripts via `mounts`

Serve local files through `mounts` so a `<script>` tag can load them without a real server:

```python
page = Page(mounts={"http://localhost/ext/": tmp_path})
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
`requestSubmit()`:

```python
form = page.find("form")
form.requestSubmit()
```

If a script (e.g. htmx via `hx-post`, `hx-get`, ...) intercepts the submit, this waits for the
page to settle. If not, it performs the form's native GET/POST navigation and reloads the page.
Clicking a `<button type="submit">` or `<input type="submit">` inside the form works the same
way, through `.click()`.

## Executing JavaScript

For anything not covered by `Page` / `Element`, evaluate JavaScript directly.

With sync `Page`, use `eval()` (`Page` doesn't expose a `.runtime` property the way
`AsyncPage` does — the raw `Runtime` isn't thread-safe, so use this method instead):

```python
page.eval("document.title")
```

With `AsyncPage`, use `.runtime`, which also supports async evaluation:

```python
page.runtime.eval("document.title")
await page.runtime.eval_async("fetch('/api/status').then(r => r.json())")
```
