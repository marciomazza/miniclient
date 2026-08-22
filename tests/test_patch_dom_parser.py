import pytest

# ---------------------------------------------------------------------------
# Fast path — no templates
# ---------------------------------------------------------------------------


async def test_regular_html_unchanged(runtime):
    result = runtime.eval(
        """new DOMParser().parseFromString('<div><p>hello</p></div>', 'text/html').body.innerHTML"""
    )
    assert result == "<div><p>hello</p></div>"


async def test_body_wrapping(runtime):
    result = runtime.eval("""
        new DOMParser()
            .parseFromString('<body><p>hi</p></body>', 'text/html')
            .documentElement.outerHTML
    """)
    assert result == "<html><head></head><body><p>hi</p></body></html>"


# ---------------------------------------------------------------------------
# Template with table orphan tags — isolated and embedded
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "html, expected_inner",
    [
        # isolated template (original case)
        ("<template><tr><td>x</td></tr></template>", "<tr><td>x</td></tr>"),
        # template embedded in larger HTML (new general case)
        ("<table><template><tr><td>x</td></tr></template></table>", "<tr><td>x</td></tr>"),
        ("<div><template><tr><td>x</td></tr></template></div>", "<tr><td>x</td></tr>"),
        # td / th — need extra <tr> wrapper
        ("<template><td>x</td></template>", "<td>x</td>"),
        ("<template><th>x</th></template>", "<th>x</th>"),
        # section tags — just need <table> wrapper
        (
            "<template><thead><tr><th>x</th></tr></thead></template>",
            "<thead><tr><th>x</th></tr></thead>",
        ),
        (
            "<template><tbody><tr><td>x</td></tr></tbody></template>",
            "<tbody><tr><td>x</td></tr></tbody>",
        ),
        (
            "<template><tfoot><tr><td>x</td></tr></tfoot></template>",
            "<tfoot><tr><td>x</td></tr></tfoot>",
        ),
    ],
)
async def test_template_table_tags_in_content(runtime, html, expected_inner):
    result = runtime.eval(
        f"""new DOMParser()
            .parseFromString({html!r}, 'text/html')
            .querySelector('template').innerHTML"""
    )
    assert result == expected_inner


# ---------------------------------------------------------------------------
# Template with non-table content
# ---------------------------------------------------------------------------


async def test_template_with_paragraph(runtime):
    result = runtime.eval("""
        new DOMParser()
            .parseFromString('<template><p>hello</p></template>', 'text/html')
            .querySelector('template').innerHTML
    """)
    assert result == "<p>hello</p>"


async def test_template_with_list(runtime):
    result = runtime.eval("""
        new DOMParser()
            .parseFromString('<template><ul><li>a</li><li>b</li></ul></template>', 'text/html')
            .querySelector('template').innerHTML
    """)
    assert result == "<ul><li>a</li><li>b</li></ul>"


# ---------------------------------------------------------------------------
# Template attributes are preserved
# ---------------------------------------------------------------------------


async def test_template_id_preserved(runtime):
    result = runtime.eval("""
        new DOMParser()
            .parseFromString('<template id="tmpl1"><tr><td>x</td></tr></template>', 'text/html')
            .body.innerHTML
    """)
    assert result == '<template id="tmpl1"><tr><td>x</td></tr></template>'


async def test_template_data_attribute_preserved(runtime):
    result = runtime.eval("""
        new DOMParser()
            .parseFromString('<template data-foo="bar"><p>x</p></template>', 'text/html')
            .body.innerHTML
    """)
    assert result == '<template data-foo="bar"><p>x</p></template>'


# ---------------------------------------------------------------------------
# Multiple templates in one parse
# ---------------------------------------------------------------------------


async def test_multiple_templates(runtime):
    result = runtime.eval("""
        const html =
            '<template id="a"><tr><td>A</td></tr></template>' +
            '<template id="b"><tr><td>B</td></tr></template>';
        new DOMParser().parseFromString(html, 'text/html').body.innerHTML
    """)
    assert result == (
        '<template id="a"><tr><td>A</td></tr></template>'
        '<template id="b"><tr><td>B</td></tr></template>'
    )


