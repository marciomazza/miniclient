from __future__ import annotations

import json
import re
from urllib.parse import urlencode

import httpx

from htmxclient.runtime import HxRuntime, build_runtime


def _extract_body_html(html: str) -> str:
    m = re.search(r"<body[^>]*>(.*?)</body>", html, re.DOTALL | re.IGNORECASE)
    body = m.group(1) if m else html
    # happy-dom executes scripts in innerHTML (unlike real browsers); strip them to prevent
    # side-effects when navigating to pages that load scripts already present in the snapshot.
    return re.sub(r"<script\b[^>]*>.*?</script>", "", body, flags=re.DOTALL | re.IGNORECASE)


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


def _htmx_action_js(selector: str, action_js: str) -> str:
    """Wrap an element action in a Promise that resolves after htmx settles
    (or immediately if no request fires).
    """
    return f"""
    new Promise((resolve, reject) => {{
        let willRequest = false;
        document.addEventListener(
            'htmx:before:request',
            () => {{ willRequest = true; }},
            {{once: true}},
        );
        document.addEventListener(
            'htmx:after:settle',
            () => resolve(),
            {{once: true}},
        );
        document.addEventListener('htmx:error', (e) => {{
            reject(new Error(
                'htmx:error — '
                + (e.detail?.error ?? e.detail?.ctx?.status)
            ));
        }}, {{once: true}});
        const el = document.querySelector({json.dumps(selector)});
        if (!el) {{
            reject(new Error('Element not found: {selector}'));
            return;
        }}
        {action_js}
        setTimeout(() => {{ if (!willRequest) resolve(); }}, 0);
    }})
    """


def _dispatch_js(selector: str, event: str, event_init: dict | None) -> str:
    event_cls = _event_class(event)
    init_json = json.dumps(event_init) if event_init else "{bubbles: true}"
    action = f"el.dispatchEvent(new {event_cls}({json.dumps(event)}, {init_json}));"
    return _htmx_action_js(selector, action)


class Element:
    """Represents a DOM element found via Browser.find() or Browser.find_all()."""

    def __init__(self, selector: str, runtime: HxRuntime) -> None:
        self.selector = selector
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

    async def submit(self) -> None:
        """Submit the form (or the element's form) and wait for htmx to settle if needed."""
        no_form_msg = json.dumps(f"No form found for: {self.selector}")
        action = (
            f"const form = el.form ?? el.closest('form');\n"
            f"if (!form) {{ reject(new Error({no_form_msg})); return; }}\n"
            f"const submitter = el.tagName === 'BUTTON' || el.tagName === 'INPUT'"
            f" ? el : null;\n"
            f"form.dispatchEvent(new SubmitEvent('submit', "
            f"{{bubbles: true, cancelable: true, submitter}}));"
        )
        await self.runtime.eval_async(_htmx_action_js(self.selector, action))

    async def trigger(self, event: str, event_init: dict | None = None) -> None:
        """Dispatch a DOM event and wait for htmx to settle."""
        js = _dispatch_js(self.selector, event, event_init)
        await self.runtime.eval_async(js)

    # --- Internal ---

    def _eval(self, expr: str) -> object:
        """Evaluate an expression with `el` bound to the selected element."""
        js = f"""
        (() => {{
            const el = document.querySelector({json.dumps(self.selector)});
            if (!el) throw new Error('Element not found: {self.selector}');
            return {expr};
        }})()
        """
        return self.runtime.eval(js)


class Browser:
    def __init__(self, runtime: HxRuntime) -> None:
        self.runtime = runtime

    @classmethod
    async def create(
        cls,
        url: str = "http://localhost/",
        httpx_transport: httpx.AsyncBaseTransport | None = None,
    ) -> Browser:
        return cls(await build_runtime(url, httpx_transport=httpx_transport))

    # --- Element queries ---

    def find(self, selector: str) -> Element | None:
        """Return the first matching element, or None if not found."""
        js = f"document.querySelector({json.dumps(selector)}) !== null"
        if not self.runtime.eval(js):
            return None
        return Element(selector, self.runtime)

    def find_all(self, selector: str) -> list[Element]:
        """Return all matching elements."""
        js = f"""
        (() => {{
            const nodes = document.querySelectorAll({json.dumps(selector)});
            const selectors = [];
            for (let i = 0; i < nodes.length; i++) {{
                if (nodes[i].id) {{
                    selectors.push('#' + CSS.escape(nodes[i].id));
                }} else {{
                    const tag = nodes[i].tagName.toLowerCase();
                    const sameTag = nodes[i].parentNode
                        ? Array.from(nodes[i].parentNode.children)
                            .filter(c => c.tagName === nodes[i].tagName)
                        : [];
                    const idx = sameTag.indexOf(nodes[i]) + 1;
                    selectors.push(tag + ':nth-of-type(' + idx + ')');
                }}
            }}
            return selectors;
        }})()
        """
        selectors = self.runtime.eval(js)
        return [Element(sel, self.runtime) for sel in selectors]

    # --- Page operations ---

    async def goto(self, url: str) -> None:
        """Fetch url, load its body into the document, and process htmx."""
        async with httpx.AsyncClient(transport=self.runtime.httpx_transport) as client:
            response = await client.get(url)
        await self.load(_extract_body_html(response.text))

    async def submit_form(self, selector: str) -> None:
        """
        Submit the form containing selector:
        serialize its data, fetch the action URL, load the response.
        """
        info = self.runtime.eval(f"""
        (() => {{
            const el = document.querySelector({json.dumps(selector)});
            const form = el?.tagName === 'FORM' ? el : el?.closest('form');
            if (!form) throw new Error('No form found for: {selector}');
            const submitter = (el.tagName === 'INPUT' || el.tagName === 'BUTTON') ? el : null;
            const fd = new FormData(form, submitter);
            const entries = [];
            for (const [k, v] of fd.entries()) entries.push([k, String(v)]);
            return {{
                action: form.action,
                method: (form.method || 'get').toLowerCase(),
                entries,
            }};
        }})()
        """)
        action = info["action"]
        method = info["method"]
        entries = [(k, v) for k, v in info["entries"]]
        async with httpx.AsyncClient(transport=self.runtime.httpx_transport) as client:
            if method == "post":
                body = urlencode(entries).encode("utf-8")
                response = await client.post(
                    action,
                    content=body,
                    headers={"content-type": "application/x-www-form-urlencoded"},
                )
            else:
                response = await client.get(action, params=entries)
        await self.load(_extract_body_html(response.text))

    async def load(self, html: str) -> None:
        """Set document body and initialize htmx on the new content."""
        self.runtime.eval(f"document.body.innerHTML = {json.dumps(html)};")
        # happy-dom does not reflect the `selected` HTML attribute onto the .selected
        # IDL property when parsing via innerHTML — re-apply it before htmx.process.
        self.runtime.eval(
            "document.querySelectorAll('option[selected]')"
            ".forEach(opt => { opt.selected = true; });"
        )
        self.runtime.eval("htmx.process(document.body);")

    def close(self) -> None:
        self.runtime.close()

    def __enter__(self) -> Browser:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()
