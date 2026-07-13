from wsgiref.types import StartResponse, WSGIEnvironment

import pytest

from miniclient.browser import AsyncBrowser
from miniclient.wsgi import WSGITransport


def _echo_cookie_app(environ: WSGIEnvironment, start_response: StartResponse):
    cookie = environ.get("HTTP_COOKIE")
    body = f"<html><body><p id='cookie'>{cookie!r}</p></body></html>".encode()
    start_response("200 OK", [("Content-Type", "text/html")])
    return [body]


async def test_document_cookie_reaches_request_cookie_header(snapshot: bytes) -> None:
    browser = await AsyncBrowser(
        snapshot=snapshot, httpx_transport=WSGITransport(app=_echo_cookie_app)
    )
    try:
        await browser.goto("http://testserver/")
        browser.runtime.eval("document.cookie = 'sessionid=abc123; path=/'")

        await browser.goto("http://testserver/")
        el = browser.find("#cookie")
        assert el and "sessionid=abc123" in el.text
    finally:
        browser.close()


@pytest.mark.parametrize(
    "set_cookie_header,expected_document_cookie",
    [
        ("sessionid=xyz789; Path=/", "sessionid=xyz789"),
        # HttpOnly cookies must still reach the wire but stay hidden from document.cookie.
        ("sessionid=httponlyval; Path=/; HttpOnly", ""),
    ],
)
async def test_set_cookie_response_reaches_next_request(
    snapshot: bytes, set_cookie_header: str, expected_document_cookie: str
) -> None:
    cookie_pair = set_cookie_header.split(";")[0].strip()

    def _app(environ: WSGIEnvironment, start_response: StartResponse):
        cookie = environ.get("HTTP_COOKIE")
        body = f"<html><body><p id='cookie'>{cookie!r}</p></body></html>".encode()
        headers = [("Content-Type", "text/html")]
        if environ["PATH_INFO"] == "/login":
            headers.append(("Set-Cookie", set_cookie_header))
        start_response("200 OK", headers)
        return [body]

    browser = await AsyncBrowser(snapshot=snapshot, httpx_transport=WSGITransport(app=_app))
    try:
        await browser.goto("http://testserver/login")
        assert browser.runtime.eval("document.cookie") == expected_document_cookie

        await browser.goto("http://testserver/")
        el = browser.find("#cookie")
        assert el and cookie_pair in el.text
    finally:
        browser.close()
