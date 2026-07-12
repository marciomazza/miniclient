---
icon: lucide/book-open
---

# Usage

The `Browser` class simulates browser interaction for testing. It runs a real V8 engine (via jsrun) with
happy-dom providing the DOM, and lets you drive it from Python — load pages, query and interact
with DOM elements, fill forms, click and trigger events.
If the page includes htmx, it runs normally and htmx requests are awaited automatically.

## Creating a browser

Create a browser with a context manager:

```python
from miniclient.browser import Browser

with Browser() as browser:
    ...
```

`Browser(...)` accepts:

- `url` — the initial page URL (default `"http://localhost/"`).
- `httpx_transport` — an `httpx2.AsyncBaseTransport`, useful to test an ASGI/WSGI app in-process
  with no real HTTP server (see below).
- `mounts` — a `dict[str, Path]` mapping a URL prefix to a local directory, so `<script>` tags can
  load local files (e.g. htmx itself) without a real server.

### Async

An `AsyncBrowser` with the same constructor is available for async codebases:

```python
from miniclient.browser import AsyncBrowser

async with AsyncBrowser() as browser:
    ...
```

## Testing a WSGI/ASGI app in-process

You can test your WSGI/ASGI app directly (Django, Flask, FastAPI, etc)
with no HTTP server or network involved, by passing a `miniclient.wsgi.WSGITransport` to `Browser(...)`.

An example with [nanodjango](https://nanodjango.dev/):

```python
from miniclient.browser import Browser
from miniclient.wsgi import WSGITransport
from nanodjango import Django

app = Django()

@app.route("/")
def index(request):
    return '<button hx-get="/hello" hx-target="#result">Say hi</button><div id="result"></div>'

@app.route("/hello")
def hello(request):
    return "Hello from Django!"

with Browser(
    httpx_transport=WSGITransport(app=app.wsgi),
    url="http://testserver/",
) as browser:
    browser.goto("/")
    browser.find("button").click()
    print(browser.find("#result").text())  # prints "Hello from Django!"
```

For an ASGI app instead, pass an `httpx2.ASGITransport(app=app.asgi)` — see
[httpx's documentation](https://www.python-httpx.org/advanced/transports/#asgi-transport).

## Loading external scripts via `mounts`

Serve local files through `mounts` so a `<script>` tag can load them without a real server:

```python
browser = Browser(mounts={"http://localhost/ext/": tmp_path})
browser.eval(
    'document.head.innerHTML = \'<script src="http://localhost/ext/external-script.js"></script>\''
)
```

## Loading pages

Load content either via a real request, or directly as raw HTML:

```python
# Fetch a URL, load the full document, and process htmx (real request via httpx_transport/network)
browser.goto("http://localhost/page")

# Load raw HTML directly into the document body, no request involved
browser.load("<p id='msg'>hello</p>")
```

Each call to `load()` replaces the previous body entirely.

## Finding elements

Locate elements by CSS selector, returning `Element` wrappers:

```python
el = browser.find("#msg")          # the first match, or None
items = browser.find_all("li")     # a list of all matches, possibly empty
```

## Reading elements

`Element` exposes the usual ways to read content and attributes:

```python
el.html()        # outerHTML — the element's tag plus its content
el.innerHTML()   # innerHTML — the element's content, without its own tag
el.text()        # textContent — all text inside, with tags stripped
el.attr("href")  # value of the "href" attribute, or None if absent
```

## Filling inputs

Set an input's value directly:

```python
input = browser.find("#input-id")
input.fill("new value")
```
This works for `<input>`, `<textarea>` and `<select>` elements.
For `<select>`, this only takes effect if the value matches an existing `<option>`'s value,
just like in a real browser.

## Clicking and triggering events

Simulate a click, or dispatch any DOM event:

```python
browser.find("button").click()
browser.find("div").trigger("my-event")  # any DOM event, e.g. for hx-trigger="my-event"
```

Both wait for htmx to settle if the event fires an htmx request.

## Submitting forms

`browser.find(...)` returns a `FormElement` when the match is a `<form>`, which adds
`requestSubmit()`:

```python
form = browser.find("form")
form.requestSubmit()
```

If the form is htmx-wired (`hx-post`, `hx-get`, ...), this waits for htmx to settle. If not, it
performs a plain fetch and reloads the page. Clicking a `<button type="submit">` or `<input type="submit">` inside the form works the same way, through `.click()`.

## Executing JavaScript

For anything not covered by `Browser` / `Element`, evaluate JavaScript directly.

With sync `Browser`, use `eval()` (`Browser` doesn't expose a `.runtime` property the way
`AsyncBrowser` does — the raw `Runtime` isn't thread-safe, so use this method instead):

```python
browser.eval("document.title")
```

With `AsyncBrowser`, use `.runtime`, which also supports async evaluation:

```python
browser.runtime.eval("document.title")
await browser.runtime.eval_async("fetch('/api/status').then(r => r.json())")
```

For other uses of the JavaScript runtime, check the [jsrun documentation](https://imfing.github.io/jsrun).
