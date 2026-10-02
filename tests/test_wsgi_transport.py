from wsgiref.types import StartResponse, WSGIEnvironment

from miniclient.page import AsyncPage
from miniclient.wsgi import WSGITransport


def _app(environ: WSGIEnvironment, start_response: StartResponse):
    start_response("200 OK", [("Content-Type", "text/html")])
    return [b"<html><body><p id='msg'>hello from wsgi</p></body></html>"]


async def test_goto_via_wsgi_transport() -> None:
    page = AsyncPage(httpx_transport=WSGITransport(app=_app))
    try:
        await page.goto("http://testserver/")
        el = page.find("#msg")
        assert el is not None
        assert el.text == "hello from wsgi"
    finally:
        page.close()
