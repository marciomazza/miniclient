(function () {
    // Real, circular-safe console (replaces deno_core's built-in one, which throws on any
    // circular arg -- htmx passes those). 00_url.js is force-loaded too: 01_console.js
    // lazy-loads it on first use, and only scripts touched here (the snapshot's cold pass)
    // survive into the snapshot's residual lazy table.
    {
        Deno.core.loadExtScript("ext:deno_web/00_url.js");
        const { Console } = Deno.core.loadExtScript("ext:deno_web/01_console.js");
        globalThis.console = new Console(Deno.core.print);
    }

    globalThis.process = {
        platform: "linux",
        arch: "x64",
        env: {},
        version: "",
        versions: {},
        argv: ["node"],
    };

    globalThis.performance ??= {
        now: () => Date.now(),
        mark: () => {},
        measure: () => {},
        getEntriesByType: () => [],
        getEntriesByName: () => [],
    };

    // TODO: add deno_crypto as a dependency and drop this polyfill.
    // Math.random()-backed, not cryptographically secure -- fine here since
    // this runtime has no native CSPRNG and only ever drives test/dev DOM code.
    // Must exist before happy-dom is imported: its BrowserWindow does
    // `import { webcrypto } from 'crypto'; crypto = webcrypto`, which resolves through
    // node-crypto.js's `globalThis.crypto` forwarding -- upgrade to a real RNG binding
    // in this runtime if this is ever used somewhere security-sensitive.
    globalThis.crypto ??= {
        getRandomValues(arr) {
            for (let i = 0; i < arr.length; i++) arr[i] = (Math.random() * 256) | 0;
            return arr;
        },
        randomUUID() {
            const b = [...globalThis.crypto.getRandomValues(new Uint8Array(16))];
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            const h = b.map((x) => x.toString(16).padStart(2, "0"));
            return [h.slice(0, 4), h.slice(4, 6), h.slice(6, 8), h.slice(8, 10), h.slice(10, 16)]
                .map((g) => g.join(""))
                .join("-");
        },
    };

    // Must exist before happy-dom is imported: its modules do
    // `globalThis.setTimeout.bind(globalThis)` at import time and keep that bound
    // reference forever, so happy-dom's internal timers run on whatever is defined here.
    // win.setTimeout (happy-dom) never fires in this runtime because its internal timer
    // queue is never drained; these replace it entirely.
    {
        let _nextId = 1;
        const _active = {}; // timerId -> true
        const _intervals = {}; // intervalId -> current timerId

        const _wait = (ms) => Deno.core.ops.op_sleep(ms || 0);

        globalThis.setTimeout = (fn, ms = 0, ...args) => {
            const id = _nextId++;
            _active[id] = true;
            _wait(ms).then(() => {
                if (_active[id]) {
                    delete _active[id];
                    fn(...args);
                }
            });
            return id;
        };

        globalThis.clearTimeout = (id) => {
            if (id != null) delete _active[id];
        };

        globalThis.setInterval = (fn, ms = 0, ...args) => {
            const intervalId = _nextId++;
            const schedule = () => {
                const timerId = _nextId++;
                _intervals[intervalId] = timerId;
                _active[timerId] = true;
                _wait(ms).then(() => {
                    delete _active[timerId];
                    if (intervalId in _intervals) {
                        fn(...args);
                        schedule();
                    }
                });
            };
            schedule();
            return intervalId;
        };

        globalThis.clearInterval = (intervalId) => {
            const timerId = _intervals[intervalId];
            delete _intervals[intervalId];
            if (timerId != null) delete _active[timerId];
        };

        // Silently drop all pending timer callbacks — used between tests to prevent
        // stale timers from a previous test firing into the next one's context.
        globalThis.__clearAllTimers = () => {
            for (const k of Object.keys(_active)) delete _active[k];
            for (const k of Object.keys(_intervals)) delete _intervals[k];
        };
    }
    globalThis.setImmediate ??= (fn, ...a) => setTimeout(fn, 0, ...a);
    globalThis.clearImmediate ??= () => {};

    // Base64 — not built into V8; needed by entities and node-buffer.js polyfill.
    // Force-loaded here (like 00_url.js) so it survives into the snapshot.
    {
        const { atob, btoa } = Deno.core.loadExtScript("ext:deno_web/05_base64.js");
        globalThis.atob = atob;
        globalThis.btoa = btoa;
    }

    // XPathEvaluator backed by the xpath npm package (XPath 1.0).
    // Loaded via __xpathLib injected before this script runs.
    globalThis.XPathEvaluator = class XPathEvaluator {
        createExpression(expr) {
            const compiled = globalThis.__xpathLib.parse(expr);
            return {
                evaluate(ctx) {
                    const nodes = compiled.select({ node: ctx });
                    let i = 0;
                    return { iterateNext: () => nodes[i++] ?? null };
                },
            };
        }
    };

    // Minimal Buffer stub — the real polyfill (node-buffer.js) replaces this when loaded
    globalThis.Buffer ??= {
        from: () => new Uint8Array(),
        isBuffer: () => false,
        alloc: (n) => new Uint8Array(n),
        concat: () => new Uint8Array(),
    };

    globalThis.CSS = {
        escape(value) {
            value = String(value);
            if (value.length === 0) return value;
            let out = "";
            for (let i = 0; i < value.length; i++) {
                const c = value.charCodeAt(i);
                if (c === 0) {
                    out += "�";
                    continue;
                }
                if ((c >= 0x0001 && c <= 0x001f) || c === 0x007f) {
                    out += "\\" + c.toString(16) + " ";
                    continue;
                }
                if (i === 0 && c >= 0x0030 && c <= 0x0039) {
                    out += "\\" + c.toString(16) + " ";
                    continue;
                }
                if (i === 1 && c >= 0x0030 && c <= 0x0039 && value.charCodeAt(0) === 0x002d) {
                    out += "\\" + c.toString(16) + " ";
                    continue;
                }
                if (i === 0 && value.length === 1 && c === 0x002d) {
                    out += "\\" + value[i];
                    continue;
                }
                if (
                    c >= 0x0080 ||
                    c === 0x002d ||
                    c === 0x005f ||
                    (c >= 0x0030 && c <= 0x0039) ||
                    (c >= 0x0041 && c <= 0x005a) ||
                    (c >= 0x0061 && c <= 0x007a)
                ) {
                    out += value[i];
                    continue;
                }
                out += "\\" + value[i];
            }
            return out;
        },
    };
})();
