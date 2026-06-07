/**
 * JS bridge that replaces fetch-mock.js.
 *
 * mockResponse/mockJsonResponse/... register mocks via Python sync ops so
 * that actual fetch calls go through the Python httpx client, proving htmx
 * is integrated with httpx.  Call recording stays in JS so that
 * fetchMock.calls[0].request.body (FormData) remains accessible to tests.
 */
(function () {
    // Captured lazily by installFetchMock() so the snapshot build (which
    // runs before bootstrap.js sets globalThis.fetch) does not capture undefined.
    let _origFetch = null;

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
        }

        reset() {
            if (typeof globalThis.__FM_RESET__ !== "undefined") {
                __host_op_sync__(globalThis.__FM_RESET__, {});
            }
            this.calls = [];
            this.pendingRequests = [];
        }

        mockResponse(method, urlPattern, response, options) {
            options = options || {};
            let body = "";
            let status = options.status || 200;
            const headers = options.headers || {};
            const once = !!options.once;

            if (typeof response === "string") {
                body = response;
            } else if (typeof response === "function") {
                // Function-based responses (e.g. streams) are not supported;
                // they only appear in SSE ext tests which are not collected.
                return;
            } else if (response && typeof response === "object") {
                body = typeof response.body === "string" ? response.body : "";
                if (!options.status && response.status) {
                    status = response.status;
                }
            }

            const patternStr =
                urlPattern instanceof RegExp ? urlPattern.source : String(urlPattern);

            __host_op_sync__(globalThis.__FM_REGISTER__, {
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
            const patternStr =
                urlPattern instanceof RegExp ? urlPattern.source : String(urlPattern);
            __host_op_sync__(globalThis.__FM_REGISTER__, {
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
            const patternStr =
                urlPattern instanceof RegExp ? urlPattern.source : String(urlPattern);
            __host_op_sync__(globalThis.__FM_REGISTER__, {
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
            const patternStr =
                urlPattern instanceof RegExp ? urlPattern.source : String(urlPattern);
            const msg = error instanceof Error ? error.message : String(error);
            __host_op_sync__(globalThis.__FM_REGISTER__, {
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
            const patternStr =
                urlPattern instanceof RegExp ? urlPattern.source : String(urlPattern);

            const seqId = __host_op_sync__(globalThis.__FM_REGISTER_SEQ__, {
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
                    __host_op_async__(globalThis.__FM_NEXT__, { seq_id: seqId });
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
            return _origFetch(url, options);
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
