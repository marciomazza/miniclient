class PassThrough {
    constructor() {
        this._listeners = {};
        this._ended = false;
    }
    on(event, fn) {
        (this._listeners[event] ??= []).push(fn);
        return this;
    }
    emit(event, ...args) {
        (this._listeners[event] ?? []).forEach((fn) => fn(...args));
    }
    write(chunk) {
        this.emit("data", chunk);
    }
    end() {
        this._ended = true;
        this.emit("end");
    }
    pipe(dest) {
        return dest;
    }
}

function pipeline(...args) {
    const cb = args[args.length - 1];
    if (typeof cb === "function") cb(null);
}

export { PassThrough, pipeline };
export default { PassThrough, pipeline };
