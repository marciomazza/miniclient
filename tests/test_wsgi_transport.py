import threading
from concurrent.futures import ThreadPoolExecutor
from wsgiref.types import StartResponse, WSGIEnvironment

from miniclient.browser import AsyncPage, Browser
from miniclient.wsgi import WSGITransport

# Exercises the WSGITransport adapter itself (WSGI app -> httpx), not generic fetch-backend-to-
# browser plumbing -- that's already covered by tests/browser_navigation.rs against
# MockFetchBackend. WSGITransport has no Rust equivalent to port to.


def _app(environ: WSGIEnvironment, start_response: StartResponse):
    start_response("200 OK", [("Content-Type", "text/html")])
    return [b"<html><body><p id='msg'>hello from wsgi</p></body></html>"]


async def test_goto_via_wsgi_transport() -> None:
    page = AsyncPage(Browser(httpx_transport=WSGITransport(app=_app)))
    try:
        await page.goto("http://testserver/")
        el = page.find("#msg")
        assert el is not None
        assert el.text == "hello from wsgi"
    finally:
        page.close()


async def test_wsgi_body_is_read_and_closed_on_the_executor() -> None:
    # WSGI apps run cleanup in the body's close() (Django's request_finished), and a
    # streaming body runs app code on every step, so both belong on the caller's thread.
    executor = ThreadPoolExecutor(max_workers=1)
    app_thread = executor.submit(threading.get_ident).result()
    seen: list[tuple[str, int]] = []

    class Body:
        def __iter__(self):
            seen.append(("iter", threading.get_ident()))
            yield b"<p id='msg'>streamed</p>"

        def close(self) -> None:
            seen.append(("close", threading.get_ident()))

    def app(environ: WSGIEnvironment, start_response: StartResponse):
        start_response("200 OK", [("Content-Type", "text/html")])
        return Body()

    page = AsyncPage(Browser(httpx_transport=WSGITransport(app=app, executor=executor)))
    try:
        await page.goto("http://testserver/")
        assert page.find("#msg") is not None
        assert seen == [("iter", app_thread), ("close", app_thread)]
    finally:
        page.close()
        executor.shutdown()
