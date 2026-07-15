from __future__ import annotations

import asyncio
import json
import threading
import weakref
from collections.abc import Callable, Coroutine
from contextlib import AsyncExitStack
from pathlib import Path
from typing import Generic, Self, TypeVar

import httpx2 as httpx
from jsrun import Runtime

from miniclient.runtime import open_runtime


def _event_class(event_type: str) -> str:
    """Map common event types to their proper DOM event constructors."""
    mapping = {
        "click": "MouseEvent",
        "dblclick": "MouseEvent",
        "mousedown": "MouseEvent",
        "mouseup": "MouseEvent",
        "mousemove": "MouseEvent",
        "mouseover": "MouseEvent",
        "mouseout": "MouseEvent",
        "mouseenter": "MouseEvent",
        "mouseleave": "MouseEvent",
        "keydown": "KeyboardEvent",
        "keyup": "KeyboardEvent",
        "keypress": "KeyboardEvent",
        "focus": "FocusEvent",
        "blur": "FocusEvent",
        "input": "InputEvent",
        "change": "Event",
        "submit": "SubmitEvent",
        "reset": "Event",
        "scroll": "Event",
        "resize": "Event",
        "load": "Event",
        "error": "Event",
    }
    return mapping.get(event_type, "Event")


def _dispatch_js(handle: int, event: str, event_init: dict | None) -> str:
    """Dispatch a DOM event, wrapped in a Promise that resolves after htmx
    settles (or immediately if no request fires). Delegates the wait/settle
    logic to the shared JS helper also used by __zzz_submit (see submit.js).
    """
    event_cls = _event_class(event)
    init_json = json.dumps(event_init) if event_init else "{bubbles: true}"
    return f"""
        __zzz_await_htmx({handle}, el => {{
          el.dispatchEvent(new {event_cls}({json.dumps(event)}, {init_json}));
        }});"""


class AsyncElement:
    """Represents a DOM element found via Browser.find() or Browser.find_all().

    Identified by an opaque handle (assigned by the JS-side element registry),
    not by the selector used to locate it — it stays valid across DOM changes
    as long as the underlying node remains connected to the document.
    """

    def __init__(self, handle: int, runtime: Runtime) -> None:
        self.handle = handle
        self.runtime = runtime

    # --- Queries ---

    @property
    def html(self) -> str:
        """Return outerHTML of the element."""
        return str(self._eval("el.outerHTML"))

    @property
    def innerHTML(self) -> str:
        """Return innerHTML of the element."""
        return str(self._eval("el.innerHTML"))

    @property
    def text(self) -> str:
        """Return textContent of the element."""
        return str(self._eval("el.textContent"))

    def attr(self, name: str) -> str | None:
        """Return an attribute value, or None if absent."""
        return self._eval(f"el.getAttribute({json.dumps(name)})")  # type: ignore[return-value]

    # --- Form / Input ---

    def fill(self, value: str) -> None:
        """Set the element's value (for input, textarea, select)."""
        self._eval(f"el.value = {json.dumps(value)}")

    # --- Interactions ---

    async def click(self) -> None:
        """Dispatch a click MouseEvent and wait for htmx to settle if needed."""
        await self.trigger("click")

    async def trigger(self, event: str, event_init: dict | None = None) -> None:
        """Dispatch a DOM event and wait for htmx to settle."""
        js = _dispatch_js(self.handle, event, event_init)
        await self.runtime.eval_async(js)

    # --- Internal ---

    def _eval(self, expr: str) -> object:
        """Evaluate an expression with `el` bound to the selected element."""
        js = f"""
        (() => {{
          const el = __zzz_deref({self.handle});
          if (!el) throw new Error('Element not found (handle {self.handle})');
          return {expr};
        }})();
        """
        return self.runtime.eval(js)


class AsyncFormElement(AsyncElement):
    """A <form> element. Exposes requestSubmit(), which is form-only."""

    async def requestSubmit(self) -> None:
        """Submit this form and wait for it to settle.

        If htmx handles the submission, waits for htmx to settle.
        If the form is not htmx-wired, performs a plain fetch and reloads the page.
        """
        await self.runtime.eval_async(f"__zzz_submit({self.handle})")


_E = TypeVar("_E", bound="AsyncElement", default="AsyncElement")
_T = TypeVar("_T")


