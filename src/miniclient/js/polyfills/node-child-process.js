import { Buffer } from "node:buffer";

function execFileSync(_file, args, _options) {
    let envelope;
    try {
        envelope = JSON.parse(args[1]);
    } catch {
        envelope = null;
    }
    if (!envelope?.__sync_fetch__) {
        return Buffer.from(JSON.stringify({ error: "unsupported script", incomingMessage: null }));
    }
    try {
        const body = envelope.body != null ? Buffer.from(envelope.body, "base64") : undefined;
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
