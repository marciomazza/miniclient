// happy-dom's module/inline-event-handler compilers still wrap their code in
// `(function anonymous($happy_dom) { ... })` before handing it to Script.runInContext.
// That wrapping makes top-level `var`/`function` declarations local to the wrapper
// function, so they never reach a global object — true of real Node's vm module too
// (verified against the real happy-dom package), not just this polyfill. Unwrap it and
// run the body via indirect eval, which executes as genuine global code, so
// declarations land on globalThis like a real browser <script> would.
const _HAPPY_DOM_SCRIPT_WRAPPER = /^\(function anonymous\(\$happy_dom\) \{([\s\S]*)\}\)$/;

class Script {
    constructor(code) {
        this.code = code;
    }
    runInContext(context) {
        // The classic-script compiler (JavaScriptCompiler.compile) no longer wraps its
        // output — it sets context.$happy_dom itself right before calling us and expects
        // genuine global-code eval, exactly what the unwrap branch below already does for
        // the wrapped callers. Bridge that context.$happy_dom onto globalThis (context and
        // globalThis are different objects here, unlike a real browser where window IS
        // globalThis) and eval the body directly, no unwrap needed.
        // ponytail: distinguishes this case from the wrapped callers by context already
        // carrying $happy_dom, which happy-dom's compiler sets synchronously just before
        // this call and never clears — fine as long as no other caller pre-sets that same
        // property on the same window. Switch to a compiler-passed flag if that changes.
        if (Object.prototype.hasOwnProperty.call(context, "$happy_dom")) {
            const hadGlobal = Object.prototype.hasOwnProperty.call(globalThis, "$happy_dom");
            const prevGlobal = globalThis.$happy_dom;
            globalThis.$happy_dom = context.$happy_dom;
            try {
                return (0, eval)(this.code);
            } finally {
                if (hadGlobal) globalThis.$happy_dom = prevGlobal;
                else delete globalThis.$happy_dom;
            }
        }
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