def _element_from(
    handle: int,
    tag: str,
    runtime: Runtime,
    element_cls: type[_E],
    form_cls: type[_E],
) -> _E:
    """Pick the right Element subclass for a matched node."""
    return form_cls(handle, runtime) if tag == "FORM" else element_cls(handle, runtime)


class AsyncBrowser(Generic[_E]):
    def __init__(
        self,
        httpx_transport: httpx.AsyncBaseTransport | None = None,
        mounts: dict[str, Path] | None = None,
        snapshot: bytes | None = None,
        *,
        runtime: Runtime | None = None,
        element_cls: type[_E] = AsyncElement,
        form_element_cls: type[_E] = AsyncFormElement,
    ) -> None:
        self._httpx_transport = httpx_transport
        self._mounts = mounts
        self._snapshot = snapshot
        self._runtime = runtime
        self._element_cls = element_cls
        self._form_element_cls = form_element_cls
        self._stack: AsyncExitStack | None = None

    @property
    def runtime(self) -> Runtime:
        assert self._runtime is not None, "AsyncBrowser not built yet — use `await` or `async with`"
        return self._runtime

    async def _build(self) -> Self:
        if self._runtime is None:
            # open_runtime() pools one httpx client for every fetch this browser makes
            # for the rest of its life; the exit stack lets us hold that context open
            # across arbitrary later calls and unwind it (client + runtime) in aclose().
            self._stack = AsyncExitStack()
            self._runtime = await self._stack.enter_async_context(
                open_runtime(
                    snapshot=self._snapshot,
                    httpx_transport=self._httpx_transport,
                    virtual_servers=[
                        {"url": mount_url, "directory": str(directory)}
                        for mount_url, directory in (self._mounts or {}).items()
                    ],
                )
            )
        return self

    def __await__(self):
        return self._build().__await__()

    # --- Element queries ---

    def find(self, selector: str) -> _E | None:
        """Return the first matching element, or None if not found."""
        js = f"""
        (() => {{
          const el = document.querySelector({json.dumps(selector)});
          return el ? [__zzz_ref(el), el.tagName] : [null, null];
        }})();
        """
        handle, tag = self.runtime.eval(js)
        if handle is None:
            return None
        return _element_from(handle, tag, self.runtime, self._element_cls, self._form_element_cls)

    def find_all(self, selector: str) -> list[_E]:
        """Return all matching elements."""
        js = f"""\
            Array.from(document.querySelectorAll({json.dumps(selector)}), el => [
              __zzz_ref(el),
              el.tagName,
            ])
        """
        return [
            _element_from(handle, tag, self.runtime, self._element_cls, self._form_element_cls)
            for handle, tag in self.runtime.eval(js)
        ]

    # --- Page operations ---

    async def goto(self, url: str) -> None:
        """Fetch url, load the full document, and process htmx."""
        await self.runtime.eval_async(f"__zzz_fetch_and_load({json.dumps(url)})")

    async def load(self, html: str) -> None:
        """Load HTML into the document and initialize htmx."""
        self.runtime.eval(f"__document_write({json.dumps(html)})")

    def close(self) -> None:
        self.runtime.close()

    async def aclose(self) -> None:
        """Like close(), but also awaits the shared httpx client's teardown."""
        if self._stack is not None:
            await self._stack.aclose()
        else:
            self.close()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    async def __aenter__(self) -> Self:
        return await self._build()

    async def __aexit__(self, *_: object) -> None:
        await self.aclose()


# --- Synchronous facade ---
#
# jsrun's Runtime is not thread-safe: every call must happen on the thread
# that created it. _BackgroundLoop owns one dedicated thread + event loop and
# routes every runtime-touching call through it — including reads that are
# plain synchronous methods on AsyncElement/AsyncBrowser (find, html, text,
# ...) — since those still call self.runtime.eval() directly.

_bridge_loop: threading.local = threading.local()


async def _call(fn: Callable[[], _T]) -> _T:
    return fn()


class _BackgroundLoop:
    """Owns one asyncio event loop running in a dedicated background thread."""

    def __init__(self) -> None:
        self._loop = asyncio.new_event_loop()

        def _run() -> None:
            _bridge_loop.loop = self
            self._loop.run_forever()

        self._thread = threading.Thread(target=_run, daemon=True)
        self._thread.start()

    def run(self, coro: Coroutine[object, object, _T]) -> _T:
        return asyncio.run_coroutine_threadsafe(coro, self._loop).result()

    def run_sync(self, fn: Callable[[], _T]) -> _T:
        return self.run(_call(fn))

    def close(self) -> None:
        self._loop.call_soon_threadsafe(self._loop.stop)
        self._thread.join()
        self._loop.close()


