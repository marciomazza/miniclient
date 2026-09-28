// Real XMLHttpRequest-impl.js does synchronous XHR via a Node worker thread
// (require.resolve("./xhr-sync-worker.js") at module scope, plus node:worker_threads) --
// neither resolves inside this bundle (no real filesystem/require, no worker threads), and
// nothing here constructs a real XMLHttpRequest anyway: htmx 4 and mini's own fetch() are
// both fetch-based. window.XMLHttpRequest still needs to exist as a constructor for
// feature-detection scripts; every actual method throws instead of pretending to work.
const XMLHttpRequestEventTargetImpl =
    require("../../node_modules/jsdom/lib/jsdom/living/events/EventTarget-impl").implementation;

const _unsupported = () => {
    throw new Error("XMLHttpRequest is not supported in this runtime; use fetch() instead");
};

class XMLHttpRequestImpl extends XMLHttpRequestEventTargetImpl {
    constructor(window) {
        super(window);
        this.readyState = 0;
        this.status = 0;
        this.statusText = "";
        this.responseURL = "";
        this.responseType = "";
        this.response = null;
        this.timeout = 0;
        this.withCredentials = false;
        this.upload = null;
    }

    open = _unsupported;
    setRequestHeader = _unsupported;
    send = _unsupported;
    abort = _unsupported;
    getResponseHeader = _unsupported;
    getAllResponseHeaders = _unsupported;
    overrideMimeType = _unsupported;
}

exports.implementation = XMLHttpRequestImpl;
