import asyncio
from concurrent.futures import Executor
from typing import Iterable, cast
from wsgiref.types import WSGIApplication

import httpx2 as httpx


class WSGITransport(httpx.AsyncBaseTransport):
    """Runs a sync WSGI app as an async httpx transport via a thread executor.

    Lets `Browser`/`AsyncBrowser`'s `httpx_transport` param test a WSGI app
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

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        loop = asyncio.get_running_loop()
        sync_response = await loop.run_in_executor(
            self._executor, self._sync.handle_request, request
        )
        return httpx.Response(
            status_code=sync_response.status_code,
            headers=sync_response.headers,
            content=b"".join(cast(Iterable[bytes], sync_response.stream)),
        )
