import { Buffer } from "node:buffer";

function execFileSync(_file, args, _options) {
    // args[1] is the envelope object our patched SyncFetchScriptBuilder.getScript
    // hands us directly (see patch-happy-dom.js) — no serialization on this side,
    // since both ends are our own code sharing the same JS heap.
    const envelope = args[1];
    if (!envelope?.__sync_fetch__) {
        return Buffer.from(JSON.stringify({ error: "unsupported script", incomingMessage: null }));
    }
    try {
        const body = envelope.body ?? undefined;
        const res = __host_fetch_sync({
            url: envelope.url,
            method: envelope.method,
            headers: envelope.headers ?? {},
            body,
        });
        const data = res.body != null ? Buffer.from(res.body).toString("base64") : "";
        return Buffer.from(
            JSON.stringify({
                error: null,
                incomingMessage: {
                    statusCode: res.status,
                    statusMessage: res.statusText ?? "",
                    rawHeaders: (res.headers ?? []).flat(),
                    data,
                },
            }),
        );
    } catch (err) {
        return Buffer.from(
            JSON.stringify({ error: String(err?.message ?? err), incomingMessage: null }),
        );
    }
}

export { execFileSync };
export default { execFileSync };
