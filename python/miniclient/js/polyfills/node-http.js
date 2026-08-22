import { Emitter } from "node:stream";
import { Buffer } from "node:buffer";

// Node merges repeated header names into a single comma-joined value on
// IncomingMessage.headers (used by Fetch.js only for the chunked-transfer check
// on 'transfer-encoding'/'content-length', neither of which repeats in practice).
function buildNodeHeaders(pairs) {
    const headers = {};
    for (const [name, value] of pairs) {
        const key = name.toLowerCase();
        headers[key] = key in headers ? `${headers[key]}, ${value}` : value;
    }
    return headers;
}

class ClientRequest extends Emitter {
    constructor(url, options) {
        super();
        this._url = url;
        this._options = options ?? {};
        this._chunks = [];
        this._id = crypto.randomUUID();
        this.destroyed = false;
    }

    write(chunk) {
        this._chunks.push(chunk);
        return true;
    }

    setTimeout() {}

    destroy(err) {
        if (this.destroyed) return;
        this.destroyed = true;
        __host_fetch_abort(this._id);
        if (err) this.emit("error", err);
    }

    end() {
        const body = this._chunks.length ? Buffer.concat(this._chunks) : undefined;
        __host_fetch({
            url: this._url,
            method: this._options.method ?? "GET",
            headers: this._options.headers ?? {},
            body,
            id: this._id,
        }).then(
            // A destroyed request already emitted its own 'error' from destroy();
            // don't act on whatever the now-irrelevant promise settles with.
            (res) => {
                if (!this.destroyed) this._respond(res);
            },
            (err) => {
                if (!this.destroyed) this.emit("error", err);
            },
        );
    }

    _respond(res) {
        // Dummy socket: only satisfies onSocket's chunked-transfer-edge-case listeners.
        this.emit("socket", new Emitter());

        const pairs = res.headers ?? [];
        const incoming = new Emitter();
        incoming.statusCode = res.status;
        incoming.statusMessage = res.statusText ?? "";
        incoming.rawHeaders = pairs.flat();
        incoming.headers = buildNodeHeaders(pairs);
        incoming.setTimeout = () => {};
        incoming.destroyed = false;
        incoming.destroy = () => {
            incoming.destroyed = true;
        };

        // Must emit 'response' before pushing body data: this is what lets
        // Fetch.js's onResponse attach its Stream.pipeline(incoming, ...) listeners
        // (synchronously, inside this same emit() call) before any data arrives.
        this.emit("response", incoming);

        const bodyBuffer = res.body != null ? Buffer.from(res.body) : Buffer.alloc(0);
        incoming.emit("data", bodyBuffer);
        incoming.emit("end");
    }
}

function request(url, options) {
    return new ClientRequest(url, options);
}

export { request };
export default { request };
