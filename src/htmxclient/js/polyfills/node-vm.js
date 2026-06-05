class Script {
    constructor(code) {
        this.code = code;
    }
    runInContext(context) {
        const fn = new Function(this.code);
        fn.call(context);
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
