"""Tests for the pure-JS FormData implementation (formdata.js)."""

import asyncio
import json
from collections.abc import Generator

import pytest
from jsrun import Runtime

from htmxclient.runtime import build_runtime


@pytest.fixture(scope="module")
def formdata_runtime(snapshot) -> Generator[Runtime, None, None]:
    async def _build() -> Runtime:
        return await build_runtime("http://localhost/", snapshot=snapshot)

    r = asyncio.run(_build())
    yield r
    r.close()


def _pairs(r: Runtime, form_html: str) -> list[tuple[str, str]]:
    """Create a form from HTML, collect FormData, return list of (name, value) pairs."""
    result = r.eval(
        f"""
        (function () {{
          const wrap = document.createElement('div');
          wrap.innerHTML = {json.dumps(form_html)};
          const form = wrap.querySelector('form');
          return [...new FormData(form).entries()];
        }})();
        """
    )
    return [tuple(p) for p in result]


@pytest.mark.parametrize(
    "html,expected",
    [
        # text input — with and without value
        ('<form><input name="x" type="text" value="hello"></form>', [("x", "hello")]),
        ('<form><input name="x" type="text"></form>', [("x", "")]),
        # generic input
        ('<form><input name="foo" value="bar"></form>', [("foo", "bar")]),
        # textarea — with and without content
        ('<form><textarea name="msg">hello</textarea></form>', [("msg", "hello")]),
        ('<form><textarea name="x"></textarea></form>', [("x", "")]),
        # select single — explicit selected
        (
            '<form><select name="x">'
            '<option value="a">A</option>'
            '<option value="b" selected>B</option>'
            "</select></form>",
            [("x", "b")],
        ),
        # select single — no selected → first option auto-selected
        (
            '<form><select name="x">'
            '<option value="a">A</option>'
            '<option value="b">B</option>'
            "</select></form>",
            [("x", "a")],
        ),
        # select single — option without value attribute → text content
        (
            '<form><select name="x"><option>text-only</option></select></form>',
            [("x", "text-only")],
        ),
        # select multiple
        (
            '<form><select name="items" multiple>'
            '<option value="a" selected>A</option>'
            '<option value="b" selected>B</option>'
            '<option value="c">C</option>'
            "</select></form>",
            [("items", "a"), ("items", "b")],
        ),
        # checkbox — checked, with value
        (
            '<form><input type="checkbox" name="agree" value="yes" checked></form>',
            [("agree", "yes")],
        ),
        # checkbox — checked, no value attribute → "on"
        ('<form><input type="checkbox" name="ok" checked></form>', [("ok", "on")]),
        # checkbox — checked, explicit empty value attribute → ""
        ('<form><input type="checkbox" name="x" value="" checked></form>', [("x", "")]),
        # multiple checkboxes with same name
        (
            "<form>"
            '<input type="checkbox" name="hobby" value="read" checked>'
            '<input type="checkbox" name="hobby" value="game" checked>'
            "</form>",
            [("hobby", "read"), ("hobby", "game")],
        ),
        # radio — checked, explicit empty value attribute → ""
        ('<form><input type="radio" name="x" value="" checked></form>', [("x", "")]),
        # radio — checked one
        ('<form><input type="radio" name="color" value="red" checked></form>', [("color", "red")]),
        # multiple radios — only the checked one is collected
        (
            "<form>"
            '<input type="radio" name="size" value="s">'
            '<input type="radio" name="size" value="m" checked>'
            '<input type="radio" name="size" value="l">'
            "</form>",
            [("size", "m")],
        ),
    ],
)
def test_collects_successful_controls(formdata_runtime: Runtime, html: str, expected: list) -> None:
    assert _pairs(formdata_runtime, html) == expected


@pytest.mark.parametrize(
    "html",
    [
        # unchecked checkbox / radio
        '<form><input type="checkbox" name="agree" value="yes"></form>',
        '<form><input type="radio" name="color" value="red"></form>',
        # disabled control
        '<form><input name="x" value="1" disabled></form>',
        # no name attribute
        '<form><input value="1"></form>',
        # submit-like inputs
        '<form><input type="submit" name="s" value="go"></form>',
        '<form><input type="button" name="b" value="go"></form>',
        '<form><input type="file" name="f"></form>',
        # empty select
        '<form><select name="x"></select></form>',
        # select multiple with no option selected
        '<form><select name="x" multiple><option value="a">A</option></select></form>',
    ],
)
def test_excludes_unsuccessful_controls(formdata_runtime: Runtime, html: str) -> None:
    assert _pairs(formdata_runtime, html) == []


def _urlsearchparams_string(r: Runtime, form_html: str) -> str:
    """Serialize a form's FormData via URLSearchParams, as Element.submit does in JS."""
    return r.eval(
        f"""
        (function () {{
          const wrap = document.createElement('div');
          wrap.innerHTML = {json.dumps(form_html)};
          const form = wrap.querySelector('form');
          const fd = new FormData(form);
          return new URLSearchParams(fd).toString();
        }})();
        """
    )


@pytest.mark.parametrize(
    "html,expected",
    [
        # single field
        ('<form><input name="x" value="hello"></form>', "x=hello"),
        # multiple fields
        (
            '<form><input name="a" value="1"><input name="b" value="2"></form>',
            "a=1&b=2",
        ),
        # values requiring encoding
        ('<form><input name="q" value="hello world"></form>', "q=hello+world"),
        # multiple values for same name
        (
            "<form>"
            '<input type="checkbox" name="c" value="x" checked>'
            '<input type="checkbox" name="c" value="y" checked>'
            "</form>",
            "c=x&c=y",
        ),
        # empty value
        ('<form><input name="x" value=""></form>', "x="),
    ],
)
def test_urlsearchparams_from_formdata(formdata_runtime: Runtime, html: str, expected: str) -> None:
    assert _urlsearchparams_string(formdata_runtime, html) == expected


def _set_pairs(r: Runtime, js: str) -> list[tuple[str, str]]:
    """Run FormData operations against a fresh instance, return resulting (name, value) pairs."""
    result = r.eval(
        f"""
        (function () {{
          const fd = new FormData();
          {js};
          return [...fd.entries()];
        }})();
        """
    )
    return [tuple(p) for p in result]


@pytest.mark.parametrize(
    "js,expected",
    [
        # set() on an existing key replaces the value in place, keeping original position
        (
            "fd.append('a', '1'); fd.append('b', '2'); fd.append('c', '3'); fd.set('a', 'X')",
            [("a", "X"), ("b", "2"), ("c", "3")],
        ),
        # set() on a new key appends it at the end
        (
            "fd.append('a', '1'); fd.set('b', '2')",
            [("a", "1"), ("b", "2")],
        ),
        # set() with pre-existing duplicates collapses them to the first entry's position
        (
            "fd.append('a', '1'); fd.append('a', '2'); fd.append('b', '3'); fd.set('a', 'X')",
            [("a", "X"), ("b", "3")],
        ),
    ],
)
def test_set_replaces_in_place(formdata_runtime: Runtime, js: str, expected: list) -> None:
    assert _set_pairs(formdata_runtime, js) == expected
