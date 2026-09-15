// Only jsdom's VirtualConsole needs this (`new EventEmitter()` + `.on("error", ...)` in its
// constructor, so it never hits the throw-on-unheard-"error" branch below in practice).
class EventEmitter {
    #listeners = new Map();

    on(event, fn) {
        const fns = this.#listeners.get(event) ?? [];
        fns.push(fn);
        this.#listeners.set(event, fns);
        return this;
    }

    once(event, fn) {
        const wrapper = (...args) => {
            this.off(event, wrapper);
            fn(...args);
        };
        return this.on(event, wrapper);
    }

    off(event, fn) {
        const fns = this.#listeners.get(event);
        if (fns)
            this.#listeners.set(
                event,
                fns.filter((f) => f !== fn),
            );
        return this;
    }

    emit(event, ...args) {
        const fns = this.#listeners.get(event);
        if (!fns || fns.length === 0) {
            if (event === "error") throw args[0] ?? new Error("Unhandled error event");
            return false;
        }
        for (const fn of fns) fn(...args);
        return true;
    }
}
EventEmitter.prototype.removeListener = EventEmitter.prototype.off;

// undici calls this only with a real `AbortSignal` (deno_web's, not this file's EventEmitter),
// so it always takes the `addEventListener` branch -- the `signal.once('abort', ...)` fallback
// Node's real implementation has for plain EventEmitters is dead code here.
function addAbortListener(signal, listener) {
    signal.addEventListener("abort", listener, { once: true });
    return () => signal.removeEventListener("abort", listener);
}

export { EventEmitter, addAbortListener };
export default { EventEmitter, addAbortListener };
