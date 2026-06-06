(function () {
    globalThis.process = {
        platform: "linux",
        arch: "x64",
        env: {},
        version: "",
        versions: {},
    };

    globalThis.performance ??= {
        now: () => Date.now(),
        mark: () => {},
        measure: () => {},
        getEntriesByType: () => [],
        getEntriesByName: () => [],
    };

    const _stubTimer = (fn, d, ...a) => {
        try {
            fn(...a);
        } catch {}
        return 0;
    };
    globalThis.setTimeout ??= _stubTimer;
    globalThis.clearTimeout ??= () => {};
    globalThis.setInterval ??= _stubTimer;
    globalThis.clearInterval ??= () => {};
    globalThis.setImmediate ??= (fn, ...a) => setTimeout(fn, 0, ...a);
    globalThis.clearImmediate ??= () => {};

    // Base64 — not built into V8; needed by entities and node-buffer.js polyfill
    const _B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    globalThis.atob ??= (str) => {
        str = String(str).replace(/[\t\n\f\r =]/g, "");
        let out = "",
            i = 0;
        while (i < str.length) {
            const a = _B64.indexOf(str[i++]);
            const b = _B64.indexOf(str[i++]);
            const c = _B64.indexOf(str[i++]);
            const d = _B64.indexOf(str[i++]);
            out += String.fromCharCode((a << 2) | (b >> 4));
            if (c !== -1) out += String.fromCharCode(((b & 0xf) << 4) | (c >> 2));
            if (d !== -1) out += String.fromCharCode(((c & 0x3) << 6) | d);
        }
        return out;
    };
    globalThis.btoa ??= (str) => {
        let out = "";
        for (let i = 0; i < str.length; i += 3) {
            const a = str.charCodeAt(i),
                b = str.charCodeAt(i + 1),
                c = str.charCodeAt(i + 2);
            out += _B64[a >> 2];
            out += _B64[((a & 3) << 4) | (b >> 4)];
            out += isNaN(b) ? "=" : _B64[((b & 0xf) << 2) | (c >> 6)];
            out += isNaN(c) ? "=" : _B64[c & 0x3f];
        }
        return out;
    };

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
