from wsgiref.types import StartResponse, WSGIEnvironment

from miniclient.page import AsyncPage
from miniclient.wsgi import WSGITransport


def _app(environ: WSGIEnvironment, start_response: StartResponse):
    start_response("200 OK", [("Content-Type", "text/html")])
    return [b"<html><body><p id='msg'>hello from wsgi</p></body></html>"]


async def test_goto_via_wsgi_transport() -> None:
    page = await AsyncPage(httpx_transport=WSGITransport(app=_app))
    try:
        await page.goto("http://testserver/")
        el = page.find("#msg")
        assert el is not None
        assert el.text == "hello from wsgi"
    finally:
        page.close()


async def test_sync_xhr_via_wsgi_transport() -> None:
    # WSGITransport is async-only (handle_async_request, no handle_request), so a
    # sync XHR going through the old per-call `httpx.Client(transport=...)` would
    # raise AttributeError. Regression test for routing sync fetch through the
    # shared AsyncClient instead.
    page = await AsyncPage(httpx_transport=WSGITransport(app=_app))
    try:
        await page.goto("http://testserver/")
        result = await page.runtime.eval_async("""
            (() => {
              const xhr = new XMLHttpRequest();
              xhr.open('GET', 'http://testserver/', false);
              xhr.send();
              return {status: xhr.status, body: xhr.responseText};
            })();
        """)
        assert result["status"] == 200
        assert "hello from wsgi" in result["body"]
    finally:
        page.close()
