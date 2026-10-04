import pytest
from pytest_httpx2 import HTTPXMock

from miniclient.browser import Browser, Page


def test_new_page_goto_reads_the_document(httpx_mock: HTTPXMock) -> None:
    httpx_mock.add_response(url="http://localhost/a", text="<p id='m'>hi</p>")
    with Browser() as browser, browser.new_page() as page:
        page.goto("http://localhost/a")
        el = page.find("#m")
        assert el is not None and el.text == "hi"


def test_pages_are_independent() -> None:
    with Browser() as browser, browser.new_page() as one, browser.new_page() as two:
        one.load("<p id='m'>one</p>")
        two.load("<p id='m'>two</p>")
        one.eval("document.cookie = 'k=1'")
        el = one.find("#m")
        assert el is not None and el.text == "one"
        el = two.find("#m")
        assert el is not None and el.text == "two"
        assert two.eval("document.cookie") == ""


def test_pages_survive_browser_close() -> None:
    browser = Browser()
    page = browser.new_page()
    browser.close()
    browser.close()  # idempotent
    page.load("<p id='m'>still here</p>")
    el = page.find("#m")
    assert el is not None and el.text == "still here"
    page.close()


def test_new_page_after_close_raises() -> None:
    browser = Browser()
    browser.close()
    with pytest.raises(RuntimeError, match="closed"):
        browser.new_page()


def test_new_page_uses_page_class() -> None:
    class MyPage(Page):
        pass

    class MyBrowser(Browser):
        page_class = MyPage

    with MyBrowser() as browser, browser.new_page() as page:
        assert isinstance(page, MyPage)
