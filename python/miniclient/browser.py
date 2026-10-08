from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any, Self, cast

import httpx2 as httpx

from miniclient._miniclient import (
    Browser as NativeBrowser,
    Element as NativeElement,
    Page as NativePage,
)
from miniclient.fetch import _make_fetch_op, _make_fetch_sync_op, new_client


class AsyncPage(NativePage):
    """Async-facing page: a `Page` native extended with the httpx client/fetch backend
    its ops call into. Every other method (`find`, `find_all`, `goto`, `load`, `eval`,
    `close`) is inherited straight from the native class.
    """

    def __new__(cls, browser: Browser) -> Self:
        # `type.__call__` hands both __new__ and __init__ the same call args -- the native
        # `Page.__new__` takes the `Browser` that hands it a pre-booted runtime.
        return cast(Self, super().__new__(cls, browser))

    def __init__(self, browser: Browser) -> None:
        # No `async with`: `install_host_ops` only needs the client to exist and a
        # loop running (true here, same as whenever this is itself awaited into
        # existence) -- entering the client's own async context buys nothing `send()`
        # requires (it only ever checks for CLOSED), so there's nothing to await yet.
        self._httpx_client = new_client(browser._transport)
        fetch_op, fetch_abort_op = _make_fetch_op(self._httpx_client)
        self.install_host_ops(
            fetch_op,
            fetch_abort_op,
            _make_fetch_sync_op(self._httpx_client, asyncio.get_running_loop()),
        )

    async def url(self) -> str:
        """The current document URL (`location.href`)."""
        return cast(str, await self.eval("location.href"))

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    async def aclose(self) -> None:
        """Like close(), but also awaits the shared httpx client's teardown."""
        self.close()
        await self._httpx_client.aclose()

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.aclose()


# --- Synchronous facade ---
#
# Page needs *a* loop to drive the native async-facing calls (eval, fill, click, ...)
# through `loop.run_until_complete()`. It uses a private event loop on the caller's own
# thread rather than spinning up a separate thread, purely because asyncio can't nest
# `run_until_complete` on a thread that already has a running loop -- Page.__init__
# checks for that and raises up front instead of letting the violation surface later as
# a cryptic RuntimeError. Async callers should use AsyncPage instead.
#
# The native Page/Element have no thread affinity: each owns a dedicated OS thread
# internally and every call crosses to it over a channel, so this has nothing to do
# with which thread drives the loop above.


class Element:
    """Synchronous facade over a native `Element`: queries pass straight through (the
    native getters are already synchronous), interactions run their native coroutine to
    completion on the owning `Page`'s loop.
    """

    def __init__(self, element: NativeElement, loop: asyncio.AbstractEventLoop) -> None:
        self._element = element
        self._loop = loop

    # --- Queries ---

    @property
    def html(self) -> str:
        """Return outerHTML of the element."""
        return self._element.html

    @property
    def inner_html(self) -> str:
        """Return innerHTML of the element."""
        return self._element.inner_html

    @property
    def text(self) -> str:
        """Return textContent of the element."""
        return self._element.text

    @property
    def tag_name(self) -> str:
        """Return the element's tag name, e.g. `"DIV"`."""
        return self._element.tag_name

    def attr(self, name: str) -> str | None:
        """Return an attribute value, or None if absent."""
        return self._element.attr(name)

    @property
    def parent(self) -> Element | None:
        """Return the parent element, or None if it has no parent (e.g.
        it's the root <html> element, or has been removed from the DOM).
        """
        return _wrap(self._element.parent, self._loop)

    def find(self, selector: str, text: str | None = None) -> Element | None:
        """Return the first matching descendant, or None if not found.

        If text is given, only consider elements whose textContent contains it.
        """
        return _wrap(self._element.find(selector, text), self._loop)

    def find_all(self, selector: str, text: str | None = None) -> list[Element]:
        """Return all matching descendants.

        If text is given, only include elements whose textContent contains it.
        """
        return [Element(native, self._loop) for native in self._element.find_all(selector, text)]

    # --- Form / Input ---

    def fill(self, value: str) -> None:
        """Set the element's value, dispatch `change`, and wait for the page to settle."""
        self._loop.run_until_complete(self._element.fill(value))

    def type(self, value: str) -> None:
        """Set the element's value, dispatch `input`, and wait for the page to settle."""
        self._loop.run_until_complete(self._element.type(value))

    # --- Interactions ---

    def click(self) -> None:
        """Dispatch a click MouseEvent and wait for the page to settle."""
        self._loop.run_until_complete(self._element.click())

    def trigger(self, event: str, event_init: dict[str, Any] | None = None) -> None:
        """Dispatch a DOM event and wait for the page to settle."""
        self._loop.run_until_complete(self._element.trigger(event, event_init))

    def request_submit(self) -> None:
        """Submit this `<form>` element and wait for it to settle.

        If a script (e.g. htmx) intercepts the submit, waits for the page to settle.
        Otherwise, performs the form's native GET/POST navigation and reloads the page.
        """
        self._loop.run_until_complete(self._element.request_submit())

    def eval(self, code: str) -> object:
        """Evaluate JavaScript with `this` bound to this element, returning the result."""
        return self._loop.run_until_complete(self._element.eval(code))


