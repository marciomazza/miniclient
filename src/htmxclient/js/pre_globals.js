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

    globalThis.setTimeout ??= (fn, d, ...a) => {
        try {
            fn(...a);
        } catch {}
        return 0;
    };
    globalThis.clearTimeout ??= () => {};
    globalThis.setInterval ??= (fn, d, ...a) => {
        try {
            fn(...a);
        } catch {}
        return 0;
    };
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

    // TextEncoder / TextDecoder — not in jsrun's V8
    globalThis.TextEncoder ??= class TextEncoder {
        get encoding() {
            return "utf-8";
        }
        encode(str) {
            const bytes = [];
            for (let i = 0; i < str.length; i++) {
                const c = str.charCodeAt(i);
                if (c < 0x80) bytes.push(c);
                else if (c < 0x800) {
                    bytes.push(0xc0 | (c >> 6));
                    bytes.push(0x80 | (c & 0x3f));
                } else {
                    bytes.push(0xe0 | (c >> 12));
                    bytes.push(0x80 | ((c >> 6) & 0x3f));
                    bytes.push(0x80 | (c & 0x3f));
                }
            }
            return new Uint8Array(bytes);
        }
    };
    globalThis.TextDecoder ??= class TextDecoder {
        constructor(enc = "utf-8") {
            this.encoding = enc;
        }
        decode(buf) {
            if (!buf) return "";
            const arr = buf instanceof Uint8Array ? buf : new Uint8Array(buf.buffer ?? buf);
            let out = "";
            for (let i = 0; i < arr.length; ) {
                const b = arr[i++];
                if (b < 0x80) {
                    out += String.fromCharCode(b);
                } else if ((b & 0xe0) === 0xc0) {
                    out += String.fromCharCode(((b & 0x1f) << 6) | (arr[i++] & 0x3f));
                } else {
                    out += String.fromCharCode(
                        ((b & 0x0f) << 12) | ((arr[i++] & 0x3f) << 6) | (arr[i++] & 0x3f),
                    );
                }
            }
            return out;
        }
    };

    // XPathEvaluator stub — happy-dom has no XPath; htmx uses it only for hx-on:* attributes
    globalThis.XPathEvaluator ??= class XPathEvaluator {
        createExpression() {
            return { evaluate: () => ({ iterateNext: () => null }) };
        }
    };

    // Minimal Buffer stub — the real polyfill (node-buffer.js) replaces this when loaded
    globalThis.Buffer ??= {
        from: () => new Uint8Array(),
        isBuffer: () => false,
        alloc: (n) => new Uint8Array(n),
        concat: () => new Uint8Array(),
    };
})();
