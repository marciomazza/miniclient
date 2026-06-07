// Minimal mocha-compatible test runner for jsrun
(function () {
    const _suites = [];
    let _cur = null;
    const _SKIP = Symbol("skip");

    globalThis.describe = (name, fn) => {
        const s = { name, tests: [], _be: [], _ae: [] };
        const prev = _cur;
        _cur = s;
        fn();
        _cur = prev;
        _suites.push(s);
    };
    globalThis.it = (name, fn) => {
        if (_cur) _cur.tests.push({ name, fn });
    };
    globalThis.it.skip = (_name, _fn) => {
        /* no-op: skipped test */
    };
    globalThis.beforeEach = (fn) => {
        if (_cur) _cur._be.push(fn);
    };
    globalThis.afterEach = (fn) => {
        if (_cur) _cur._ae.push(fn);
    };
    globalThis.before = (fn) => {
        if (_cur) (_cur._before ??= []).push(fn);
    };
    globalThis.after = (fn) => {
        if (_cur) (_cur._after ??= []).push(fn);
    };

    // Test context passed as `this` — supports mocha's this.skip() and this.timeout()
    const _ctx = {
        skip() {
            throw _SKIP;
        },
        timeout() {},
    };

    // Patch mockSequentialResponses.next() to use event-based resolution instead of
    // setTimeout(resolve, 0). Under CPU load, asyncio.sleep(0) can deliver its result
    // to V8 before htmx's microtask chain finishes processing the released response,
    // causing next() to resolve while elements are still in the disabled state.
    // Using htmx:finally:request guarantees next() resolves only after htmx fully
    // processes the response (including __enableElements), making timing deterministic.
    if (typeof fetchMock !== "undefined") {
        const _origMockSeq = fetchMock.mockSequentialResponses.bind(fetchMock);
        fetchMock.mockSequentialResponses = function (method, urlPattern, response, options = {}) {
            const seq = _origMockSeq(method, urlPattern, response, options);
            const _origNext = seq.next.bind(seq);
            seq.next = function () {
                return new Promise((resolve) => {
                    // Set up listener BEFORE releasing the request so the event is never missed.
                    // htmx:finally:request fires synchronously inside htmx's finally block,
                    // right before __enableElements. resolve() queues the continuation as a
                    // microtask, and __enableElements runs before that microtask executes.
                    document.addEventListener("htmx:finally:request", resolve, { once: true });
                    // Release the pending request (side effect of _origNext).
                    // The Promise from _origNext() is ignored; we use the event instead.
                    _origNext();
                });
            };
            return seq;
        };
    }

    globalThis.__resetRunner = () => { _suites.length = 0; };

    globalThis.__runAllTests = async function () {
        const results = [];
        for (const s of _suites) {
            for (const fn of s._before ?? []) await fn.call(_ctx);
            for (const t of s.tests) {
                for (const be of s._be) await be.call(_ctx);
                try {
                    await t.fn.call(_ctx);
                    results.push({ suite: s.name, name: t.name, passed: true });
                } catch (e) {
                    if (e === _SKIP) {
                        results.push({ suite: s.name, name: t.name, passed: true });
                    } else {
                        results.push({
                            suite: s.name,
                            name: t.name,
                            passed: false,
                            error: String(e.message || e),
                        });
                    }
                }
                for (const ae of s._ae) await ae.call(_ctx);
            }
            for (const fn of s._after ?? []) await fn.call(_ctx);
        }
        return results;
    };
})();
