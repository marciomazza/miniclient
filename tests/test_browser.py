from pytest_httpx2 import HTTPXMock

from miniclient.browser import Browser, Page


def test_page_goto_reads_the_document(browser: Browser, httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(url="http://localhost/a", text="<p id='m'>hi</p>")
    with Page(browser) as page:
        page.goto("http://localhost/a")
        el = page.find("#m")
        assert el is not None and el.text == "hi"


def test_pages_are_independent(browser: Browser) -> None:
    with Page(browser) as one, Page(browser) as two:
        one.load("<p id='m'>one</p>")
        two.load("<p id='m'>two</p>")
        one.eval("document.cookie = 'k=1'")
        el = one.find("#m")
        assert el is not None and el.text == "one"
        el = two.find("#m")
        assert el is not None and el.text == "two"
        assert two.eval("document.cookie") == ""
