from __future__ import annotations

import asyncio
import json
import weakref
from contextlib import AsyncExitStack
from pathlib import Path
from typing import TYPE_CHECKING, Generic, Self, TypeVar, cast

import httpx2 as httpx

from miniclient.runtime import Runtime, open_runtime


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


def _dispatch_js(handle: int, event: str, event_init: dict | None, js: str = "") -> str:
    """Dispatch a DOM event, wrapped in a Promise that resolves once the page
    settles. Delegates the wait/settle logic to the shared JS helper also used
    by __zzz_submit (see submit.js). If `js` is given, runs it against `el`
    first (used by fill()/type() to set the value).
    """
    event_cls = _event_class(event)
    init_json = json.dumps(event_init) if event_init else "{bubbles: true, cancelable: true}"
    return f"""
        __zzz_await_settle({handle}, el => {{
          {js};
          el.dispatchEvent(new {event_cls}({json.dumps(event)}, {init_json}));
        }});"""


_E = TypeVar("_E", bound="AsyncElementBase", default="AsyncElement")


class _FindMixin(Generic[_E]):
    """Shared find()/find_all(), scoped to whatever `_root_js` resolves to in JS."""

    _runtime: Runtime
    _root_js: str
    _element_cls: type[_E]

    if TYPE_CHECKING:
        # Real implementation always comes from a sibling in the MRO
        # (AsyncElementBase for AsyncElement/Element, or AsyncPage's own
        # method). Declared here only so find()/find_all() type-check; a real
        # `def` would shadow the sibling's implementation depending on base
        # order, since Python MRO doesn't know one side is "just a stub".
        def _eval(self, expr: str) -> object: ...

    def _make_element(self, handle: int, tag: str) -> _E:
        """Wrap a matched (handle, tagName) pair in the right Element subclass."""

        cls = form_classes[self._element_cls] if tag == "FORM" else self._element_cls
        return cls(handle, self._runtime)  # type: ignore[bad-return]

    def find(self, selector: str, text: str | None = None) -> _E | None:
        """Return the first matching element, or None if not found.

        If text is given, only consider elements whose textContent contains it.
        """
        # "match"/"matches", not "el": _root_js can itself be "el" (element
        # scope), and redeclaring `const el` in the same block as that would
        # shadow it into the temporal dead zone (ReferenceError) before this
        # code even runs.
        js = f"""
        (() => {{
          const text = {json.dumps(text)};
          const matches = Array.from({self._root_js}.querySelectorAll({json.dumps(selector)}));
          const match =
            text === null ? matches[0] : matches.find(m => m.textContent.includes(text));
          return match ? [__zzz_ref(match), match.tagName] : [null, null];
        }})();
        """
        handle, tag = self._eval(js)  # type: ignore[misc]
        if handle is None:
            return None
        return self._make_element(handle, tag)

    def find_all(self, selector: str, text: str | None = None) -> list[_E]:
        """Return all matching elements.

        If text is given, only include elements whose textContent contains it.
        """
        js = f"""\
            (() => {{
              const text = {json.dumps(text)};
              return Array.from({self._root_js}.querySelectorAll({json.dumps(selector)}))
                .filter(m => text === null || m.textContent.includes(text))
                .map(m => [__zzz_ref(m), m.tagName]);
            }})();
        """
        return [self._make_element(handle, tag) for handle, tag in self._eval(js)]  # type: ignore[misc]


