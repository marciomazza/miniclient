import json
import string
from dataclasses import dataclass
from http import HTTPStatus
from textwrap import dedent
from types import SimpleNamespace
from typing import Callable, Literal

from hypothesis import assume, strategies as st
from hypothesis.strategies import SearchStrategy
from more_itertools import collapse, map_reduce

HxMethod = Literal["hx-get", "hx-post", "hx-put", "hx-patch", "hx-delete"]
HxSwap = Literal["innerHTML", "outerHTML", "beforebegin", "afterbegin", "beforeend", "afterend"]


def _attr_val(v):
    return json.dumps(v) if isinstance(v, dict) else str(v)


_BOOLEAN_ATTRS = {"checked", "selected", "multiple"}


def _attr_key_value(k, v):
    if k in _BOOLEAN_ATTRS:
        return k
    return f"{k}='{_attr_val(v)}'"


def _attrs_str(attrs):
    # discard if value is None
    return " ".join(_attr_key_value(k, v) for k, v in attrs.items() if v is not None)


st_some_text = st.text(string.ascii_letters, min_size=1, max_size=10)
st_some_text_maybe_empty = st.text(string.ascii_letters, min_size=0, max_size=10)
st_value = st.none() | st.text(string.ascii_letters, max_size=10)
st_none_or_true = st.none() | st.just("true")
st_maybe_dicts = st.none() | (st.dictionaries(st_some_text, st_some_text, max_size=3))


def st_maybe_from_type(type):
    return st.none() | st.from_type(type)


Interaction = Literal["fill", "click", "submit", "option", "select"]

_st_id = st.integers(min_value=0, max_value=2**20).map(lambda n: f"id_{n:x}")
_st_op_id = st.integers(min_value=0, max_value=2**20).map(lambda n: f"op_{n:x}")


@dataclass
class SimpleElement:
    tag: str
    id: str
    attrs: dict[str, str | None]
    interaction: Interaction
    content: str | list[SimpleElement] | None = None  # None => self closing

    @property
    def html(self):
        tag_with_attrs = f"{self.tag} {_attrs_str({'id': self.id, **self.attrs})}"
        if self.content is None:
            return f"<{tag_with_attrs} />"
        match self.content:
            case list():
                content = "\n".join(c.html for c in self.content)
            case _:
                content = self.content

        return f"<{tag_with_attrs}>{content}</{self.tag}>"

    @property
    def all_elements(self) -> list[SimpleElement]:
        match self.content:
            case list():
                return [self, *self.content]
            case _:
                return [self]


# --------------------------------------------------------------------------------
# single htmx node
# --------------------------------------------------------------------------------


@st.composite
def st_htmx_node(draw) -> SimpleElement:
    tag = draw(st.sampled_from(("div", "span", "button")))
    method = draw(st.from_type(HxMethod))
    return SimpleElement(
        tag=tag,
        id="focus",
        attrs={
            "id": "focus",
            method: "/fragment",
            "hx-target": draw(st.sampled_from(("this", "#result"))),
            "hx-swap": draw(st_maybe_from_type(HxSwap)),
            "hx-vals": draw(st_maybe_dicts),
            "hx-headers": draw(st_maybe_dicts),
        },
        content="Initial",
        interaction="click",
    )


def _page_with_node(node) -> bytes:
    return dedent(f"""\
        <html><head></head><body>
          <script src="/htmx.js"></script>
          {node.html}
          <div id="result"></div>
        </body></html>
        """).encode()


HTTP_GOOD_STATUS, HTTP_BAD_STATUS = (
    [f"{s.value} {s.phrase}" for s in HTTPStatus if s.value in batch]
    for batch in ({201, 301}, {400, 403, 500})
)


@st.composite
def st_wsgi_app(draw, st_node_strategy: Callable[..., SearchStrategy]):
    node = draw(st_node_strategy())
    status_list = draw(st.sampled_from([HTTP_GOOD_STATUS, HTTP_BAD_STATUS]))
    status_phrase = draw(st.sampled_from(status_list))

    def app(environ, start_response):
        path = environ["PATH_INFO"]
        if path == "/":
            body = _page_with_node(node)
            start_response("200 OK", [("Content-Type", "text/html")])
            return [body]
        if path == "/fragment":
            start_response(status_phrase, [("Content-Type", "text/html")])
            return [b"<span>Hello</span>"]
        start_response("404 Not Found", [("Content-Type", "text/plain")])
        return [b"Not found"]

    return app, node


# --------------------------------------------------------------------------------
# forms
# --------------------------------------------------------------------------------


# Names that, when used on form controls, shadow DOM properties and break htmx.
# Chromium exposes named form controls as element properties, which can shadow
# built-in DOM methods and properties, breaking htmx internals.
_RESERVED_FORM_NAMES = frozenset(
    # HTMLFormElement own properties (spec-excluded)
    "action autocomplete enctype encoding method name noValidate target "
    "elements length submit reset checkValidity reportValidity requestSubmit "
    # Node/Element properties
    "tagName nodeType nodeName textContent childNodes children attributes "
    "getAttribute setAttribute removeAttribute hasAttribute getAttributeNames "
    "toggleAttribute id className classList innerHTML outerHTML "
    "parentNode parentElement firstChild lastChild nextSibling previousSibling "
    "firstElementChild lastElementChild nextElementSibling previousElementSibling "
    "childElementCount isConnected ownerDocument "
    # Element mutation methods
    "append prepend before after remove replaceWith replaceChildren "
    "appendChild removeChild insertBefore insertAdjacentHTML insertAdjacentElement "
    "cloneNode contains "
    # Element query methods
    "querySelector querySelectorAll closest matches "
    # Event methods
    "addEventListener removeEventListener dispatchEvent "
    # Form-related
    "type value checked selected disabled hidden style form "
    # Other common properties
    "focus blur click scrollIntoView getBoundingClientRect "
    "offsetParent offsetWidth offsetHeight".split()
)


