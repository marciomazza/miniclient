class Buffer extends Uint8Array {
    static from(value, encodingOrOffset, length) {
        if (typeof value === "string") {
            const enc = encodingOrOffset ?? "utf8";
            if (enc === "utf8" || enc === "utf-8")
                return new Buffer(new TextEncoder().encode(value).buffer);
            if (enc === "base64") {
                const bin = atob(value);
                const bytes = new Uint8Array(bin.length);
                for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
                return new Buffer(bytes.buffer);
            }
            if (enc === "hex") {
                const bytes = new Uint8Array(value.length / 2);
                for (let i = 0; i < bytes.length; i++)
                    bytes[i] = parseInt(value.slice(i * 2, i * 2 + 2), 16);
                return new Buffer(bytes.buffer);
            }
        }
        if (value instanceof ArrayBuffer) return new Buffer(value);
        if (ArrayBuffer.isView(value))
            return new Buffer(value.buffer, value.byteOffset, value.byteLength);
        if (Array.isArray(value)) return new Buffer(new Uint8Array(value).buffer);
        return new Buffer(value);
    }
    static isBuffer(obj) {
        return obj instanceof Buffer;
    }
    static alloc(size, fill = 0) {
        const b = new Buffer(size);
        b.fill(fill);
        return b;
    }
    static concat(buffers, totalLength) {
        const len = totalLength ?? buffers.reduce((s, b) => s + b.length, 0);
        const result = new Buffer(len);
        let offset = 0;
        for (const b of buffers) {
            result.set(b, offset);
            offset += b.length;
        }
        return result;
    }
    toString(encoding = "utf8") {
        if (encoding === "utf8" || encoding === "utf-8") return new TextDecoder().decode(this);
        if (encoding === "base64") return btoa(String.fromCharCode(...this));
        if (encoding === "hex")
            return Array.from(this)
                .map((b) => b.toString(16).padStart(2, "0"))
                .join("");
        return new TextDecoder().decode(this);
    }
}
const Blob = globalThis.Blob;
globalThis.Buffer = Buffer;
export { Buffer, Blob };
export default { Buffer, Blob };
