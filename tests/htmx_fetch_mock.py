import asyncio
import re
from dataclasses import dataclass, field
from urllib.parse import urlparse

import httpx
from jsrun import Runtime


@dataclass
class _MockEntry:
    method: str
    pattern: re.Pattern
    body: str
    status: int
    headers: dict
    once: bool = False
    used: bool = False
    is_error: bool = False
    error_msg: str = ""


@dataclass
class _SeqEntry:
    method: str
    pattern: re.Pattern
    body: str
    status: int
    headers: dict
    release_queue: asyncio.Queue = field(default_factory=asyncio.Queue)


class _MockTransport(httpx.AsyncBaseTransport):
    def __init__(self, owner: "HttpxFetchMock") -> None:
        self._owner = owner

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        return self._owner._dispatch(request)


class HttpxFetchMock:
    """
    Routes htmx fetch calls through the Python httpx client using a custom
    transport, proving htmx is integrated with httpx.

    Usage:
        mock = HttpxFetchMock()
        runtime = await build_browser(
            before_fetch=mock.before_fetch,
            httpx_transport=mock.transport,
        )
        mock.install(runtime)
    """

    def __init__(self) -> None:
        self._entries: list[_MockEntry] = []
        self._seq_entries: dict[int, _SeqEntry] = {}
        self._next_seq_id = 0
        self.transport = _MockTransport(self)

    def install(self, runtime: Runtime) -> None:
        register_id = runtime.register_op("fetch_mock_register", self._op_register)
        reset_id = runtime.register_op("fetch_mock_reset", self._op_reset)
        register_seq_id = runtime.register_op("fetch_mock_register_seq", self._op_register_seq)
        next_id = runtime.register_op("fetch_mock_next", self._op_next, mode="async")
        runtime.eval(
            f"globalThis.__FM_REGISTER__ = {register_id};"
            f"globalThis.__FM_RESET__ = {reset_id};"
            f"globalThis.__FM_REGISTER_SEQ__ = {register_seq_id};"
            f"globalThis.__FM_NEXT__ = {next_id};"
        )

    def _op_register(self, req: dict) -> dict:
        self._entries.append(
            _MockEntry(
                method=req["method"].upper(),
                pattern=re.compile(req["urlPattern"]),
                body=req.get("body", ""),
                status=req.get("status", 200),
                headers=req.get("headers", {}),
                once=req.get("once", False),
                is_error=req.get("is_error", False),
                error_msg=req.get("error_msg", ""),
            )
        )
        return {}

    def _op_reset(self, req: dict) -> dict:
        self._entries.clear()
        self._seq_entries.clear()
        return {}

    def _op_register_seq(self, req: dict) -> int:
        seq_id = self._next_seq_id
        self._next_seq_id += 1
        self._seq_entries[seq_id] = _SeqEntry(
            method=req["method"].upper(),
            pattern=re.compile(req["urlPattern"]),
            body=req.get("body", ""),
            status=req.get("status", 200),
            headers=req.get("headers", {}),
        )
        return seq_id

    async def _op_next(self, req: dict) -> dict:
        seq_id = req["seq_id"]
        if entry := self._seq_entries.get(seq_id):
            await entry.release_queue.put(True)
        return {}

    async def before_fetch(self, req: dict) -> None:
        """Await the release gate for sequential mock entries before calling httpx."""
        url = req["url"]
        method = req["method"].upper()
        for entry in self._seq_entries.values():
            if entry.method == method and entry.pattern.search(url):
                await entry.release_queue.get()
                return

    @staticmethod
    def _url_matches(pattern: re.Pattern, url: str) -> bool:
        if pattern.search(url):
            return True
        # Also try relative forms so patterns like /^$/ match the base URL and
        # patterns like /\/test$/ match both the full URL and the path.
        parsed = urlparse(url)
        base = f"{parsed.scheme}://{parsed.netloc}"
        relative = url.removeprefix(base)  # e.g. "/" or "/test"
        return bool(pattern.search(relative) or pattern.search(relative.lstrip("/")))

    def _dispatch(self, request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        method = request.method

        for entry in self._seq_entries.values():
            if entry.method == method and self._url_matches(entry.pattern, url):
                return httpx.Response(entry.status, text=entry.body, headers=entry.headers)

        for entry in reversed(self._entries):
            if entry.method == method and self._url_matches(entry.pattern, url):
                if entry.once and entry.used:
                    continue
                if entry.once:
                    entry.used = True
                if entry.is_error:
                    raise httpx.NetworkError(entry.error_msg)
                return httpx.Response(entry.status, text=entry.body, headers=entry.headers)

        raise httpx.NetworkError(f"No mock configured for {method} {url}")