class AsyncElementBase(Generic[_E]):
    """Shared element implementation (queries, form/input, interactions),
    used by both the async and sync facades. `AsyncElement`/`Element` each
    combine this with `_FindMixin[_E]` bound to their own concrete class, so
    find()/find_all()/parent stay in the right facade.

    Identified by an opaque handle (assigned by the JS-side element registry),
    not by the selector used to locate it — it stays valid across DOM changes
    as long as the underlying node remains connected to the document.
    """

    _root_js = "el"

    def __init__(
        self,
        handle: int,
        runtime: Runtime,
    ) -> None:
        self.handle = handle
        self._runtime = runtime

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

    if TYPE_CHECKING:
        # Real implementation always comes from _FindMixin, the sibling this
        # class is combined with in AsyncElement/Element. See the matching
        # note on _FindMixin._eval for why this isn't a real `def`.
        def _make_element(self, handle: int, tag: str) -> _E: ...

    @property
    def parent(self) -> _E | None:
        """Return the parent element, or None if it has no parent (e.g.
        it's the root <html> element, or has been removed from the DOM).
        """
        js = """
        (() => {
          const p = el.parentElement;
          return p ? [__zzz_ref(p), p.tagName] : [null, null];
        })();
        """
        handle, tag = self._eval(js)  # type: ignore[misc]
        if handle is None:
            return None
        return self._make_element(handle, tag)

    # --- Form / Input ---

    async def fill(self, value: str) -> None:
        """Set the element's value, dispatch `change`, and wait for the page to settle."""
        js = _dispatch_js(
            self.handle, "change", {"bubbles": True}, js=f"el.value = {json.dumps(value)}"
        )
        await self._runtime.eval_async(js)

    async def type(self, value: str) -> None:
        """Set the element's value, dispatch `input`, and wait for the page to settle."""
        js = _dispatch_js(self.handle, "input", None, js=f"el.value = {json.dumps(value)}")
        await self._runtime.eval_async(js)

    # --- Interactions ---

    async def click(self) -> None:
        """Dispatch a click MouseEvent and wait for the page to settle."""
        await self.trigger("click")

    async def trigger(self, event: str, event_init: dict | None = None) -> None:
        """Dispatch a DOM event and wait for the page to settle."""
        js = _dispatch_js(self.handle, event, event_init)
        await self._runtime.eval_async(js)

    # --- Internal ---

    def _eval(self, expr: str) -> object:
        """Evaluate `expr` with `el` bound to the selected element.

        `expr` may be one statement or several (`;`-separated);
        the result is the completion value of the last one, same as a normal script.
        """
        js = f"""\
            const el = __zzz_deref({self.handle});
            if (!el) throw new Error('Element not found (handle {self.handle})');
            {expr.strip()};
        """
        # Runs via indirect `eval()` so each call's `const el`
        # gets its own scope, instead of colliding with earlier calls' `el` in
        # a shared top-level scope.
        return self._runtime.eval(f"(0, eval)({json.dumps(js)})")


class AsyncElement(_FindMixin["AsyncElement"], AsyncElementBase["AsyncElement"]):
    """Async-facade element: `AsyncElementBase` + find()/find_all() that
    return `AsyncElement`."""


class AsyncFormElementBase:
    """Shared <form>-only requestSubmit(), used by both `AsyncFormElement`
    and `FormElement`. Kept separate from `AsyncElementBase` so combining it
    with `Element` (which is not an `AsyncElement` subclass) doesn't create
    a diamond with two different `AsyncElementBase[_E]` type arguments.
    """

    handle: int
    _runtime: Runtime

    async def requestSubmit(self) -> None:
        """Submit this form and wait for it to settle.

        If a script (e.g. htmx) intercepts the submit, waits for the page to settle.
        Otherwise, performs the form's native GET/POST navigation and reloads the page.
        """
        await self._runtime.eval_async(f"__zzz_submit({self.handle})")


class AsyncFormElement(AsyncElement, AsyncFormElementBase):
    """A <form> element. Exposes requestSubmit(), which is form-only."""