class Element(AsyncElement):
    """Synchronous facade over AsyncElement.

    Every method (including the ones that are plain sync on AsyncElement)
    routes through the background thread that owns the Runtime.
    """

    def __init__(self, handle: int, runtime: Runtime) -> None:
        super().__init__(handle, runtime)
        self._loop: _BackgroundLoop = _bridge_loop.loop

    def click(self) -> None:  # type: ignore[override]
        """Dispatch a click MouseEvent and wait for htmx to settle if needed."""
        self.trigger("click")

    def trigger(self, event: str, event_init: dict | None = None) -> None:  # type: ignore[override]
        """Dispatch a DOM event and wait for htmx to settle."""
        self._loop.run(AsyncElement.trigger(self, event, event_init))

    def _eval(self, expr: str) -> object:
        return self._loop.run_sync(lambda: AsyncElement._eval(self, expr))


class FormElement(Element, AsyncFormElement):
    """A <form> element. Exposes requestSubmit(), which is form-only."""

    def requestSubmit(self) -> None:  # type: ignore[override]
        """Submit this form and wait for it to settle.

        If htmx handles the submission, waits for htmx to settle.
        If the form is not htmx-wired, performs a plain fetch and reloads the page.
        """
        self._loop.run(AsyncFormElement.requestSubmit(self))


class Browser:
    """Synchronous facade over AsyncBrowser, backed by a dedicated background thread.

    jsrun's Runtime panics (at the Rust level, uncatchable-looking but not
    fatal to the process) if it is ever garbage-collected on a thread other
    than the one that created it — not just called from one. close() defuses
    this by clearing every reference it knows about (its own and every
    Element/FormElement it has ever handed out) while still running on the
    background thread, so a caller holding onto a stale Element after close()
    can't trigger the panic when that Element is eventually collected.
    """

    def __init__(
        self,
        httpx_transport: httpx.AsyncBaseTransport | None = None,
        mounts: dict[str, Path] | None = None,
        snapshot: bytes | None = None,
    ) -> None:
        self._closed = False
        self._elements: weakref.WeakSet[Element] = weakref.WeakSet()
        self._loop = _BackgroundLoop()
        self._async: AsyncBrowser[Element] = AsyncBrowser(
            httpx_transport=httpx_transport,
            mounts=mounts,
            snapshot=snapshot,
            element_cls=Element,
            form_element_cls=FormElement,
        )
        self._loop.run(self._async._build())

    def eval(self, code: str) -> object:
        """Evaluate arbitrary JavaScript and return the result.

        Unlike AsyncBrowser, Browser does not expose a `.runtime` property:
        the raw Runtime is not thread-safe, so any direct use of it from the
        caller's thread would risk the same cross-thread panic close()
        otherwise defuses. Use this method (or find()/goto()/load()) instead.
        """
        return self._loop.run_sync(lambda: self._async.runtime.eval(code))

    def find(self, selector: str) -> Element | None:
        """Return the first matching element, or None if not found."""
        el = self._loop.run_sync(lambda: self._async.find(selector))
        if el is not None:
            self._elements.add(el)
        return el

    def find_all(self, selector: str) -> list[Element]:
        """Return all matching elements."""
        els = self._loop.run_sync(lambda: self._async.find_all(selector))
        self._elements.update(els)
        return els

    def goto(self, url: str) -> None:
        """Fetch url, load the full document, and process htmx."""
        self._loop.run(self._async.goto(url))

    def load(self, html: str) -> None:
        """Load HTML into the document and initialize htmx."""
        self._loop.run(self._async.load(html))

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True

        async def _shutdown() -> None:
            await self._async.aclose()
            self._async._runtime = None
            for el in list(self._elements):
                el.runtime = None  # type: ignore[assignment]

        try:
            self._loop.run(_shutdown())
        finally:
            self._loop.close()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def __del__(self) -> None:
        # Safety net for a Browser that was never explicitly closed — best
        # effort, since __del__ ordering/timing at interpreter shutdown isn't
        # guaranteed. Never let cleanup itself raise from a finalizer.
        try:
            self.close()
        except Exception:  # pragma: no cover
            pass
