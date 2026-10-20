import asyncio
from concurrent.futures import Executor
from typing import cast
from wsgiref.types import WSGIApplication

import httpx2 as httpx


class WSGITransport(httpx.AsyncBaseTransport):
    """Runs a sync WSGI app as an async httpx transport via a thread executor.

    Lets `Page`/`AsyncPage`'s `httpx_transport` param test a WSGI app
    (Flask, Django's `app.wsgi`, ...) in-process, the same way `ASGITransport`
    does for ASGI apps.
    """

    def __init__(self, app: WSGIApplication, executor: Executor | None = None) -> None:
        """`executor`: where each request's WSGI call runs.

        Defaults to asyncio's own default executor (a thread pool, no
        particular thread per call). Pass a caller-owned `Executor` (e.g. a
        `ThreadPoolExecutor(max_workers=1)`) to pin every request to a
        specific thread instead — needed if the WSGI app relies on
        thread-affine state (e.g. Django's SQLite test connections shared
        with another thread).
        """
        self._sync = httpx.WSGITransport(app=app)
        self._executor = executor

    def _handle(self, request: httpx.Request) -> httpx.Response:
        response = self._sync.handle_request(request)
        stream = cast(httpx.SyncByteStream, response.stream)
        # The body's close() is where WSGI apps clean up (Django's request_finished), and a
        # streaming body runs app code, so both stay on the executor thread.
        try:
            content = b"".join(stream)
        finally:
            stream.close()
        return httpx.Response(response.status_code, headers=response.headers, content=content)

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        loop = asyncio.get_running_loop()
        return await loop.run_in_executor(self._executor, self._handle, request)
