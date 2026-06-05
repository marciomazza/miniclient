// Minimal mocha-compatible test runner for jsrun
(function () {
    const _suites = [];
    let _cur = null;

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
    globalThis.beforeEach = (fn) => {
        if (_cur) _cur._be.push(fn);
    };
    globalThis.afterEach = (fn) => {
        if (_cur) _cur._ae.push(fn);
    };
    globalThis.before = (fn) => {};
    globalThis.after = (fn) => {};

    globalThis.__runAllTests = async function () {
        const results = [];
        for (const s of _suites) {
            for (const t of s.tests) {
                for (const be of s._be) await be();
                try {
                    await t.fn();
                    results.push({ suite: s.name, name: t.name, passed: true });
                } catch (e) {
                    results.push({
                        suite: s.name,
                        name: t.name,
                        passed: false,
                        error: String(e.message || e),
                    });
                }
                for (const ae of s._ae) await ae();
            }
        }
        return results;
    };
})();
