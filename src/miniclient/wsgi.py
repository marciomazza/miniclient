import asyncio
from typing import Iterable, cast
from wsgiref.types import WSGIApplication

import httpx2 as httpx


class _AsyncByteStream(httpx.AsyncByteStream):
    """Wraps a bytes body as an AsyncByteStream (required by httpx.AsyncClient)."""

    def __init__(self, data: bytes) -> None:
        self._data = data

    async def __aiter__(self):
        yield self._data

    async def aclose(self) -> None:
        pass


class WSGITransport(httpx.AsyncBaseTransport):
    """Runs a sync WSGI app as an async httpx transport via a thread executor.

    Lets `Browser`/`AsyncBrowser`'s `httpx_transport` param test a WSGI app
    (Flask, Django's `app.wsgi`, ...) in-process, the same way `ASGITransport`
    does for ASGI apps.
    """

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
