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

// jsdom's own dispatcher (data:/file: URL bodies, and synthetic Responses from a
// requestInterceptor) wraps its output in one of these -- only the read side matters here,
// since mini never writes to a Readable of its own. Backpressure (pause/resume) is tracked
// but not enforced: nothing produces chunks faster than `stream-handler.js` consumes them.
class Readable extends Emitter {
    constructor() {
        super();
        this.destroyed = false;
        this.readableEnded = false;
        this.errored = null;
        this._paused = false;
    }
    isPaused() {
        return this._paused;
    }
    pause() {
        this._paused = true;
        return this;
    }
    resume() {
        this._paused = false;
        return this;
    }
    destroy(err) {
        if (this.destroyed) return this;
        this.destroyed = true;
        if (err) {
            this.errored = err;
            this.emit("error", err);
        }
        this.emit("close");
        return this;
    }
    _finish() {
        this.readableEnded = true;
        this.emit("close");
    }
    // `iterable` is always a plain array of whole-body chunks here (jsdom hands
    // `Readable.from([wholeBody])` or `Readable.from([])`), never a real generator.
    static from(iterable) {
        const r = new Readable();
        queueMicrotask(() => {
            for (const chunk of iterable) {
                if (r.destroyed) return;
                r.emit("data", chunk);
            }
            if (!r.destroyed) r._finish();
        });
        return r;
    }
    // `webStream` is the WHATWG ReadableStream backing a synthetic Response body
    // (requestInterceptor's return value); mini's own Response never streams incrementally.
    static fromWeb(webStream) {
        const r = new Readable();
        (async () => {
            const reader = webStream.getReader();
            try {
                for (;;) {
                    const { done, value } = await reader.read();
                    if (done) break;
                    if (r.destroyed) return;
                    r.emit("data", value);
                }
                if (!r.destroyed) r._finish();
            } catch (err) {
                r.destroy(err);
            }
        })();
        return r;
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

// Sequential src -> dest chain only: pump() can wire a Node-style source to a PassThrough
// sink, but can't apply a true Transform in the middle (no _transform/_flush hooks).
// undici's pipeline() callers in this bundle (eventsource, fetch decoder chain, HTTP/2
// dispatcher, decompress interceptor) all fit src->dest in practice; the gzip/deflate
// Transform paths aren't reachable because runtime.py's _fetch_op_impl strips
// content-encoding before undici sees the response body.
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

// Stub for undici's `class Foo extends Transform` declarations
// (eventsource-stream.js:95, web/fetch/util.js:1248: InflateStream). The class body must
// exist at module-load for the `extends` clauses to parse; no Transform instance is ever
// constructed in mini's runtime (content-encoding is stripped server-side, so the
// gzip/deflate decoder path is dead), but the subclasses would fail to load without a
// constructor here. Same role as node-http.js: file needed, contents aren't exercised.
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

export { Emitter, PassThrough, Readable, Transform, pipeline, isErrored, isDisturbed };
export default { PassThrough, Readable, Transform, pipeline, isErrored, isDisturbed };
