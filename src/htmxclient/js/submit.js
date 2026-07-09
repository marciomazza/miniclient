globalThis.__zzz_submit = function (handle) {
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

        const form = el.form ?? el.closest("form");
        if (!form) {
            reject(new Error("No form found for handle " + handle));
            return;
        }
        const submitter = el.tagName === "BUTTON" || el.tagName === "INPUT" ? el : null;

        form.dispatchEvent(
            new SubmitEvent("submit", {
                bubbles: true,
                cancelable: true,
                submitter,
            }),
        );

        setTimeout(() => {
            if (!willRequest) {
                // htmx didn't intercept — plain form submission
                const fd = new FormData(form, submitter);
                const method = (form.method || "get").toLowerCase();
                const action = form.action;
                let requestUrl = action;
                let p;
                if (method === "post") {
                    p = fetch(action, {
                        method: "POST",
                        body: new URLSearchParams(fd).toString(),
                        headers: {
                            "content-type": "application/x-www-form-urlencoded",
                        },
                    });
                } else {
                    const params = new URLSearchParams(fd).toString();
                    requestUrl = params ? action + "?" + params : action;
                    p = fetch(requestUrl);
                }
                p.then((r) =>
                    r.text().then((text) => {
                        document.documentElement.innerHTML = text;
                        htmx.process(document.body);
                        return {
                            status: r.status,
                            ok: r.ok,
                            url: requestUrl,
                            headers: Object.fromEntries(r.headers.entries()),
                            text,
                        };
                    }),
                )
                    .then(resolve)
                    .catch(reject);
            }
        }, 0);
    });
};
