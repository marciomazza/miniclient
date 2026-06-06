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
