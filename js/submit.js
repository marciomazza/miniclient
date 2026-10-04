globalThis.__mini ??= {};

// Form submission. Navigation primitives live in bootstrap.js.
(function () {
    const __mini = globalThis.__mini;

    // Runs `doAction(el)` against the element. Settle wait now happens natively in runtime.rs
    // after every eval_async. Shared by Element.trigger()/click() (browser.py) and __mini.submit
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
    // FormElement.requestSubmit(), browser.py) and a real click on a submit button/input
    // (jsdom's own activation behaviour calls requestSubmit(submitter) internally too), so a
    // plain, unintercepted form now navigates the same way regardless of which one
    // triggered it.
    // Every branch below ends in op_request_navigation -- the old isolate (and its timers)
    // is dropped as part of that navigation, so there's nothing to clear here.
    __mini.formSubmitFetch = function (form, submitter) {
        const method = form.method; // already normalized/defaulted by jsdom: "get" or "post"
        const enctype = form.enctype; // already normalized/defaulted to urlencoded
        const formData = new FormData(form, submitter); // includes submitter's name/value

        async function navigate() {
            if (method === "get") {
                const url = new URL(form.action);
                url.search = new URLSearchParams(formData).toString();
                Deno.core.ops.op_request_navigation(url.href, "push");
                return;
            }

            let body, headers;
            if (enctype === "multipart/form-data") {
                const serialized = __mini.serializeMultipart(formData);
                body = serialized.body;
                headers = {
                    "content-type": `multipart/form-data; boundary=${serialized.boundary}`,
                };
            } else if (enctype === "text/plain") {
                body = [...formData].map(([k, v]) => `${k}=${v}\r\n`).join("");
                headers = { "content-type": "text/plain" };
            } else {
                body = new URLSearchParams(formData).toString();
                // Fetch's own URLSearchParams-body handling appends ";charset=UTF-8"; real
                // browsers send the bare mime type for form submissions.
                headers = { "content-type": "application/x-www-form-urlencoded" };
            }
            Deno.core.ops.op_request_navigation(
                new URL(form.action).href,
                "push",
                "post",
                body,
                headers,
            );
        }

        // Not awaited by the caller (jsdom's own requestSubmit() isn't async either) --
        // catch here so a bad enctype/URL surfaces on the console instead of as an
        // unhandled rejection. The actual self-fetch runs later, in the fresh realm
        // op_request_navigation navigates once this eval settles (runtime.rs).
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
