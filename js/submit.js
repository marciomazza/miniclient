globalThis.__mini ??= {};

// Form submission. Navigation primitives (fetchAndNavigate, navigateDocument, ...) live in
// navigate.js.
(function () {
    const __mini = globalThis.__mini;

    // Runs `doAction(el)` against the element. Settle wait now happens natively in runtime.rs
    // after every eval_async. Shared by Element.trigger()/click() (page.py) and __mini.submit
    // below.
    __mini.awaitSettle = async function (handle, doAction) {
        const el = __mini.deref(handle);
        if (!el) throw new Error("Element not found (handle " + handle + ")");
        await doAction(el);
    };

    // Patched into jsdom's HTMLFormElement#requestSubmit (see patches/jsdom+30.0.1.patch):
    // jsdom already validates the form (novalidate/reportValidity()) and dispatches the
    // `submit` event, respecting preventDefault() -- it just has no navigation of its own
    // past that point. This runs for both __mini.submit below (explicit
    // FormElement.requestSubmit(), page.py) and a real click on a submit button/input
    // (jsdom's own activation behaviour calls requestSubmit(submitter) internally too), so a
    // plain, unintercepted form now navigates the same way regardless of which one
    // triggered it.
    __mini.formSubmitFetch = function (form, submitter) {
        __clearAllTimers();
        const method = form.method; // already normalized/defaulted by jsdom: "get" or "post"
        const enctype = form.enctype; // already normalized/defaulted to urlencoded
        const formData = new FormData(form, submitter); // includes submitter's name/value

        async function navigate() {
            if (method === "get") {
                const url = new URL(form.action);
                url.search = new URLSearchParams(formData).toString();
                await __mini.fetchAndNavigate(url);
                return;
            }

            let body, headers;
            if (enctype === "multipart/form-data") {
                body = formData; // bootstrap.js's fetch() already serializes FormData bodies.
                headers = undefined;
            } else if (enctype === "text/plain") {
                body = [...formData].map(([k, v]) => `${k}=${v}\r\n`).join("");
                headers = { "content-type": "text/plain" };
            } else {
                body = new URLSearchParams(formData);
                // Fetch's own URLSearchParams-body handling appends ";charset=UTF-8"; real
                // browsers send the bare mime type for form submissions.
                headers = { "content-type": "application/x-www-form-urlencoded" };
            }
            const res = await fetch(form.action, { method: "post", body, headers });
            __mini.navigateDocument(await res.text(), res.url);
        }

        // Not awaited by the caller (jsdom's own requestSubmit() isn't async either) --
        // catch here so a network failure surfaces on the console instead of as an
        // unhandled rejection. The native event-loop drain (runtime.rs) waits for this
        // fetch regardless of who started it.
        navigate().catch((err) => console.error("form submission failed:", err));
    };

    // Drives jsdom's patched requestSubmit() (which already does validation, dispatch, and
    // navigation); this only awaits settle.
    __mini.submit = async function (handle) {
        const el = __mini.deref(handle);
        if (!el) throw new Error("Element not found (handle " + handle + ")");
        if (el.tagName !== "FORM") {
            throw new Error(
                "requestSubmit() only works on <form> elements (handle " + handle + ")",
            );
        }
        el.requestSubmit();
    };
})();
