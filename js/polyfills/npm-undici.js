// Dispatcher.dispatch() is never called: JSDOMDispatcher (jsdom-dispatcher.js,
// wired from bootstrap.js's `resources.interceptors`) intercepts first via op_fetch_hop.
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
export class DecoratorHandler {}
export class WebSocket {}
export function getGlobalDispatcher() {
    return new Dispatcher();
}
