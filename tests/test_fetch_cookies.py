from wsgiref.types import StartResponse, WSGIEnvironment

import pytest

from miniclient.page import AsyncPage
from miniclient.wsgi import WSGITransport


def _echo_cookie_app(environ: WSGIEnvironment, start_response: StartResponse):
    cookie = environ.get("HTTP_COOKIE")
    body = f"<html><body><p id='cookie'>{cookie!r}</p></body></html>".encode()
    start_response("200 OK", [("Content-Type", "text/html")])
    return [body]


async def test_document_cookie_reaches_request_cookie_header() -> None:
    page = await AsyncPage(httpx_transport=WSGITransport(app=_echo_cookie_app))
    try:
        await page.goto("http://testserver/")
        page.runtime.eval("document.cookie = 'sessionid=abc123; path=/'")

        await page.goto("http://testserver/")
        el = page.find("#cookie")
        assert el and "sessionid=abc123" in el.text
    finally:
        page.close()


@pytest.mark.parametrize(
    "set_cookie_header,expected_document_cookie",
    [
        ("sessionid=xyz789; Path=/", "sessionid=xyz789"),
        # HttpOnly cookies must still reach the wire but stay hidden from document.cookie.
        ("sessionid=httponlyval; Path=/; HttpOnly", ""),
    ],
)
async def test_set_cookie_response_reaches_next_request(
    set_cookie_header: str, expected_document_cookie: str
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

    page = await AsyncPage(httpx_transport=WSGITransport(app=_app))
    try:
        await page.goto("http://testserver/login")
        assert page.runtime.eval("document.cookie") == expected_document_cookie

        await page.goto("http://testserver/")
        el = page.find("#cookie")
        assert el and cookie_pair in el.text
    finally:
        page.close()


async def test_cookie_set_on_followed_redirect_reaches_jar() -> None:
    # A hop that answers 302 + Set-Cookie (Django login()/session rotation): the cookie must
    # reach happy-dom's jar even though it never appears on the final response's headers.
    def _app(environ: WSGIEnvironment, start_response: StartResponse):
        if environ["PATH_INFO"] == "/login":
            start_response(
                "302 Found", [("Location", "/echo"), ("Set-Cookie", "sessionid=rotated; Path=/")]
            )
            return [b""]
        start_response("200 OK", [("Content-Type", "text/html")])
        return [b"<html><body>ok</body></html>"]

    page = await AsyncPage(httpx_transport=WSGITransport(app=_app))
    try:
        await page.goto("http://testserver/login")
        assert page.runtime.eval("document.cookie") == "sessionid=rotated"
    finally:
        page.close()


async def test_cookie_survives_auto_followed_redirect() -> None:
    # httpx follows the 302 internally (`follow_redirects=True` in open_runtime), without
    # going back through JS -- the redirected leg must still carry the cookie happy-dom's
    # jar already has, not just the leg fetch() was originally called for.
    def _app(environ: WSGIEnvironment, start_response: StartResponse):
        if environ["PATH_INFO"] == "/submit" and environ["REQUEST_METHOD"] == "POST":
            start_response("302 Found", [("Location", "/echo")])
            return [b""]
        cookie = environ.get("HTTP_COOKIE")
        body = f"<html><body><p id='cookie'>{cookie!r}</p></body></html>".encode()
        start_response("200 OK", [("Content-Type", "text/html")])
        return [body]

    page = await AsyncPage(httpx_transport=WSGITransport(app=_app))
    try:
        await page.goto("http://testserver/echo")
        page.runtime.eval("document.cookie = 'sessionid=abc123; path=/'")

        result = await page.runtime.eval_async(
            "fetch('/submit', {method: 'POST'}).then(r => r.text())"
        )
        assert "sessionid=abc123" in result
    finally:
        page.close()