def _draw_safe_name(draw) -> str:
    name = draw(st_some_text)
    assume(name not in _RESERVED_FORM_NAMES)
    return name


def _attrs_name_value(draw):
    return {
        "name": _draw_safe_name(draw),
        "value": draw(st_value),
    }


@st.composite
def st_input(draw) -> SimpleElement:
    type = draw(st.none() | st.sampled_from(("", "text", "submit")))
    return SimpleElement(
        tag="input",
        id=draw(_st_id),
        attrs={
            "type": type,
            **_attrs_name_value(draw),
        },
        interaction="submit" if type == "submit" else "fill",
    )


@st.composite
def st_button_submit(draw) -> SimpleElement:
    return SimpleElement(
        tag="button",
        id=draw(_st_id),
        attrs={
            "type": "submit",
            **_attrs_name_value(draw),
        },
        content="Submit",
        interaction="submit",
    )


@st.composite
def st_checkbox_group(draw) -> list[SimpleElement]:
    labels = draw(st.lists(st_some_text, min_size=1, max_size=4, unique=True))
    return [
        SimpleElement(
            tag="input",
            id=draw(_st_id),
            attrs={
                "type": "checkbox",
                **_attrs_name_value(draw),
                "checked": draw(st_none_or_true),
            },
            interaction="click",
        )
        for lbl in labels
    ]


@st.composite
def st_radio_group(draw) -> list[SimpleElement]:
    n = draw(st.integers(min_value=0, max_value=4))
    selected_idx = draw(
        st.none() if n == 0 else st.none() | st.integers(min_value=0, max_value=n - 1)
    )
    return [
        SimpleElement(
            tag="input",
            id=draw(_st_id),
            attrs={
                "type": "radio",
                **_attrs_name_value(draw),
                "checked": "checked" if i == selected_idx else None,
            },
            interaction="click",
        )
        for i in range(n)
    ]


@st.composite
def st_textarea(draw) -> SimpleElement:
    return SimpleElement(
        tag="textarea",
        id=draw(_st_id),
        attrs={"name": _draw_safe_name(draw)},
        content=draw(st_some_text_maybe_empty),
        interaction="fill",
    )


@st.composite
def st_select(draw) -> SimpleElement:
    multiple = draw(st_none_or_true)
    option_contents = draw(st.lists(st_some_text, min_size=1, max_size=5, unique=True))
    n = len(option_contents)
    st_index = st.integers(min_value=0, max_value=n - 1)
    selected_indices = draw(st.sets(st_index)) if multiple else {draw(st_index)}
    options = [
        SimpleElement(
            tag="option",
            id=draw(_st_op_id),
            attrs={
                "value": draw(st_value),
                "selected": "selected" if i in selected_indices else None,
            },
            content=content,
            interaction="option",
        )
        for i, content in enumerate(option_contents)
    ]
    return SimpleElement(
        tag="select",
        id=draw(_st_id),
        attrs={"name": _draw_safe_name(draw), "multiple": multiple},
        content=options,
        interaction="select",
    )


st_form_control = (
    st_input()
    | st_checkbox_group()
    | st_radio_group()
    | st_textarea()
    | st_select()
    | st_button_submit()
)


def flat(iterables):
    return list(collapse(iterables))


def _draw_form_controls(draw):
    controls: list[SimpleElement] = flat(draw(st.lists(st_form_control, min_size=1, max_size=10)))
    all_ids = [e.id for c in controls for e in c.all_elements]
    assume(len(all_ids) == len(set(all_ids)))
    ids_by_interaction = map_reduce(
        collapse(c.all_elements for c in controls),
        keyfunc=lambda c: c.interaction,
        valuefunc=lambda c: c.id,
    )
    return controls, ids_by_interaction


def _build_form(extra_attrs: str, controls, ids_by_interaction) -> SimpleNamespace:
    html = dedent(f"""\
        <form id='focus' {extra_attrs}>
            {"\n".join(c.html for c in controls)}
        </form>
    """)
    return SimpleNamespace(html=html, ids_by_interaction=ids_by_interaction)


@st.composite
def st_plain_html_form(draw) -> SimpleNamespace:
    method = draw(st.sampled_from(("get", "post")))
    controls, ids_by_interaction = _draw_form_controls(draw)
    return _build_form(f"method='{method}' action='/fragment'", controls, ids_by_interaction)


@st.composite
def st_html_form(draw) -> SimpleNamespace:
    method = draw(st.from_type(HxMethod))
    attrs = {
        "hx-target": draw(st.sampled_from(("this", "#result"))),
        "hx-swap": draw(st_maybe_from_type(HxSwap)),
        "hx-vals": draw(st_maybe_dicts),
        "hx-headers": draw(st_maybe_dicts),
    }
    controls, ids_by_interaction = _draw_form_controls(draw)
    return _build_form(f"{method}='/fragment' {_attrs_str(attrs)}", controls, ids_by_interaction)