class AsyncPage(_FindMixin[_E], Generic[_E]):
    _root_js = "document"
    _element_cls: type[_E] = AsyncElement  # type: ignore[bad-assignment]

    def __init__(
        self,
        httpx_transport: httpx.AsyncBaseTransport | None = None,
        mounts: dict[str, Path] | None = None,
        v8_snapshot: bytes | None = None,
        *,
        runtime: Runtime | None = None,
    ) -> None:
        self._httpx_transport = httpx_transport
        self._mounts = mounts
        self._v8_snapshot = v8_snapshot
        self._runtime = runtime  # type: ignore[assignment]
        self._stack: AsyncExitStack | None = None

    @property
    def runtime(self) -> Runtime:
        assert self._runtime is not None, "AsyncPage not built yet — use `await` or `async with`"
        return self._runtime

    async def _build(self) -> Self:
        if self._runtime is None:
            # open_runtime() pools one httpx client for every fetch this browser makes
            # for the rest of its life; the exit stack lets us hold that context open
            # across arbitrary later calls and unwind it (client + runtime) in aclose().
            self._stack = AsyncExitStack()  # type: ignore[assignment]
            self._runtime = await self._stack.enter_async_context(
                open_runtime(
                    v8_snapshot=self._v8_snapshot,
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

    def _eval(self, expr: str) -> object:
        return self.runtime.eval(expr)

    @property
    def url(self) -> str:
        """The current document URL (`location.href`)."""
        return cast(str, self._eval("location.href"))

    # --- Page operations ---

    async def goto(self, url: str) -> None:
        """Fetch url, load the full document, and process htmx."""
        await self.runtime.eval_async(f"__zzz_fetch_and_load({json.dumps(url)})")

    async def load(self, html: str) -> None:
        """Load HTML into the document and initialize htmx."""
        await self.runtime.eval_async(f"__document_write({json.dumps(html)})")

    def close(self) -> None:
        self.runtime.close()

    async def aclose(self) -> None:
        """Like close(), but also awaits the shared httpx client's teardown."""
        self.close()
        if self._stack is not None:
            await self._stack.aclose()

    def __enter__(self) -> Self:
        assert self._runtime is not None, "AsyncPage not built yet — await it or use `async with`"
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    async def __aenter__(self) -> Self:
        return await self._build()

    async def __aexit__(self, *_: object) -> None:
        await self.aclose()


# --- Synchronous facade ---
#
# Page needs *a* loop to drive Runtime's async-facing calls (eval_async,
# fill, click, ...) through `loop.run_until_complete()`. It uses a private
# event loop on the caller's own thread rather than spinning up a separate
# thread, purely because asyncio can't nest `run_until_complete` on a thread
# that already has a running loop -- Page.__init__ checks for that and raises
# up front instead of letting the violation surface later as a cryptic
# RuntimeError. Async callers should use AsyncPage instead.
#
# Runtime itself has no thread affinity: it owns a dedicated OS thread
# internally and every call crosses to it over a channel, so this has
# nothing to do with which thread drives the loop above.

# Element construction needs to know which loop drives its Runtime; looked
# up explicitly by Runtime identity since it's not otherwise passed through
# the shared _FindMixin._make_element() construction path.
_loop_by_runtime: weakref.WeakKeyDictionary[Runtime, asyncio.AbstractEventLoop] = (
    weakref.WeakKeyDictionary()
)


class Element(_FindMixin["Element"], AsyncElementBase["Element"]):
    """Sync-facade element: `AsyncElementBase` + find()/find_all() that
    return `Element`. Async-facing methods (trigger/fill) run to completion
    on the same event loop as the owning `Page`; reads call straight
    through to the inherited `AsyncElementBase`/`_FindMixin` implementation.
    """

    def __init__(
        self,
        handle: int,
        runtime: Runtime,
    ) -> None:
        super().__init__(handle, runtime)
        self._loop = _loop_by_runtime[runtime]

    def click(self) -> None:  # type: ignore[override]
        """Dispatch a click MouseEvent and wait for the page to settle."""
        self.trigger("click")

    def trigger(self, event: str, event_init: dict | None = None) -> None:  # type: ignore[override]
        """Dispatch a DOM event and wait for the page to settle."""
        self._loop.run_until_complete(AsyncElementBase.trigger(self, event, event_init))

    def fill(self, value: str) -> None:  # type: ignore[override]
        """Set the element's value, dispatch `change`, and wait for the page to settle."""
        self._loop.run_until_complete(AsyncElementBase.fill(self, value))

    def type(self, value: str) -> None:  # type: ignore[override]
        """Set the element's value, dispatch `input`, and wait for the page to settle."""
        self._loop.run_until_complete(AsyncElementBase.type(self, value))


class FormElement(Element, AsyncFormElementBase):
    """A <form> element. Exposes requestSubmit(), which is form-only."""

    def requestSubmit(self) -> None:  # type: ignore[override]
        """Submit this form and wait for it to settle.

        If a script (e.g. htmx) intercepts the submit, waits for the page to settle.
        Otherwise, performs the form's native GET/POST navigation and reloads the page.
        """
        self._loop.run_until_complete(AsyncFormElementBase.requestSubmit(self))


form_classes = {AsyncElement: AsyncFormElement, Element: FormElement}
AsyncElement._element_cls = AsyncElement
Element._element_cls = Element


class Page:
    """Synchronous facade over AsyncPage, running on a persistent event
    loop on the caller's own thread (see the "Synchronous facade" note
    above).

    Must not be constructed from a thread that already has a running event
    loop (e.g. inside `async def` code) — use AsyncPage there instead.
    """

    def __init__(
        self,
        httpx_transport: httpx.AsyncBaseTransport | None = None,
        mounts: dict[str, Path] | None = None,
        v8_snapshot: bytes | None = None,
    ) -> None:
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
        self._async: AsyncPage[Element] = AsyncPage(
            httpx_transport=httpx_transport,
            mounts=mounts,
            v8_snapshot=v8_snapshot,
        )
        self._async._element_cls = Element
        self._loop.run_until_complete(self._async._build())
        _loop_by_runtime[self._async.runtime] = self._loop

    def eval(self, code: str) -> object:
        """Evaluate arbitrary JavaScript and return the result."""
        return self._async.runtime.eval(code)

    @property
    def url(self) -> str:
        """The current document URL (`location.href`)."""
        return self._async.url

    def find(self, selector: str, text: str | None = None) -> Element | None:
        """Return the first matching element, or None if not found."""
        return self._async.find(selector, text)

    def find_all(self, selector: str, text: str | None = None) -> list[Element]:
        """Return all matching elements."""
        return self._async.find_all(selector, text)

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
