from wsgiref.types import StartResponse, WSGIEnvironment

from miniclient.browser import AsyncBrowser
from miniclient.wsgi import WSGITransport


def _app(environ: WSGIEnvironment, start_response: StartResponse):
    start_response("200 OK", [("Content-Type", "text/html")])
    return [b"<html><body><p id='msg'>hello from wsgi</p></body></html>"]


async def test_goto_via_wsgi_transport(snapshot: bytes) -> None:
    browser = await AsyncBrowser(snapshot=snapshot, httpx_transport=WSGITransport(app=_app))
    try:
        await browser.goto("http://testserver/")
        el = browser.find("#msg")
        assert el is not None
        assert el.text() == "hello from wsgi"
    finally:
        browser.close()
