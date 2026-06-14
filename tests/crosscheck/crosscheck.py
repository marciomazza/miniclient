import asyncio
import threading
from dataclasses import dataclass
from http.server import HTTPServer
from pathlib import Path
from typing import Iterable, Literal, cast
from urllib.parse import urlparse
from wsgiref.simple_server import WSGIRequestHandler, make_server
from wsgiref.types import WSGIApplication

import httpx
from playwright.async_api import Page, Request
from pydantic import BaseModel, field_validator

from htmxclient.browser import Browser
from htmxclient.runtime import build_runtime

_KEEP_REQUEST_HEADERS = {"content-type"}
_SKIP_RESPONSE_HEADERS = {"date", "server", "content-length"}
_HX_PREFIX = "hx-"

_SETTLE_INIT_SCRIPT = """\
window.__htmxSettled = false;
window.__htmxWillRequest = false;
document.addEventListener("htmx:after:settle", () => { window.__htmxSettled = true; });
document.addEventListener("htmx:before:request", () => { window.__htmxWillRequest = true; });
"""

_JS_SERIALIZE = """\
() => {
    // Use prototype getters to avoid shadowing by named form controls (e.g. name="tagName").
    const _getDescriptor = Object.getOwnPropertyDescriptor;

    const _tagName = _getDescriptor(Element.prototype, 'tagName').get;
    const _nodeType = _getDescriptor(Node.prototype, 'nodeType').get;
    const _childNodes = _getDescriptor(Node.prototype, 'childNodes').get;
    const _attributes = _getDescriptor(Element.prototype, 'attributes').get;
    const _getAttribute = Element.prototype.getAttribute;
    const _value = _getDescriptor(HTMLInputElement.prototype, 'value').get;
    const _inputType = _getDescriptor(HTMLInputElement.prototype, 'type').get;
    const _checked = _getDescriptor(HTMLInputElement.prototype, 'checked').get;
    const _textareaValue = _getDescriptor(HTMLTextAreaElement.prototype, 'value').get;
    const _selectValue = _getDescriptor(HTMLSelectElement.prototype, 'value').get;
    const _optionSelected = _getDescriptor(window.HTMLOptionElement.prototype, 'selected').get;

    function serializeNode(node) {
        const nodeType = _nodeType.call(node);
        if (nodeType === Node.TEXT_NODE) {
            // node.data is the reliable property for text nodes
            //   in both happy-dom and real browsers;
            // Node.prototype.textContent getter returns '' for Text nodes in happy-dom.
            const data = node.data.trim();
            return data ? {type: "text", data} : null;
        }
        if (nodeType !== Node.ELEMENT_NODE) return null;
        const tag = _tagName.call(node).toLowerCase();
        if (tag === "script") return null;
        const attrList = _attributes.call(node);
        const attrNames = [...attrList].map(a => a.name).sort();
        const attrs = {};
        for (const name of attrNames) attrs[name] = _getAttribute.call(node, name);
        const result = {tag, attrs};
        if (tag === "input") {
            result.value = _value.call(node);
            const t = _inputType.call(node);
            if (t === "checkbox" || t === "radio") result.checked = _checked.call(node);
        } else if (tag === "textarea") {
            result.value = _textareaValue.call(node);
        } else if (tag === "select") {
            result.value = _selectValue.call(node);
        } else if (tag === "option") {
            result.selected = _optionSelected.call(node);
        }
        result.children = [..._childNodes.call(node)].map(serializeNode).filter(Boolean);
        return result;
    }
    return serializeNode(document.body);
}
"""

_HTMX_JS = (Path(__file__).parents[2] / "vendor/htmx/src/htmx.js").read_bytes()


def _url_path_and_query(url: str) -> str:
    parsed = urlparse(url)
    return parsed.path + (f"?{parsed.query}" if parsed.query else "")


def _normalize_headers(headers: dict[str, str], keep: set[str]) -> dict[str, str]:
    result = {}
    for k, v in headers.items():
        key = k.lower()
        if key not in keep and not key.startswith(_HX_PREFIX):
            continue
        if key == "hx-current-url":
            v = _url_path_and_query(v)
        result[key] = v
    return result


def _normalize_response_headers(headers: dict[str, str]) -> dict[str, str]:
    return {k.lower(): v for k, v in headers.items() if k.lower() not in _SKIP_RESPONSE_HEADERS}


