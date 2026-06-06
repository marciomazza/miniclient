import pytest

# ---------------------------------------------------------------------------
# Fast path — no templates
# ---------------------------------------------------------------------------


def test_regular_html_unchanged(browser):
    result = browser.eval(
        """new DOMParser().parseFromString('<div><p>hello</p></div>', 'text/html').body.innerHTML"""
    )
    assert result == "<div><p>hello</p></div>"


def test_body_wrapping(browser):
    result = browser.eval("""
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
def test_template_table_tags_in_content(browser, html, expected_inner):
    result = browser.eval(
        f"""new DOMParser()
            .parseFromString({html!r}, 'text/html')
            .querySelector('template').innerHTML"""
    )
    assert result == expected_inner


# ---------------------------------------------------------------------------
# Template with non-table content
# ---------------------------------------------------------------------------


def test_template_with_paragraph(browser):
    result = browser.eval("""
        new DOMParser()
            .parseFromString('<template><p>hello</p></template>', 'text/html')
            .querySelector('template').innerHTML
    """)
    assert result == "<p>hello</p>"


def test_template_with_list(browser):
    result = browser.eval("""
        new DOMParser()
            .parseFromString('<template><ul><li>a</li><li>b</li></ul></template>', 'text/html')
            .querySelector('template').innerHTML
    """)
    assert result == "<ul><li>a</li><li>b</li></ul>"


# ---------------------------------------------------------------------------
# Template attributes are preserved
# ---------------------------------------------------------------------------


def test_template_id_preserved(browser):
    result = browser.eval("""
        new DOMParser()
            .parseFromString('<template id="tmpl1"><tr><td>x</td></tr></template>', 'text/html')
            .body.innerHTML
    """)
    assert result == '<template id="tmpl1"><tr><td>x</td></tr></template>'


def test_template_data_attribute_preserved(browser):
    result = browser.eval("""
        new DOMParser()
            .parseFromString('<template data-foo="bar"><p>x</p></template>', 'text/html')
            .body.innerHTML
    """)
    assert result == '<template data-foo="bar"><p>x</p></template>'


# ---------------------------------------------------------------------------
# Multiple templates in one parse
# ---------------------------------------------------------------------------


def test_multiple_templates(browser):
    result = browser.eval("""
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


def test_nested_template_inner_content(browser):
    result = browser.eval("""
        const html =
            '<template id="outer">' +
            '<template id="inner"><tr><td>deep</td></tr></template>' +
            '</template>';
        new DOMParser().parseFromString(html, 'text/html').querySelector('#outer').innerHTML
    """)
    assert result == '<template id="inner"><tr><td>deep</td></tr></template>'


def test_nested_template_outer_content_preserved(browser):
    result = browser.eval("""
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


def test_empty_template(browser):
    result = browser.eval("""
        new DOMParser()
            .parseFromString('<template></template>', 'text/html')
            .querySelector('template').innerHTML
    """)
    assert result == ""
