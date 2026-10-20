import asyncio

import httpx2 as httpx
import pytest

from miniclient.fetch import _make_fetch_op, new_client

# The abort op runs on the page's own thread, so it can land before or during the request
# the loop is about to send.


class _Server(httpx.AsyncBaseTransport):
    def __init__(self) -> None:
        self.received = asyncio.Event()
        self.requests: list[httpx.Request] = []

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        self.received.set()
        await asyncio.Event().wait()  # never answers
        raise AssertionError("unreachable")


_REQUEST = {"id": "1", "method": "POST", "url": "http://t/"}


async def test_abort_before_the_request_starts_never_sends_it() -> None:
    server = _Server()
    fetch, abort = _make_fetch_op(new_client(server))
    abort("1")
    with pytest.raises(RuntimeError, match="fetch aborted"):
        await asyncio.wait_for(fetch(_REQUEST), 1)
    assert server.requests == []


async def test_abort_cancels_a_request_in_flight() -> None:
    server = _Server()
    fetch, abort = _make_fetch_op(new_client(server))
    task = asyncio.ensure_future(fetch(_REQUEST))
    await server.received.wait()
    abort("1")
    with pytest.raises(RuntimeError, match="fetch aborted"):
        await asyncio.wait_for(task, 1)