def _wrap(native: NativeElement | None, loop: asyncio.AbstractEventLoop) -> Element | None:
    return Element(native, loop) if native is not None else None


class Page:
    """Synchronous facade over AsyncPage, running on a persistent event
    loop on the caller's own thread (see the "Synchronous facade" note
    above).

    Must not be constructed from a thread that already has a running event
    loop (e.g. inside `async def` code) — use AsyncPage there instead.
    """

    def __init__(self, browser: Browser) -> None:
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            pass
        else:
            raise RuntimeError(
                "Page() can't be used on a thread with a running event loop "
                "(e.g. inside `async def` code) — use AsyncPage instead."
            )

        self._closed = False
        self._loop = asyncio.new_event_loop()

        async def _make_async_page() -> AsyncPage:
            # AsyncPage.__init__ needs a loop running (to call install_host_ops) --
            # running this one-line coroutine on self._loop supplies that.
            return AsyncPage(browser)

        try:
            self._async = self._loop.run_until_complete(_make_async_page())
        except BaseException:
            # Nothing to close later, so don't leak the loop.
            self._closed = True
            self._loop.close()
            raise

    def eval(self, code: str) -> object:
        """Evaluate arbitrary JavaScript and return the result.

        The result is the completion value of the last statement, same as a script.
        Declarations (`var`/`function`) attach to `globalThis` and persist across calls.
        """
        return self._loop.run_until_complete(self._async.eval(code))

    @property
    def url(self) -> str:
        """The current document URL (`location.href`)."""
        return self._loop.run_until_complete(self._async.url())

    def find(self, selector: str, text: str | None = None) -> Element | None:
        """Return the first matching element, or None if not found."""
        return _wrap(self._async.find(selector, text), self._loop)

    def find_all(self, selector: str, text: str | None = None) -> list[Element]:
        """Return all matching elements."""
        return [Element(native, self._loop) for native in self._async.find_all(selector, text)]

    def goto(self, url: str) -> None:
        """Fetch url, load the full document, and process htmx."""
        self._loop.run_until_complete(self._async.goto(url))

    def load(self, html: str) -> None:
        """Load HTML into the document and initialize htmx."""
        self._loop.run_until_complete(self._async.load(html))

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self._loop.run_until_complete(self._async.aclose())
        finally:
            self._loop.close()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def __del__(self) -> None:
        # Safety net for a Page that was never explicitly closed — best
        # effort, since __del__ ordering/timing at interpreter shutdown isn't
        # guaranteed. Never let cleanup itself raise from a finalizer.
        try:
            self.close()
        except Exception:  # pragma: no cover
            pass


class Browser(NativeBrowser):
    """Shares one transport and one set of mounts across the `Page`s created from it, keeping one
    blank runtime booted in the background so a new `Page` doesn't pay the startup cost.
    `preboot=False` boots each Page's runtime on the spot instead, saving that extra isolate.

    Each Page still gets its own event loop and httpx client, which is bound to the loop it ran on.
    """

    def __init__(
        self,
        httpx_transport: httpx.AsyncBaseTransport | None = None,
        mounts: dict[str, Path] | None = None,
        preboot: bool = True,
    ) -> None:
        self._transport = httpx_transport
