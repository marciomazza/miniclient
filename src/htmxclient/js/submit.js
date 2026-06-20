globalThis.__zzz_submit = function (selector) {
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

        const el = document.querySelector(selector);
        if (!el) {
            reject(new Error("Element not found: " + selector));
            return;
        }

        const form = el.form ?? el.closest("form");
        if (!form) {
            reject(new Error("No form found for: " + selector));
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
                    p = fetch(params ? action + "?" + params : action);
                }
                p.then((r) => r.text())
                    .then(resolve)
                    .catch(reject);
            }
        }, 0);
    });
};
