// happy-dom wraps every <script> body in `(function anonymous($happy_dom) { ... })`
// before handing it to Script.runInContext (see JavaScriptCompiler.compile). That
// wrapping makes the script's top-level `var`/`function` declarations local to that
// wrapper function, so they never reach a global object — true of real Node's vm
// module too (verified against the real happy-dom package), not just this polyfill.
// Unwrap it and run the body via indirect eval, which executes as genuine global
// code, so declarations land on globalThis like a real browser <script> would.
const _HAPPY_DOM_SCRIPT_WRAPPER = /^\(function anonymous\(\$happy_dom\) \{([\s\S]*)\}\)$/;

class Script {
    constructor(code) {
        this.code = code;
    }
    runInContext(context) {
        const wrapped = _HAPPY_DOM_SCRIPT_WRAPPER.exec(this.code);
        if (!wrapped) {
            return new Function("return " + this.code).call(context);
        }
        const body = wrapped[1];
        return function ($happy_dom) {
            globalThis.$happy_dom = $happy_dom;
            try {
                return (0, eval)(body);
            } finally {
                delete globalThis.$happy_dom;
            }
        };
    }
}
const _sym = Symbol("context");
const isContext = (ctx) => ctx[_sym] === true;
const createContext = (ctx) => {
    ctx[_sym] = true;
    return ctx;
};
export { Script, isContext, createContext };
export default { Script, isContext, createContext };
