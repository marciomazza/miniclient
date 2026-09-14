// Bundles jsdom into one classic (non-ESM) script so its module graph can be baked into the
// V8 snapshot instead of being re-parsed by an async module loader on every Runtime() startup
// (measured ~100ms of a ~110ms open_runtime() call for happy-dom's ~500-file, ~856KB minified
// graph; jsdom's minified bundle is ~5.2MB, over 6x larger). Node builtin imports (url, buffer,
// stream, ...) are redirected to this project's own polyfills instead of esbuild's Node shims,
// so the bundle stays consistent with what's provided elsewhere at runtime.
import { build } from "esbuild";
import { fileURLToPath } from "node:url";

const _JS = fileURLToPath(new URL(".", import.meta.url));
const _POLYFILLS = _JS + "polyfills/";

const NODE_POLYFILL_FILES = {
  buffer: "node-buffer.js",
  crypto: "node-crypto.js",
  events: "node-events.js",
  fs: "node-fs.js",
  http: "node-http.js",
  https: "node-http.js",
  net: "node-net.js",
  os: "node-os.js",
  path: "node-path.js",
  perf_hooks: "node-perf-hooks.js",
  stream: "node-stream.js",
  "stream/web": "node-stream-web.js",
  url: "node-url.js",
  util: "node-util.js",
  vm: "node-vm.js",
  worker_threads: "node-worker-threads.js",
  zlib: "node-zlib.js",
};

const NPM_POLYFILL_FILES = {
  "whatwg-mimetype": "npm-whatwg-mimetype.js",
  undici: "npm-undici.js",
};

function polyfillResolverPlugin() {
  return {
    name: "node-polyfills",
    setup(pluginBuild) {
      // Real XMLHttpRequest-impl.js does require.resolve("./xhr-sync-worker.js") at module
      // scope for its sync-XHR worker thread; esbuild can't statically resolve that dynamic
      // require and neither worker_threads nor a real fs exist here. Nothing in mini/htmx
      // constructs a real XMLHttpRequest (both are fetch-based), so swap the whole module.
      pluginBuild.onResolve({ filter: /living\/xhr\/XMLHttpRequest-impl\.js$/ }, () => ({
        path: _POLYFILLS + "npm-jsdom-xhr-stub.js",
      }));
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

// JSDOM_BUNDLE_SOURCEMAP=1: unminified + inline sourcemap, so V8 profiles and DevTools map
// frames back to jsdom's sources. Off by default -- it bloats the bundle and changes the
// snapshot cache key. Toggling it does not re-trigger build.rs's staleness check; delete
// _generated/jsdom-bundle.js to force a rebuild.
const debug = process.env.JSDOM_BUNDLE_SOURCEMAP === "1";

await build({
  entryPoints: [_JS + "jsdom-entry.js"],
  bundle: true,
  format: "iife",
  platform: "browser",
  target: "es2022",
  minify: !debug,
  // Keep original jsdom identifiers in stack traces; costs ~1.7% bundle size (measured).
  keepNames: true,
  sourcemap: debug ? "inline" : false,
  outfile: _JS + "_generated/jsdom-bundle.js",
  // esbuild's default loader for .css is "css" (produces a separate stylesheet asset, no JS
  // export) -- node-fs.js imports jsdom's default UA stylesheet expecting a plain JS string.
  loader: { ".css": "text" },
  plugins: [polyfillResolverPlugin()],
  logLevel: "info",
  // esbuild's `platform: "browser"` deliberately never defines `__dirname` (no
  // filesystem assumption) -- jsdom's computed-style.js is the one file that reads one
  // at module scope (`fs.readFileSync(path.resolve(__dirname, ...))`, to load its default
  // UA stylesheet). The value only has to be a stable, recognisable string:
  // node-fs.js's readFileSync special-cases that exact suffix and returns the
  // stylesheet text esbuild already inlined at bundle time (see its own `.css` import),
  // rather than going through the real op_fs_read op (not registered during the
  // snapshot build, and pointless for a file whose content never changes at runtime).
  define: { __dirname: '"/jsdom-lib"', global: "globalThis" },
});