# ---------------------------------------------------------------------------
# Nested templates
# ---------------------------------------------------------------------------


async def test_nested_template_inner_content(runtime):
    result = runtime.eval("""
        const html =
            '<template id="outer">' +
            '<template id="inner"><tr><td>deep</td></tr></template>' +
            '</template>';
        new DOMParser().parseFromString(html, 'text/html').querySelector('#outer').innerHTML
    """)
    assert result == '<template id="inner"><tr><td>deep</td></tr></template>'


async def test_nested_template_outer_content_preserved(runtime):
    result = runtime.eval("""
        const html =
            '<template id="outer">' +
            '<p>before</p>' +
            '<template id="inner"><p>inside</p></template>' +
            '<p>after</p>' +
            '</template>';
        new DOMParser().parseFromString(html, 'text/html').querySelector('#outer').innerHTML
    """)
    assert result == '<p>before</p><template id="inner"><p>inside</p></template><p>after</p>'


# ---------------------------------------------------------------------------
# Empty template
# ---------------------------------------------------------------------------


async def test_empty_template(runtime):
    result = runtime.eval("""
        new DOMParser()
            .parseFromString('<template></template>', 'text/html')
            .querySelector('template').innerHTML
    """)
    assert result == ""


# ---------------------------------------------------------------------------
# insertAdjacentHTML — parsing context and node order
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "container_html, selector, markup, expected_selector",
    [
        # each of these is dropped by stock happy-dom, which parses without a context element
        ("<table><tbody></tbody></table>", "tbody", "<tr><td>x</td></tr>", "tbody > tr"),
        ("<table><tbody><tr></tr></tbody></table>", "tr", "<td>x</td>", "tr > td"),
        ("<table></table>", "table", "<tbody><tr><td>x</td></tr></tbody>", "table > tbody"),
    ],
)
async def test_insert_adjacent_html_table_context(
    runtime, container_html, selector, markup, expected_selector
):
    runtime.eval(f"""\
        document.body.innerHTML = "{container_html}";
        document.querySelector("{selector}").insertAdjacentHTML("beforeend", "{markup}");
    """)
    assert runtime.eval(f"""document.querySelectorAll("{expected_selector}").length""") == 1


@pytest.mark.parametrize(
    "position, expected",
    [
        # afterbegin/afterend regress: node-by-node insertion reuses one anchor, reversing them
        ("afterbegin", "<b>1</b><b>2</b><i>z</i>"),
        ("beforeend", "<i>z</i><b>1</b><b>2</b>"),
        ("beforebegin", "<b>1</b><b>2</b><div id='t'><i>z</i></div>"),
        ("afterend", "<div id='t'><i>z</i></div><b>1</b><b>2</b>"),
    ],
)
async def test_insert_adjacent_html_order(runtime, position, expected):
    runtime.eval(f"""\
        document.body.innerHTML = "<div id='t'><i>z</i></div>";
        document.getElementById("t").insertAdjacentHTML("{position}", "<b>1</b><b>2</b>");
    """)
    target = "t" if position in ("afterbegin", "beforeend") else "body"
    got = runtime.eval(f"""\
        ({{t: () => document.getElementById("t"), body: () => document.body}})["{target}"]()
            .innerHTML
    """)
    assert got.replace('"', "'") == expected


@pytest.mark.parametrize(
    "position, expected_error",
    [
        # detached element: no parent to insert next to (stock happy-dom loops forever here)
        ("beforebegin", "NoModificationAllowedError"),
        ("nonsense", "SyntaxError"),  # not one of the four legal keywords
    ],
)
async def test_insert_adjacent_html_throws_dom_exception(runtime, position, expected_error):
    # the DOM must be untouched: the spec throws before inserting anything
    assert runtime.eval(f"""\
        (() => {{
            const div = document.createElement("div");
            try {{
                div.insertAdjacentHTML("{position}", "<b>x</b>");
            }} catch (e) {{
                return [e.name, e instanceof DOMException, div.childNodes.length];
            }}
            return ["no error", false, div.childNodes.length];
        }})()
    """) == [expected_error, True, 0]
