export default function apply(win) {
    const _NativeDOMParser = win.DOMParser;
    const _TABLE_CHILD_TAGS = new Set([
        "tr",
        "td",
        "th",
        "thead",
        "tbody",
        "tfoot",
        "col",
        "colgroup",
    ]);

    globalThis.DOMParser = class {
        parseFromString(str, type) {
            if (type === "text/html") {
                // happy-dom treats <body>...</body> as content inside body rather than as the body element itself.
                // Wrapping in <html> makes it parse correctly.
                if (/^\s*<body[\s>]/i.test(str)) str = "<html>" + str + "</html>";

                // happy-dom strips orphan table elements (tr, td, etc.) inside <template> because the
                // parsing context lacks table ancestry.  Re-parse those with a <table> wrapper and populate
                // template.content manually.
                const m = str.match(/^<template(\s[^>]*)?>([^]*)<\/template>$/i);
                if (m) {
                    const inner = m[2];
                    const firstTag = inner.match(/^\s*<([a-z][a-z0-9]*)/i)?.[1]?.toLowerCase();
                    if (_TABLE_CHILD_TAGS.has(firstTag)) {
                        const doc = new _NativeDOMParser().parseFromString(
                            "<html><head></head><body></body></html>",
                            "text/html",
                        );
                        const tmpl = doc.createElement("template");
                        doc.body.appendChild(tmpl);
                        const helper = doc.createElement("table");
                        // td/th need an extra <tr> wrapper to parse correctly in happy-dom
                        const needsRow = firstTag === "td" || firstTag === "th";
                        helper.innerHTML = needsRow ? `<tbody><tr>${inner}</tr></tbody>` : inner;
                        // tr → extract rows from tbody; td/th → extract cells from first row
                        const src =
                            firstTag === "tr"
                                ? helper.querySelector("tbody") || helper
                                : needsRow
                                  ? helper.querySelector("tr") || helper
                                  : helper;
                        for (const node of [...src.childNodes]) tmpl.content.appendChild(node);
                        return doc;
                    }
                }
            }
            return new _NativeDOMParser().parseFromString(str, type);
        }
    };
}
