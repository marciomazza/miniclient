from wsgiref.types import StartResponse, WSGIEnvironment

from miniclient.browser import AsyncPage, Browser
from miniclient.wsgi import WSGITransport


async def test_cookie_survives_auto_followed_redirect() -> None:
    # The 302 is followed hop by hop in the page, so the redirected leg must carry the cookie
    # from the page's jar like the first one. Pytest-only: cargo tests mock the backend, which
    # has no WSGI app to answer the second hop (the rest of cookie handling lives in
    # tests/browser_cookies.rs).
    def _app(environ: WSGIEnvironment, start_response: StartResponse):
        if environ["PATH_INFO"] == "/submit" and environ["REQUEST_METHOD"] == "POST":
            start_response("302 Found", [("Location", "/echo")])
            return [b""]
        cookie = environ.get("HTTP_COOKIE")
        body = f"<html><body><p id='cookie'>{cookie!r}</p></body></html>".encode()
        start_response("200 OK", [("Content-Type", "text/html")])
        return [body]

    page = AsyncPage(Browser(httpx_transport=WSGITransport(app=_app)))
    try:
        await page.goto("http://testserver/echo")
        await page.eval("document.cookie = 'sessionid=abc123; path=/'")

        result = await page.eval("fetch('/submit', {method: 'POST'}).then(r => r.text())")
        assert "sessionid=abc123" in result
    finally:
        page.close()
