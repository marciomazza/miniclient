class Emitter {
    constructor() {
        this._listeners = {};
    }
    on(event, fn) {
        (this._listeners[event] ??= []).push(fn);
        return this;
    }
    prependListener(event, fn) {
        (this._listeners[event] ??= []).unshift(fn);
        return this;
    }
    once(event, fn) {
        const wrapped = (...args) => {
            this.removeListener(event, wrapped);
            fn(...args);
        };
        return this.on(event, wrapped);
    }
    removeListener(event, fn) {
        this._listeners[event] = (this._listeners[event] ?? []).filter((l) => l !== fn);
        return this;
    }
    emit(event, ...args) {
        for (const fn of (this._listeners[event] ?? []).slice()) fn(...args);
    }
}

class PassThrough extends Emitter {
    constructor() {
        super();
        this.destroyed = false;
    }
    write(chunk) {
        this.emit("data", chunk);
        return true;
    }
    end() {
        this.emit("end");
    }
    destroy(err) {
        if (this.destroyed) return;
        this.destroyed = true;
        if (err) this.emit("error", err);
    }
}

// Pumps a single src -> dest pair, resolving when the source is exhausted.
// src is either a WHATWG ReadableStream (has getReader, e.g. a fetch Request body)
// or a Node-style event-emitting stream (has on('data'|'end'|'error')).
function pump(src, dest) {
    return new Promise((resolve, reject) => {
        const fail = (err) => {
            dest.destroy?.(err);
            reject(err);
        };
        if (typeof src.getReader === "function") {
            const reader = src.getReader();
            (async () => {
                try {
                    for (;;) {
                        const { done, value } = await reader.read();
                        if (done) break;
                        dest.write(value);
                    }
                    dest.end();
                    resolve();
                } catch (err) {
                    fail(err);
                }
            })();
        } else {
            src.on("data", (chunk) => dest.write(chunk));
            src.on("end", () => {
                dest.end();
                resolve();
            });
            src.on("error", fail);
        }
    });
}

// ponytail: only chains src -> dest pairs sequentially, no real Transform support —
// nothing in this codebase's Fetch usage needs one (gzip/deflate response bodies
// never reach here, see the content-encoding stripping in runtime.py's _fetch_op_impl).
function pipeline(...args) {
    const cb = typeof args[args.length - 1] === "function" ? args.pop() : null;
    const streams = args;
    (async () => {
        try {
            for (let i = 0; i < streams.length - 1; i++) {
                await pump(streams[i], streams[i + 1]);
            }
            cb?.(null);
        } catch (err) {
            cb?.(err);
        }
    })();
    return streams[streams.length - 1];
}

// Stub, never instantiated: only referenced by undici's `InflateStream extends Transform`
// (gzip/deflate decoding), the same dead path the `pipeline()` comment above already covers --
// content-encoding is stripped server-side, so this class body is unreachable at runtime.
class Transform extends Emitter {}

// undici's body-extraction code checks these on the WHATWG `ReadableStream` it just created
// itself (deno_web's, not this file's PassThrough), to bail out of writing more chunks once a
// consumer errors or cancels it. `locked` is the only piece of that state a plain ReadableStream
// exposes publicly; a stream that's been errored gets cancelled/unlocked too, so this is a safe
// approximation, not a full port of Node's internal stream-state introspection.
function isErrored() {
    return false;
}
function isDisturbed(stream) {
    return !!stream?.locked;
}

export { Emitter, PassThrough, Transform, pipeline, isErrored, isDisturbed };
export default { PassThrough, Transform, pipeline, isErrored, isDisturbed };
