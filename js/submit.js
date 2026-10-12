// @ts-check
import { deref } from "./element_registry.js";

// Form submission. Navigation primitives live in bootstrap.js.

// Runs `doAction(el)` against the element. Settle wait now happens natively in runtime.rs
// after every eval_async. Shared by Element.trigger()/click() (browser.py) and __mini.submit
// below.
/**
 * @param {number} handle
 * @param {(el: Element) => unknown} doAction
 */
export async function awaitSettle(handle, doAction) {
    const el = deref(handle);
    if (!el) throw new Error("Element not found (handle " + handle + ")");
    await doAction(el);
}

// Patched into jsdom's HTMLFormElement#requestSubmit (see patches/jsdom+30.1.2+001+fork.patch):
// jsdom already validates the form (novalidate/reportValidity()) and dispatches the
// `submit` event, respecting preventDefault() -- it just has no navigation of its own
// past that point. This runs for both __mini.submit below (explicit
// FormElement.requestSubmit(), browser.py) and a real click on a submit button/input
// (jsdom's own activation behaviour calls requestSubmit(submitter) internally too), so a
// plain, unintercepted form now navigates the same way regardless of which one
// triggered it.
// Every branch below ends in op_request_navigation -- the old isolate (and its timers)
// is dropped as part of that navigation, so there's nothing to clear here.
/**
 * @param {HTMLFormElement} form
 * @param {HTMLElement | null} [submitter]
 */
export function formSubmitFetch(form, submitter) {
    const method = form.method; // already normalized/defaulted by jsdom: "get" or "post"
    const enctype = form.enctype; // already normalized/defaulted to urlencoded
    const formData = new FormData(form, submitter); // includes submitter's name/value
    // Non-multipart encodings send a file entry as its filename, never as "[object File]".
    const pairs = [...formData].map(([k, v]) => [k, v instanceof File ? v.name : v]);

    async function navigate() {
        if (method === "get") {
            const url = new URL(form.action);
            url.search = new URLSearchParams(/** @type {any} */ (pairs)).toString();
            Deno.core.ops.op_request_navigation(url.href, "push");
            return;
        }

        let body, headers;
        if (enctype === "multipart/form-data") {
            // Same encoder as fetch(), so files keep their filename, type and bytes.
            const res = new Response(formData);
            body = new Uint8Array(await res.arrayBuffer());
            headers = { "content-type": /** @type {string} */ (res.headers.get("content-type")) };
        } else if (enctype === "text/plain") {
            body = new TextEncoder().encode(pairs.map(([k, v]) => `${k}=${v}\r\n`).join(""));
            headers = { "content-type": "text/plain" };
        } else {
            body = new TextEncoder().encode(
                new URLSearchParams(/** @type {any} */ (pairs)).toString(),
            );
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
}

// Drives jsdom's patched requestSubmit() (which already does validation, dispatch, and
// navigation); this only awaits settle.
/** @param {number} handle */
export async function submit(handle) {
    const el = deref(handle);
    if (!el) throw new Error("Element not found (handle " + handle + ")");
    if (el.tagName !== "FORM") {
        throw new Error("requestSubmit() only works on <form> elements (handle " + handle + ")");
    }
    /** @type {HTMLFormElement} */ (el).requestSubmit();
}
