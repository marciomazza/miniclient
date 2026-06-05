def test_window_instantiates(browser):
    assert browser.eval("typeof window") == "object"


def test_document_basic(browser):
    assert browser.eval("document.createElement('div').tagName") == "DIV"


def test_abort_controller(browser):
    assert browser.eval("new AbortController().signal.aborted") is False
