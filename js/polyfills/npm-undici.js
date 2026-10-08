// Dispatcher.dispatch() is never called: JSDOMDispatcher (jsdom-dispatcher.js) dispatches
// through the base dispatcher in fetch.js, which sends each hop over op_fetch_hop.
//
// .request() IS needed: fetchCollected() calls it directly, expecting undici's full
// package's wrapper installed on Dispatcher.prototype via its entry-point side effect.
// Importing jsdom's individual source files skips that side effect; the stub below
// reimplements just the {statusCode, headers, body: {bytes()}} shape fetchCollected reads.
export class Dispatcher {
    dispatch() {
        throw new Error("Dispatcher.dispatch: not implemented");
    }

    request(opts) {
        return new Promise((resolve, reject) => {
            let context = {};
            const chunks = [];
            let resolveBytes, rejectBytes;
            const bytes = new Promise((res, rej) => {
                resolveBytes = res;
                rejectBytes = rej;
            });
            this.dispatch(opts, {
                onRequestStart(_controller, ctx) {
                    context = ctx ?? {};
                },
                onResponseStart(_controller, statusCode, headers, statusText) {
                    resolve({
                        statusCode,
                        headers,
                        statusText,
                        context,
                        trailers: {},
                        body: { bytes: () => bytes },
                    });
                },
                onResponseData(_controller, chunk) {
                    chunks.push(chunk);
                },
                onResponseEnd() {
                    const total = chunks.reduce((n, c) => n + c.length, 0);
                    const out = new Uint8Array(total);
                    let offset = 0;
                    for (const c of chunks) {
                        out.set(c, offset);
                        offset += c.length;
                    }
                    resolveBytes(out);
                },
                onResponseError(_controller, err) {
                    rejectBytes(err);
                    reject(err);
                },
            });
        });
    }
}
// Forwards every callback to the wrapped handler, which is all jsdom's decompress interceptor
// needs from it. That interceptor sits in every dispatch chain, and the wire never reports a
// content-encoding it could act on (the transport strips it after decoding).
export class DecoratorHandler {
    #handler;

    constructor(handler) {
        this.#handler = handler;
    }

    onRequestStart(...args) {
        return this.#handler.onRequestStart?.(...args);
    }

    onRequestUpgrade(...args) {
        return this.#handler.onRequestUpgrade?.(...args);
    }

    onResponseStart(...args) {
        return this.#handler.onResponseStart?.(...args);
    }

    onResponseData(...args) {
        return this.#handler.onResponseData?.(...args);
    }

    onResponseEnd(...args) {
        return this.#handler.onResponseEnd?.(...args);
    }

    onResponseError(...args) {
        return this.#handler.onResponseError?.(...args);
    }
}
export class WebSocket {}
export function getGlobalDispatcher() {
    return new Dispatcher();
}
