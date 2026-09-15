// The real network client/pool/agent machinery goes unused here: mini's JSDOMDispatcher
// (jsdom-dispatcher.js, wired from bootstrap.js's `resources.interceptors`) always answers
// through mini's own op_fetch transport or a local resource mount, never real undici sockets --
// this stub's `dispatch()` is consequently never called, only inherited by JSDOMDispatcher.
//
// `.request()` IS needed: jsdom-dispatcher.js's `fetchCollected()` calls it directly on the
// dispatcher instance, expecting the promise-returning convenience wrapper real undici's full
// package provides by mixing an `api` object onto `Dispatcher.prototype` as an index.js side
// effect (`Object.assign(Dispatcher.prototype, api)`) -- a side effect this bundle skips by
// importing jsdom's individual source files instead of undici's package entry point. This is a
// minimal reimplementation: collect the handler-protocol callbacks (onRequestStart/
// onResponseStart/onResponseData/onResponseEnd/onResponseError) into the
// `{statusCode, headers, context, body: {bytes()}}` shape fetchCollected expects, with no
// pause/resume/streaming support since nothing here reads the body incrementally.
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
                    resolve({ statusCode, headers, statusText, context, trailers: {}, body: { bytes: () => bytes } });
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
export class DecoratorHandler {}
export class WebSocket {}
export function getGlobalDispatcher() {
    return new Dispatcher();
}