class CapturedRequest(BaseModel):
    method: str
    path: str
    headers: dict[str, str]

    @field_validator("path", mode="before")
    @classmethod
    def _extract_path(cls, v) -> str:
        return _url_path_and_query(str(v))

    @field_validator("headers", mode="before")
    @classmethod
    def _normalize(cls, v) -> dict[str, str]:
        return _normalize_headers(dict(v), _KEEP_REQUEST_HEADERS)


class CapturedResponse(BaseModel):
    status: int
    headers: dict[str, str]

    @field_validator("headers", mode="before")
    @classmethod
    def _normalize(cls, v) -> dict[str, str]:
        return _normalize_response_headers(dict(v))


class _SilentHandler(WSGIRequestHandler):
    def log_message(self, format, *args): ...


@dataclass
class Talk:
    request: CapturedRequest | None = None
    response: CapturedResponse | None = None

    def reset(self):
        self.request = self.response = None


class _AsyncByteStream(httpx.AsyncByteStream):
    """Wraps a bytes body as an AsyncByteStream (required by httpx.AsyncClient)."""

    def __init__(self, data: bytes) -> None:
        self._data = data

    async def __aiter__(self):
        yield self._data

    async def aclose(self) -> None:
        pass


class _AsyncWSGITransport(httpx.AsyncBaseTransport):
    """Runs a sync WSGI app as an async httpx transport via a thread executor."""

    def __init__(self, app: WSGIApplication) -> None:
        self._sync = httpx.WSGITransport(app=app)

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        loop = asyncio.get_running_loop()
        sync_response = await loop.run_in_executor(None, self._sync.handle_request, request)
        body = b"".join(cast(Iterable[bytes], sync_response.stream))
        response = httpx.Response(
            status_code=sync_response.status_code,
            headers=sync_response.headers,
            stream=_AsyncByteStream(body),
        )
        response._content = body  # pre-set so r.content works without await aread()
        return response


class _CapturingTransport(httpx.AsyncBaseTransport):
    """Wraps an async transport and records the last request/response into a Talk."""

    def __init__(self, wrapped: httpx.AsyncBaseTransport, talk: Talk) -> None:
        self._wrapped = wrapped
        self._talk = talk

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        self._talk.request = CapturedRequest(
            method=request.method, path=str(request.url), headers=dict(request.headers)
        )
        response = await self._wrapped.handle_async_request(request)
        self._talk.response = CapturedResponse(
            status=response.status_code, headers=dict(response.headers)
        )
        return response


_pages_initialized: set[int] = set()


