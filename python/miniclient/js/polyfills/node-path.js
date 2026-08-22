const sep = "/";

function normalize(path) {
    const isAbsolute = path.startsWith("/");
    const segments = [];
    for (const part of path.split("/")) {
        if (part === "" || part === ".") continue;
        if (part === "..") {
            if (segments.length && segments[segments.length - 1] !== "..") segments.pop();
            else if (!isAbsolute) segments.push("..");
        } else segments.push(part);
    }
    return (isAbsolute ? "/" : "") + segments.join("/");
}

function join(...parts) {
    return normalize(parts.join("/")) || ".";
}

function resolve(...parts) {
    // ponytail: no cwd fallback — every caller in this codebase passes an absolute path
    let result = "";
    for (let i = parts.length - 1; i >= 0; i--) {
        result = result ? `${parts[i]}/${result}` : parts[i];
        if (parts[i].startsWith("/")) break;
    }
    return normalize(result) || "/";
}

function dirname(path) {
    const parts = path.split("/");
    parts.pop();
    return parts.join("/") || (path.startsWith("/") ? "/" : ".");
}

function basename(path, ext) {
    let b = path.split("/").pop();
    if (ext && b.endsWith(ext)) b = b.slice(0, -ext.length);
    return b;
}

function extname(path) {
    const b = basename(path);
    const i = b.lastIndexOf(".");
    return i > 0 ? b.slice(i) : "";
}

export { sep, join, resolve, dirname, basename, extname };
export default { sep, join, resolve, dirname, basename, extname };
