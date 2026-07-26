/**
 * JS bridge that replaces fetch-mock.js.
 *
 * mockResponse/mockJsonResponse/... register mocks via Python sync ops so
 * that actual fetch calls go through the Python httpx client, proving htmx
 * is integrated with httpx.  Call recording stays in JS so that
 * fetchMock.calls[0].request.body (FormData) remains accessible to tests.
 */
(function () {
    // Captured lazily by installFetchMock() so the v8_snapshot build (which
    // runs before bootstrap.js sets globalThis.fetch) does not capture undefined.
    let _origFetch = null;

    // A plain string urlPattern (e.g. '/test?name=test') is meant as a literal
    // substring to match, not a regex — but Python compiles it with re.compile().
    // Escape regex metacharacters so literal '?', '.', etc. in real paths/query
    // strings don't get misinterpreted. RegExp patterns pass through untouched.
    function toPatternStr(urlPattern) {
        if (urlPattern instanceof RegExp) return urlPattern.source;
        return String(urlPattern).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    }

    // Minimal MockResponse for compatibility with end2end tests that pass
    // `new MockResponse(body, {status, headers})` to mockResponse().
    class MockResponse {
        constructor(body, init) {
            init = init || {};
            this.body = typeof body === "string" ? body : body || "";
            this.status = init.status || 200;
            this.statusText = init.statusText || "";
            this.headers = init.headers || {};
        }
    }
    globalThis.MockResponse = MockResponse;

    class FetchMock {
        constructor() {
            this.calls = [];
            this.pendingRequests = [];
            // Function-based responses (e.g. a "was this hit" flag test) must run at
            // actual fetch time, not at registration time — handled entirely on the JS
            // side in fetch() below, bypassing the Python-backed httpx transport.
            this.functionMocks = [];
        }

        reset() {
            __host_fm_reset({});
            this.calls = [];
            this.pendingRequests = [];
            this.functionMocks = [];
        }

        mockResponse(method, urlPattern, response, options) {
            options = options || {};
            let body = "";
            let status = options.status || 200;
            const headers = options.headers || {};
            const once = !!options.once;

            if (typeof response === "string") {
                body = response;
            } else if (typeof response === "function" || response instanceof Response) {
                // A real Response can't be registered through the Python-backed
                // __host_fm_register (its body/headers require an async .text() read,
                // and a ReadableStream body must stay a live stream, not be flattened to
                // a string) — resolve it lazily in JS instead, same as a function mock.
                const patternStr = toPatternStr(urlPattern);
                this.functionMocks.push({
                    method: method.toUpperCase(),
                    regex: new RegExp(patternStr),
                    fn: typeof response === "function" ? response : () => response,
                });
                return;
            } else if (response && typeof response === "object") {
                body = typeof response.body === "string" ? response.body : "";
                if (!options.status && response.status) {
                    status = response.status;
                }
            }

            const patternStr = toPatternStr(urlPattern);

            __host_fm_register({
                method: method.toUpperCase(),
                urlPattern: patternStr,
                body: body,
                status: status,
                headers: headers,
                once: once,
                is_error: false,
                error_msg: "",
            });
        }

        mockJsonResponse(method, urlPattern, data, status) {
            status = status || 200;
            const patternStr = toPatternStr(urlPattern);
            __host_fm_register({
                method: method.toUpperCase(),
                urlPattern: patternStr,
                body: JSON.stringify(data),
                status: status,
                headers: { "content-type": "application/json" },
                once: false,
                is_error: false,
                error_msg: "",
            });
        }

        mockErrorResponse(method, urlPattern, status, message) {
            status = status || 500;
            message = message || "Server Error";
            const patternStr = toPatternStr(urlPattern);
            __host_fm_register({
                method: method.toUpperCase(),
                urlPattern: patternStr,
                body: message,
                status: status,
                headers: {},
                once: false,
                is_error: false,
                error_msg: "",
            });
        }

        mockNetworkError(method, urlPattern, error) {
            error = error || new Error("Network Error");
            const patternStr = toPatternStr(urlPattern);
            const msg = error instanceof Error ? error.message : String(error);
            __host_fm_register({
                method: method.toUpperCase(),
                urlPattern: patternStr,
                body: "",
                status: 0,
                headers: {},
                once: false,
                is_error: true,
                error_msg: msg,
            });
        }

        mockFailure(method, urlPattern, message) {
            message = message || "Network failure";
            this.mockNetworkError(method, urlPattern, new Error(message));
        }

        mockSequentialResponses(method, urlPattern, response, options) {
            options = options || {};
            const status = options.status || 200;
            const headers = options.headers || {};
            const body = typeof response === "string" ? response : JSON.stringify(response);
            const patternStr = toPatternStr(urlPattern);

            const seqId = __host_fm_register_seq({
                method: method.toUpperCase(),
                urlPattern: patternStr,
                body: body,
                status: status,
                headers: headers,
            });

            return {
                next() {
                    // Fire-and-forget: runner.js ignores the return value and waits
                    // for htmx:finally:request instead.
                    __host_fm_next({ seq_id: seqId });
                    return Promise.resolve();
                },
                get pendingCount() {
                    return 0;
                },
            };
        }

        getCalls() {
            return this.calls;
        }

        getLastCall() {
            return this.calls[this.calls.length - 1];
        }

        fetch(url, options) {
            options = options || {};
            if (!options.method) options.method = "GET";
            options.method = options.method.toUpperCase();
            // Record before calling _origFetch so FormData is still accessible.
            this.calls.push({ url: url, request: options });

            // Most-recently-registered function mock wins, matching the Python
            // transport's behavior for string/object mocks (see _dispatch).
            for (let i = this.functionMocks.length - 1; i >= 0; i--) {
                const mock = this.functionMocks[i];
                if (mock.method === options.method && mock.regex.test(String(url))) {
                    return this._resolveFunctionMock(mock);
                }
            }
            return _origFetch(url, options);
        }

        async _resolveFunctionMock(mock) {
            let result;
            try {
                result = await mock.fn();
            } catch (e) {
                throw new TypeError(e && e.message ? e.message : String(e));
            }
            if (result instanceof Response) {
                return result;
            }
            if (result instanceof MockResponse) {
                return new Response(result.body, {
                    status: result.status,
                    statusText: result.statusText,
                    headers: result.headers,
                });
            }
            if (typeof result === "string") {
                return new Response(result, { status: 200 });
            }
            // Unsupported dynamic shape (e.g. a ReadableStream-based SSE mock) —
            // this bridge can't emulate per-call streaming behavior.
            throw new TypeError(
                `fetchMock: unsupported function-mock return value for ${mock.regex}`,
            );
        }

        waitForRequests() {
            return Promise.resolve();
        }
    }

    const fetchMock = new FetchMock();
    globalThis.fetchMock = fetchMock;

    // Replaces globalThis.fetch with the recording bridge that routes through
    // the Python-backed _origFetch.  Called by helpers.js at test setup time,
    // after bootstrap.js has installed the real Python-backed fetch.
    globalThis.installFetchMock = function installFetchMock() {
        if (!_origFetch) {
            _origFetch = globalThis.fetch;
            globalThis.fetch = fetchMock.fetch.bind(fetchMock);
        }
    };

    globalThis.uninstallFetchMock = function uninstallFetchMock() {
        if (_origFetch) {
            globalThis.fetch = _origFetch;
            _origFetch = null;
        }
    };
})();
