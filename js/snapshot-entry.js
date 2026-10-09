// @ts-check
// Entry point of the bundle that runs first in the snapshot build. Import order is evaluation
// order: pre_globals installs the polyfills the later files, and jsdom's bundle after them,
// read at load time.
import "./pre_globals.js";
import { ref, deref } from "./element_registry.js";
import { awaitSettle, formSubmitFetch, submit } from "./submit.js";
import { baseDispatcher, makeFetch } from "./fetch.js";
import {
    contentTypeFor,
    installInnerHtmlRadioFix,
    installInnerText,
    installXPathShortCircuit,
    serializeMultipart,
} from "./mini_helpers.js";

// The Python<->JS bridge surface. bootstrap.js adds the navigation members once per boot.
globalThis.__mini = /** @type {Mini} */ (
    /** @type {unknown} */ ({
        ref,
        deref,
        awaitSettle,
        formSubmitFetch,
        submit,
        baseDispatcher,
        makeFetch,
        contentTypeFor,
        serializeMultipart,
        installInnerHtmlRadioFix,
        installInnerText,
        installXPathShortCircuit,
    })
);
