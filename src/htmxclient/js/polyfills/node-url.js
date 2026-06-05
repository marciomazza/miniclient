class URLSearchParams {
    constructor(init) {
        this._params = [];
        if (!init) return;
        if (typeof init === "string") {
            const s = init.startsWith("?") ? init.slice(1) : init;
            for (const pair of s.split("&")) {
                if (!pair) continue;
                const idx = pair.indexOf("=");
                const k = decodeURIComponent(idx < 0 ? pair : pair.slice(0, idx)).replace(
                    /\+/g,
                    " ",
                );
                const v =
                    idx < 0 ? "" : decodeURIComponent(pair.slice(idx + 1)).replace(/\+/g, " ");
                this._params.push([k, v]);
            }
        } else if (Array.isArray(init)) {
            for (const [k, v] of init) this._params.push([String(k), String(v)]);
        } else if (init && typeof init === "object") {
            for (const [k, v] of Object.entries(init)) this._params.push([String(k), String(v)]);
        }
    }
    append(k, v) {
        this._params.push([String(k), String(v)]);
    }
    delete(k) {
        this._params = this._params.filter(([p]) => p !== k);
    }
    get(k) {
        return this._params.find(([p]) => p === k)?.[1] ?? null;
    }
    getAll(k) {
        return this._params.filter(([p]) => p === k).map(([, v]) => v);
    }
    has(k) {
        return this._params.some(([p]) => p === k);
    }
    set(k, v) {
        const i = this._params.findIndex(([p]) => p === k);
        if (i < 0) this._params.push([k, v]);
        else {
            this._params[i] = [k, String(v)];
            this._params = this._params.filter(([p], j) => p !== k || j === i);
        }
    }
    entries() {
        return this._params[Symbol.iterator]();
    }
    keys() {
        return this._params.map(([k]) => k)[Symbol.iterator]();
    }
    values() {
        return this._params.map(([, v]) => v)[Symbol.iterator]();
    }
    [Symbol.iterator]() {
        return this.entries();
    }
    forEach(cb) {
        for (const [k, v] of this._params) cb(v, k, this);
    }
    sort() {
        this._params.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    }
    toString() {
        return this._params
            .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`)
            .join("&");
    }
    get size() {
        return this._params.length;
    }
}

const _URL_RE = /^([a-zA-Z][a-zA-Z0-9+\-.]*):(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/;

class URL {
    constructor(url, base) {
        const href = base ? _resolveRelative(String(url), String(base)) : String(url);
        const m = _URL_RE.exec(href);
        if (!m) throw new TypeError(`Invalid URL: ${href}`);
        this._protocol = m[1].toLowerCase() + ":";
        const authority = m[2] ?? "";
        let userinfo = "",
            host = authority;
        const atIdx = authority.lastIndexOf("@");
        if (atIdx >= 0) {
            userinfo = authority.slice(0, atIdx);
            host = authority.slice(atIdx + 1);
        }
        const colonIdx = host.lastIndexOf(":");
        const bracketClose = host.lastIndexOf("]");
        if (colonIdx > bracketClose) {
            this._hostname = host.slice(0, colonIdx);
            this._port = host.slice(colonIdx + 1);
        } else {
            this._hostname = host;
            this._port = "";
        }
        const ci = userinfo.indexOf(":");
        this._username = ci < 0 ? userinfo : userinfo.slice(0, ci);
        this._password = ci < 0 ? "" : userinfo.slice(ci + 1);
        this._pathname = m[3] || "/";
        this._search = m[4] ?? "";
        this._hash = m[5] ?? "";
        this._sp = null;
    }
    get protocol() {
        return this._protocol;
    }
    set protocol(v) {
        this._protocol = String(v).replace(/:?$/, ":");
    }
    get hostname() {
        return this._hostname;
    }
    set hostname(v) {
        this._hostname = String(v);
    }
    get port() {
        return this._port;
    }
    set port(v) {
        this._port = String(v);
    }
    get host() {
        return this._port ? `${this._hostname}:${this._port}` : this._hostname;
    }
    set host(v) {
        const i = v.lastIndexOf(":");
        if (i < 0) {
            this._hostname = v;
            this._port = "";
        } else {
            this._hostname = v.slice(0, i);
            this._port = v.slice(i + 1);
        }
    }
    get pathname() {
        return this._pathname;
    }
    set pathname(v) {
        this._pathname = String(v);
    }
    get search() {
        return this._search;
    }
    set search(v) {
        this._search = v ? (v.startsWith("?") ? v : `?${v}`) : "";
        this._sp = null;
    }
    get hash() {
        return this._hash;
    }
    set hash(v) {
        this._hash = v ? (v.startsWith("#") ? v : `#${v}`) : "";
    }
    get username() {
        return this._username;
    }
    set username(v) {
        this._username = String(v);
    }
    get password() {
        return this._password;
    }
    set password(v) {
        this._password = String(v);
    }
    get origin() {
        const noOrigin = ["blob:", "data:", "file:"];
        if (noOrigin.includes(this._protocol)) return "null";
        return `${this._protocol}//${this.host}`;
    }
    get href() {
        const userinfo = this._username
            ? `${this._username}${this._password ? `:${this._password}` : ""}@`
            : "";
        const authority = this._hostname ? `//${userinfo}${this.host}` : "";
        return `${this._protocol}${authority}${this._pathname}${this._search}${this._hash}`;
    }
    set href(v) {
        Object.assign(this, new URL(v));
    }
    get searchParams() {
        this._sp ??= new URLSearchParams(this._search);
        return this._sp;
    }
    toString() {
        return this.href;
    }
    toJSON() {
        return this.href;
    }

    static _blobs = new Map();
    static createObjectURL(obj) {
        const id = `blob:local/${Math.random().toString(36).slice(2)}`;
        URL._blobs.set(id, obj);
        return id;
    }
    static revokeObjectURL(id) {
        URL._blobs.delete(id);
    }
    static canParse(url, base) {
        try {
            new URL(url, base);
            return true;
        } catch {
            return false;
        }
    }
}

function _resolveRelative(url, base) {
    if (_URL_RE.test(url)) return url;
    const b = new URL(base);
    if (url.startsWith("//")) return `${b.protocol}${url}`;
    if (url.startsWith("/")) return `${b.protocol}//${b.host}${url}`;
    const dir = b.pathname.endsWith("/") ? b.pathname : b.pathname.replace(/\/[^/]*$/, "/");
    const joined = dir + url;
    const parts = [];
    for (const seg of joined.split("/")) {
        if (seg === "..") parts.pop();
        else if (seg !== ".") parts.push(seg);
    }
    return `${b.protocol}//${b.host}${parts.join("/")}`;
}

globalThis.URL = URL;
globalThis.URLSearchParams = URLSearchParams;
export { URL, URLSearchParams };
export default { URL, URLSearchParams };
