// jsdom's createWindow() needs node:vm. mini runs one Window per V8 context, so the "context" is
// a plain object during construction, and page scripts run as global code on globalThis, which
// bootstrap.js has synced with the window's properties by then.
const constants = { DONT_CONTEXTIFY: Symbol("DONT_CONTEXTIFY") };
const _contexts = new WeakSet();

function createContext() {
    const ctx = {};
    _contexts.add(ctx);
    return ctx;
}

function isContext(ctx) {
    return _contexts.has(ctx);
}

// jsdom's own "give me this context's global proxy" bootstrap call -- there is no separate
// realm to proxy into, so the context object doubles as its own global proxy.
function runInContext(code, context, options) {
    if (code === "this") return context;
    // Compiled as a Script, not an indirect eval: a strict eval keeps its declarations to
    // itself, but a "use strict" classic script still declares globals.
    const filename = options?.filename;
    const specifier = filename && URL.canParse(filename) ? filename : "about:blank";
    const [result, error] = Deno.core.evalContext(code, specifier);
    if (error) throw error.thrown;
    return result;
}

export { constants, createContext, isContext, runInContext };
export default { constants, createContext, isContext, runInContext };
