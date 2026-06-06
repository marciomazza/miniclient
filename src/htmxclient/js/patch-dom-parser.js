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

    // Extract all top-level <template>…</template> from html string, replacing each with an
    // empty placeholder <template data-__tmpl__="N">. Nesting is handled by tracking depth.
    // Returns { html: modified string, map: Map<id, innerHtml> }.
    function extractTemplates(html) {
        const map = new Map();
        let counter = 0;
        const parts = [];
        let i = 0;

        while (i < html.length) {
            const lt = html.indexOf("<", i);
            if (lt === -1) {
                parts.push(html.slice(i));
                break;
            }

            if (!/^<template[\s>]/i.test(html.slice(lt))) {
                parts.push(html.slice(i, lt + 1));
                i = lt + 1;
                continue;
            }

            parts.push(html.slice(i, lt));

            const gtPos = html.indexOf(">", lt);
            if (gtPos === -1) {
                parts.push(html.slice(lt));
                i = html.length;
                break;
            }

            const openTag = html.slice(lt, gtPos + 1);
            let depth = 1;
            let j = gtPos + 1;
            let innerEnd = -1;
            let closeEnd = -1;

            while (j < html.length && depth > 0) {
                if (/^<template[\s>]/i.test(html.slice(j))) {
                    const ng = html.indexOf(">", j);
                    if (ng === -1) break;
                    depth++;
                    j = ng + 1;
                } else if (/^<\/template>/i.test(html.slice(j))) {
                    depth--;
                    if (depth === 0) {
                        innerEnd = j;
                        closeEnd = j + "</template>".length;
                    } else {
                        j += "</template>".length;
                    }
                } else {
                    j++;
                }
            }

            if (innerEnd === -1) {
                parts.push(html.slice(lt));
                i = html.length;
                break;
            }

            const key = String(counter++);
            map.set(key, html.slice(gtPos + 1, innerEnd));
            const attrs = openTag.slice("<template".length, -1);
            parts.push(`<template${attrs} data-__tmpl__="${key}"></template>`);
            i = closeEnd;
        }

        return { html: parts.join(""), map };
    }

    function tableSource(helper, firstTag) {
        if (firstTag === "td" || firstTag === "th") return helper.querySelector("tr") || helper;
        if (firstTag === "tr") return helper.querySelector("tbody") || helper;
        return helper;
    }

    // Populate tmpl.content from innerHtml, using the appropriate wrapper for table orphans.
    // Recursively repairs nested templates found in map.
    function repairTemplate(tmpl, innerHtml, doc) {
        const { html: processed, map } = extractTemplates(innerHtml);

        const firstTag = processed.match(/^\s*<([a-z][a-z0-9]*)/i)?.[1]?.toLowerCase();
        let src;
        if (_TABLE_CHILD_TAGS.has(firstTag)) {
            const helper = doc.createElement("table");
            const needsRow = firstTag === "td" || firstTag === "th";
            helper.innerHTML = needsRow ? `<tbody><tr>${processed}</tr></tbody>` : processed;
            src = tableSource(helper, firstTag);
        } else {
            const helper = doc.createElement("div");
            helper.innerHTML = processed;
            src = helper;
        }

        for (const node of [...src.childNodes]) tmpl.content.appendChild(node);
        tmpl.removeAttribute("data-__tmpl__");

        for (const nested of [...tmpl.content.querySelectorAll("template[data-__tmpl__]")]) {
            const key = nested.getAttribute("data-__tmpl__");
            if (map.has(key)) repairTemplate(nested, map.get(key), doc);
        }
    }

    globalThis.DOMParser = class {
        parseFromString(str, type) {
            if (type === "text/html") {
                // happy-dom treats <body>…</body> as content inside body rather than as the body element.
                if (/^\s*<body[\s>]/i.test(str)) str = "<html>" + str + "</html>";

                if (!/<template[\s>]/i.test(str))
                    return new _NativeDOMParser().parseFromString(str, type);

                const { html: processed, map } = extractTemplates(str);
                const doc = new _NativeDOMParser().parseFromString(processed, type);

                for (const tmpl of [...doc.querySelectorAll("template[data-__tmpl__]")]) {
                    const key = tmpl.getAttribute("data-__tmpl__");
                    if (map.has(key)) repairTemplate(tmpl, map.get(key), doc);
                }

                return doc;
            }
            return new _NativeDOMParser().parseFromString(str, type);
        }
    };
}
