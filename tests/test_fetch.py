import json

import pytest
from fetch_helpers import js_fetch_json, js_fetch_status, js_fetch_text

# ---------------------------------------------------------------------------
# Basic response reading
# ---------------------------------------------------------------------------


async def test_fetch_text(runtime, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/hello", text="hello world")
    result = await js_fetch_text(runtime, "http://api.example.com/hello")
    assert result == "hello world"


async def test_fetch_json(runtime, httpx_mock):
    httpx_mock.add_response(
        url="http://api.example.com/data",
        json={"name": "Alice", "age": 30},
    )
    result = await js_fetch_json(runtime, "http://api.example.com/data")
    assert result == {"name": "Alice", "age": 30}


async def test_fetch_status_ok(runtime, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/ok", status_code=200)
    assert await js_fetch_status(runtime, "http://api.example.com/ok") == 200


async def test_fetch_status_not_found(runtime, httpx_mock):
    # fetch does not throw on 4xx — ok is false, status is 404
    httpx_mock.add_response(url="http://api.example.com/missing", status_code=404)
    result = await runtime.eval_async(
        "fetch('http://api.example.com/missing').then(r => ({ok: r.ok, status: r.status}))"
    )
    assert result == {"ok": False, "status": 404}


async def test_fetch_follows_redirect(runtime, httpx_mock):
    httpx_mock.add_response(
        url="http://api.example.com/old",
        status_code=302,
        headers={"location": "http://api.example.com/new"},
    )
    httpx_mock.add_response(url="http://api.example.com/new", text="moved")
    result = await runtime.eval_async(
        "fetch('http://api.example.com/old')"
        ".then(async r => ({url: r.url, status: r.status, body: await r.text()}))"
    )
    assert result == {"url": "http://api.example.com/new", "status": 200, "body": "moved"}


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
async def test_fetch_methods(runtime, httpx_mock, method):
    url = "http://api.example.com/resource"
    httpx_mock.add_response(url=url, text=method)
    result = await js_fetch_text(runtime, url, method=method)
    assert result == method


# ---------------------------------------------------------------------------
# Request headers forwarded
# ---------------------------------------------------------------------------


async def test_fetch_sends_custom_headers(runtime, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/auth", text="ok")
    await js_fetch_text(
        runtime,
        "http://api.example.com/auth",
        headers={"Authorization": "Bearer token123"},
    )
    request = httpx_mock.get_request()
    assert request.headers.get("authorization") == "Bearer token123"


# ---------------------------------------------------------------------------
# Response headers available in JS
# ---------------------------------------------------------------------------


async def test_fetch_response_headers(runtime, httpx_mock):
    httpx_mock.add_response(
        url="http://api.example.com/typed",
        headers={"content-type": "application/json; charset=utf-8"},
        text="{}",
    )
    ct = await runtime.eval_async(
        "fetch('http://api.example.com/typed').then(r => r.headers.get('content-type'))"
    )
    assert "application/json" in ct


# ---------------------------------------------------------------------------
# POST with JSON body
# ---------------------------------------------------------------------------


async def test_fetch_post_json_body(runtime, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/echo", text="saved")
    result = await runtime.eval_async(
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


async def test_fetch_empty_body(runtime, httpx_mock):
    httpx_mock.add_response(url="http://api.example.com/empty", status_code=204, text="")
    result = await runtime.eval_async("fetch('http://api.example.com/empty').then(r => r.status)")
    assert result == 204
