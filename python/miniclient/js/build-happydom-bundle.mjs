// Bundles happy-dom + our patch-happy-dom.js into one classic (non-ESM) script so its
// ~500-file module graph can be baked into the V8 snapshot instead of being re-parsed by
// an async module loader on every Runtime() startup (measured ~100ms of a ~110ms
// open_runtime() call). Node builtin imports (url, buffer, stream, ...) are redirected to
// this project's own polyfills instead of esbuild's Node shims, so the bundle stays
// consistent with what's provided elsewhere at runtime.
import { build } from "esbuild";
import { fileURLToPath } from "node:url";

const _JS = fileURLToPath(new URL(".", import.meta.url));
const _POLYFILLS = _JS + "polyfills/";

const NODE_POLYFILL_FILES = {
  buffer: "node-buffer.js",
  child_process: "node-child-process.js",
  console: "node-console.js",
  crypto: "node-crypto.js",
  fs: "node-fs.js",
  http: "node-http.js",
  https: "node-http.js",
  net: "node-net.js",
  path: "node-path.js",
  perf_hooks: "node-perf-hooks.js",
  stream: "node-stream.js",
  "stream/web": "node-stream-web.js",
  url: "node-url.js",
  util: "node-util.js",
  vm: "node-vm.js",
  zlib: "node-zlib.js",
};

const NPM_POLYFILL_FILES = {
  "whatwg-mimetype": "npm-whatwg-mimetype.js",
  ws: "npm-ws.js",
  "buffer-image-size": "npm-buffer-image-size.js",
};

function polyfillResolverPlugin() {
  return {
    name: "node-polyfills",
    setup(pluginBuild) {
      pluginBuild.onResolve({ filter: /^(node:)?[\w./-]+$/ }, (args) => {
        const bare = args.path.replace(/^node:/, "");
        if (bare in NODE_POLYFILL_FILES) {
          return { path: _POLYFILLS + NODE_POLYFILL_FILES[bare] };
        }
        if (bare in NPM_POLYFILL_FILES) {
          return { path: _POLYFILLS + NPM_POLYFILL_FILES[bare] };
        }
        return null; // fall through to esbuild's default resolution
      });
    },
  };
}

await build({
  entryPoints: [_JS + "happydom-entry.js"],
  bundle: true,
  format: "iife",
  globalName: "__happyDomBundle",
  platform: "browser",
  target: "es2022",
  minify: true,
  outfile: _JS + "_generated/happy-dom-bundle.js",
  plugins: [polyfillResolverPlugin()],
  logLevel: "info",
});
