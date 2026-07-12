from __future__ import annotations

import json
from pathlib import Path

import httpx2 as httpx
from jsrun import Runtime

from miniclient.runtime import build_runtime


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


class Element:
    """Represents a DOM element found via Browser.find() or Browser.find_all().

    Identified by an opaque handle (assigned by the JS-side element registry),
    not by the selector used to locate it — it stays valid across DOM changes
    as long as the underlying node remains connected to the document.
    """

    def __init__(self, handle: int, runtime: Runtime) -> None:
        self.handle = handle
        self.runtime = runtime

    # --- Queries ---

    def html(self) -> str:
        """Return outerHTML of the element."""
        return self._eval("el.outerHTML")  # type: ignore[return-value]

    def innerHTML(self) -> str:
        """Return innerHTML of the element."""
        return self._eval("el.innerHTML")  # type: ignore[return-value]

    def text(self) -> str:
        """Return textContent of the element."""
        return self._eval("el.textContent")  # type: ignore[return-value]

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


class FormElement(Element):
    """A <form> element. Exposes requestSubmit(), which is form-only."""

    async def requestSubmit(self) -> None:
        """Submit this form and wait for it to settle.

        If htmx handles the submission, waits for htmx to settle.
        If the form is not htmx-wired, performs a plain fetch and reloads the page.
        """
        await self.runtime.eval_async(f"__zzz_submit({self.handle})")


def _element_from(handle: int, tag: str, runtime: Runtime) -> Element:
    """Pick the right Element subclass for a matched node."""
    return FormElement(handle, runtime) if tag == "FORM" else Element(handle, runtime)


class Browser:
    def __init__(self, runtime: Runtime) -> None:
        self.runtime = runtime

    @classmethod
    async def create(
        cls,
        url: str = "http://localhost/",
        httpx_transport: httpx.AsyncBaseTransport | None = None,
        mounts: dict[str, Path] | None = None,
        snapshot: bytes | None = None,
    ) -> Browser:
        return cls(
            await build_runtime(
                url,
                snapshot=snapshot,
                httpx_transport=httpx_transport,
                virtual_servers=[
                    {"url": mount_url, "directory": str(directory)}
                    for mount_url, directory in (mounts or {}).items()
                ],
            )
        )

    # --- Element queries ---

    def find(self, selector: str) -> Element | None:
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
        return _element_from(handle, tag, self.runtime)

    def find_all(self, selector: str) -> list[Element]:
        """Return all matching elements."""
        js = f"""\
            Array.from(document.querySelectorAll({json.dumps(selector)}), el => [
              __zzz_ref(el),
              el.tagName,
            ])
        """
        return [_element_from(handle, tag, self.runtime) for handle, tag in self.runtime.eval(js)]

    # --- Page operations ---

    async def goto(self, url: str) -> None:
        """Fetch url, load the full document, and process htmx."""
        await self.runtime.eval_async(f"__zzz_fetch_and_load({json.dumps(url)})")

    async def load(self, html: str) -> None:
        """Load HTML into the document and initialize htmx."""
        self.runtime.eval(f"__document_write({json.dumps(html)})")

    def close(self) -> None:
        self.runtime.close()

    def __enter__(self) -> Browser:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    async def __aenter__(self) -> Browser:
        return self

    async def __aexit__(self, *_: object) -> None:
        self.close()