class CrossCheck:
    def __init__(
        self,
        browser: Browser,
        page: Page,
        server: HTTPServer,
        port: int,
        client_talk: Talk,
        mode: Literal["htmx", "plain"] = "htmx",
    ) -> None:
        self.client_talk = client_talk
        self.page_talk = Talk()
        self._page_raw_request: Request | None = None
        self._browser = browser
        self._page = page
        self._server = server
        self._port = port
        self._mode = mode

    @classmethod
    async def create(
        cls, wsgi_app: WSGIApplication, page: Page, mode: Literal["htmx", "plain"] = "htmx"
    ) -> "CrossCheck":
        client_talk = Talk()
        capturing_transport = _CapturingTransport(_AsyncWSGITransport(wsgi_app), client_talk)
        runtime = await build_runtime("http://testserver/", httpx_transport=capturing_transport)
        browser = Browser(runtime)

        def _wrapped_wsgi(environ, start_response):
            if environ["PATH_INFO"] == "/htmx.js":
                start_response("200 OK", [("Content-Type", "application/javascript")])
                return [_HTMX_JS]
            return wsgi_app(environ, start_response)

        server = make_server("127.0.0.1", 0, _wrapped_wsgi, handler_class=_SilentHandler)
        port = server.server_address[1]
        threading.Thread(target=server.serve_forever, daemon=True).start()

        if id(page) not in _pages_initialized:
            await page.add_init_script(_SETTLE_INIT_SCRIPT)
            _pages_initialized.add(id(page))

        cc = cls(browser, page, server, port, client_talk, mode=mode)
        page.on("request", cc._hook_request_page)
        page.on("response", cc._hook_response_page)
        return cc

    # --- page event hooks (sync, called by Playwright internally) ---

    def _hook_request_page(self, request) -> None:
        if request.headers.get("hx-request"):
            self._page_raw_request = request
            self.page_talk.request = CapturedRequest(
                method=request.method, path=request.url, headers=dict(request.headers)
            )

    def _hook_response_page(self, response) -> None:
        if self._page_raw_request is None:
            return
        if response.request is self._page_raw_request:
            self.page_talk.response = CapturedResponse(
                status=response.status, headers=dict(response.headers)
            )
            self._page_raw_request = None

    def _reset_capture(self) -> None:
        self.client_talk.reset()
        self.page_talk.reset()
        self._page_raw_request = None

    def _server_url(self, path: str) -> str:
        return f"http://127.0.0.1:{self._port}{path}"

    # --- assertions ---

    def assert_same_talk(self) -> None:
        assert self.client_talk.request == self.page_talk.request, (
            f"Request mismatch:\n  client: {self.client_talk.request}\n"
            f"  page:   {self.page_talk.request}"
        )
        assert self.client_talk.response == self.page_talk.response, (
            f"Response mismatch:\n  client: {self.client_talk.response}\n"
            f"  page:   {self.page_talk.response}"
        )

    async def assert_same_dom(self) -> None:
        client_snap = self._browser.runtime.eval(f"({_JS_SERIALIZE})()")
        page_snap = await self._page.evaluate(_JS_SERIALIZE)
        assert client_snap == page_snap, (
            f"DOM mismatch:\n  client: {client_snap}\n  page:   {page_snap}"
        )

    async def assert_same_same(self) -> None:
        self.assert_same_talk()
        await self.assert_same_dom()

    # --- navigation / interaction ---

    async def goto(self, path: str) -> None:
        await asyncio.gather(
            self._browser.goto(f"http://testserver{path}"),
            self._page.goto(self._server_url(path), wait_until="domcontentloaded"),
        )
        await self.assert_same_dom()

    async def _page_click_navigate(self, selector: str) -> None:
        async with self._page.expect_navigation(wait_until="domcontentloaded", timeout=5000):
            await self._page.locator(selector).click()

    async def click(self, selector: str, is_submit: bool = False) -> None:
        if self._mode == "plain" and is_submit:
            await asyncio.gather(
                self._browser.submit_form(selector),
                self._page_click_navigate(selector),
            )
            await self.assert_same_dom()
            return

        self._reset_capture()
        el = self._browser.find(selector)
        assert el is not None, f"Element not found: {selector!r}"
        await self._page.evaluate("window.__htmxSettled = false;")
        await asyncio.gather(
            el.click(),
            self._page.locator(selector).click(),
        )
        if self.client_talk.request is not None:
            await self._page.wait_for_function("() => window.__htmxSettled", timeout=5000)
            await self.assert_same_same()
        else:
            await self.assert_same_dom()

    async def fill(self, selector: str, value: str) -> None:
        el = self._browser.find(selector)
        if el is None:
            raise LookupError(f"No element matches {selector!r}")
        self._reset_capture()
        await self._page.evaluate("window.__htmxSettled = false;")
        el.fill(value)
        await el.trigger("change")
        await self._page.locator(selector).fill(value)
        await self._page.locator(selector).dispatch_event("change")
        if self.client_talk.request is not None:
            await self._page.wait_for_function("() => window.__htmxSettled", timeout=5000)
            await self.assert_same_same()
        else:
            await self.assert_same_dom()

    async def dispatch_event(self, selector: str, event: str) -> None:
        el = self._browser.find(selector)
        if el is None:
            raise LookupError(f"No element matches {selector!r}")
        self._reset_capture()
        await self._page.evaluate("window.__htmxSettled = false;")
        await asyncio.gather(
            el.trigger(event),
            self._page.locator(selector).dispatch_event(event),
        )
        if self.client_talk.request is not None:
            await self._page.wait_for_function("() => window.__htmxSettled", timeout=5000)
            await self.assert_same_same()
        else:
            await self.assert_same_dom()

    async def stop(self) -> None:
        self._page.remove_listener("request", self._hook_request_page)
        self._page.remove_listener("response", self._hook_response_page)
        self._server.shutdown()
        self._browser.close()
