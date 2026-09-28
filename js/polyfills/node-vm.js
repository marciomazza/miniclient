// jsdom's createWindow() needs vm.createContext/runInContext/isContext/constants to give
// window scripts (page <script> tags, inline hx-on handlers) a global scope with real
// var/function hoisting -- a `with` + eval shim can't do that, only genuine global-code eval
// can. Real Node gives each Window its own V8 context; this runtime has exactly one V8
// context per Runtime and mini only ever runs one Window per Runtime (no tabs), so there is
// nothing to actually isolate -- "the context" is a plain object during construction (kept
// distinct from globalThis so jsdom's own property setup during createWindow can't collide
// with bootstrap.js's later copy-onto-globalThis step), and real global code for everything
// else: by the time any page script actually runs, that copy step has already synced the
// window's properties onto globalThis, so indirect eval landing on globalThis (rather than a
// separate context) sees exactly the same state a real per-window realm would.
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
function runInContext(code, context) {
    if (code === "this") return context;
    return (0, eval)(code);
}

export { constants, createContext, isContext, runInContext };
export default { constants, createContext, isContext, runInContext };
