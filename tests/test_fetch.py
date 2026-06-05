import json

import pytest


async def js_fetch_text(browser, url, **opts):
    js_opts = json.dumps(opts) if opts else "{}"
    return await browser.eval_async(f"fetch({json.dumps(url)}, {js_opts}).then(r => r.text())")


async def js_fetch_json(browser, url, **opts):
    js_opts = json.dumps(opts) if opts else "{}"
    return await browser.eval_async(f"fetch({json.dumps(url)}, {js_opts}).then(r => r.json())")


async def js_fetch_status(browser, url):
    return await browser.eval_async(f"fetch({json.dumps(url)}).then(r => r.status)")


# ---------------------------------------------------------------------------
# Basic response reading
# ---------------------------------------------------------------------------


async def test_fetch_text(browser_async, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/hello", text="hello world")
    result = await js_fetch_text(browser_async, "http://api.example.com/hello")
    assert result == "hello world"


async def test_fetch_json(browser_async, httpx_mock):
    httpx_mock.add_response(
        url="http://api.example.com/data",
        json={"name": "Alice", "age": 30},
    )
    result = await js_fetch_json(browser_async, "http://api.example.com/data")
    assert result == {"name": "Alice", "age": 30}


async def test_fetch_status_ok(browser_async, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/ok", status_code=200)
    assert await js_fetch_status(browser_async, "http://api.example.com/ok") == 200


async def test_fetch_status_not_found(browser_async, httpx_mock):
    # fetch does not throw on 4xx — ok is false, status is 404
    httpx_mock.add_response(url="http://api.example.com/missing", status_code=404)
    result = await browser_async.eval_async(
        "fetch('http://api.example.com/missing').then(r => ({ok: r.ok, status: r.status}))"
    )
    assert result == {"ok": False, "status": 404}


# ---------------------------------------------------------------------------
# HTTP methods
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "method",
    [
        "GET",
        "POST",
        "PUT",
        "DELETE",
        "PATCH",
    ],
)
async def test_fetch_methods(browser_async, httpx_mock, method):
    url = "http://api.example.com/resource"
    httpx_mock.add_response(url=url, text=method)
    result = await js_fetch_text(browser_async, url, method=method)
    assert result == method


# ---------------------------------------------------------------------------
# Request headers forwarded
# ---------------------------------------------------------------------------


async def test_fetch_sends_custom_headers(browser_async, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/auth", text="ok")
    await js_fetch_text(
        browser_async,
        "http://api.example.com/auth",
        headers={"Authorization": "Bearer token123"},
    )
    request = httpx_mock.get_request()
    assert request.headers.get("authorization") == "Bearer token123"


# ---------------------------------------------------------------------------
# Response headers available in JS
# ---------------------------------------------------------------------------


async def test_fetch_response_headers(browser_async, httpx_mock):
    httpx_mock.add_response(
        url="http://api.example.com/typed",
        headers={"content-type": "application/json; charset=utf-8"},
        text="{}",
    )
    ct = await browser_async.eval_async(
        "fetch('http://api.example.com/typed').then(r => r.headers.get('content-type'))"
    )
    assert "application/json" in ct


# ---------------------------------------------------------------------------
# POST with JSON body
# ---------------------------------------------------------------------------


async def test_fetch_post_json_body(browser_async, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/echo", text="saved")
    result = await browser_async.eval_async(
        """
        fetch('http://api.example.com/echo', {
            method: 'POST',
            headers: {'content-type': 'application/json'},
            body: JSON.stringify({key: 'value'}),
        }).then(r => r.text())
        """
    )
    assert result == "saved"
    request = httpx_mock.get_request()
    assert request.method == "POST"
    assert json.loads(request.content) == {"key": "value"}


# ---------------------------------------------------------------------------
# Empty body (e.g. 204 No Content)
# ---------------------------------------------------------------------------


async def test_fetch_empty_body(browser_async, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/empty", status_code=204, text="")
    result = await browser_async.eval_async(
        "fetch('http://api.example.com/empty').then(r => r.status)"
    )
    assert result == 204
