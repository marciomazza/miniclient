globalThis.__zzz_load = function (html) {
    document.open();
    // document.write is deprecated for real browsers, but it's the only happy-dom API
    // that replaces the document and evaluates <script> tags natively.
    // Cast to `any` to silence tsserver's deprecation tag on the typed overload.
    /** @type {any} */ (document).write(html);
    document.close();
    __zzz_fixup_parsed_dom(document.body);
    if (typeof htmx !== "undefined") {
        htmx.process(document.body);
    }
};

// Fetches url and loads the response body as the new document. Shared by
// Browser.goto() (browser.py) and the plain-form-submission fallback below.
globalThis.__zzz_fetch_and_load = async function (url, options) {
    const r = await fetch(url, options);
    __zzz_load(await r.text());
};

// Runs `doAction(el)`, then resolves once htmx settles the request it triggered.
// If no request was triggered, calls `onNoRequest(el)` and resolves with its result
// (default: resolve with null). Shared by Element.trigger()/click()/submit() (browser.py)
// and __zzz_submit below.
globalThis.__zzz_await_htmx = function (handle, doAction, onNoRequest) {
    return new Promise((resolve, reject) => {
        let willRequest = false;
        document.addEventListener(
            "htmx:before:request",
            () => {
                willRequest = true;
            },
            { once: true },
        );
        document.addEventListener("htmx:finally:request", () => resolve(null), { once: true });
        document.addEventListener(
            "htmx:error",
            (e) => {
                reject(new Error("htmx:error — " + (e.detail?.error ?? e.detail?.ctx?.status)));
            },
            { once: true },
        );

        const el = __zzz_deref(handle);
        if (!el) {
            reject(new Error("Element not found (handle " + handle + ")"));
            return;
        }

        doAction(el);

        setTimeout(() => {
            if (!willRequest) {
                Promise.resolve(onNoRequest?.(el)).then(resolve).catch(reject);
            }
        }, 0);
    });
};

globalThis.__zzz_submit = function (handle) {
    let form, submitter;
    return __zzz_await_htmx(
        handle,
        (el) => {
            form = el.form ?? el.closest("form");
            if (!form) {
                throw new Error("No form found for handle " + handle);
            }
            submitter = el.tagName === "BUTTON" || el.tagName === "INPUT" ? el : null;
            form.dispatchEvent(
                new SubmitEvent("submit", { bubbles: true, cancelable: true, submitter }),
            );
        },
        async () => {
            // htmx didn't intercept => plain form submission
            const fd = new FormData(form, submitter);
            const method = (form.method || "get").toLowerCase();
            const action = form.action;
            let requestUrl = action;
            let options;
            if (method === "post") {
                options = {
                    method: "POST",
                    body: new URLSearchParams(fd).toString(),
                    headers: { "content-type": "application/x-www-form-urlencoded" },
                };
            } else {
                const params = new URLSearchParams(fd).toString();
                requestUrl = params ? action + "?" + params : action;
            }
            await __zzz_fetch_and_load(requestUrl, options);
        },
    );
};
