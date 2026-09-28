// esbuild inlines this as a string at bundle time (`.css` loader configured as "text" in
// build-jsdom-bundle.mjs) -- jsdom's computed-style.js reads its default UA stylesheet via
// `fs.readFileSync(path.resolve(__dirname, ...))` at module scope, a real disk read that
// would otherwise need op_fs_read registered during the snapshot build (it isn't) for a
// file whose content never changes at runtime anyway.
import jsdomDefaultStylesheet from "../../node_modules/jsdom/lib/jsdom/browser/default-stylesheet.css";

function statSync(path) {
    const info = Deno.core.ops.op_fs_stat(path);
    return {
        isDirectory: () => info.isDirectory,
        isFile: () => !info.isDirectory,
    };
}

function readFileSync(path, encoding) {
    if (path.endsWith("/default-stylesheet.css")) {
        return encoding ? jsdomDefaultStylesheet : Buffer.from(jsdomDefaultStylesheet);
    }
    const buf = Buffer.from(new Uint8Array(Deno.core.ops.op_fs_read(path)));
    return encoding ? buf.toString(encoding) : buf;
}

// Async resource-mount reads (used by <script type="module"> and async/deferred
// <script src>) go through fs.promises, unlike the sync path (plain <script src>)
// which uses statSync/readFileSync above.
const promises = {
    stat: async (path) => statSync(path),
    readFile: async (path, encoding) => readFileSync(path, encoding),
};

export { statSync, readFileSync, promises };
export default { statSync, readFileSync, promises };
