// Bundles jsdom into one classic (non-ESM) script so its module graph can be baked into the
// V8 snapshot instead of being re-parsed by an async module loader on every Runtime() startup
// (measured ~100ms of a ~110ms AsyncPage() call; jsdom's minified bundle is ~5.2MB).
// Node builtin imports (url, buffer, stream, ...) are redirected to this project's own
// polyfills instead of esbuild's Node shims, so the bundle stays consistent with what's
// provided elsewhere at runtime.
import { build } from "esbuild";
import { fileURLToPath } from "node:url";

const _JS = fileURLToPath(new URL(".", import.meta.url));
const _POLYFILLS = _JS + "polyfills/";

const NODE_POLYFILL_FILES = {
  buffer: "node-buffer.js",
  crypto: "node-crypto.js",
  events: "node-events.js",
  fs: "node-fs.js",
  os: "node-os.js",
  path: "node-path.js",
  stream: "node-stream.js",
  url: "node-url.js",
  util: "node-util.js",
  "util/types": "node-util-types.js",
  vm: "node-vm.js",
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
      pluginBuild.onResolve(
        { filter: /^@asamuzakjp\/dom-selector$|\/living\/css\/CSS-impl\.js$/ },
        () => ({ path: _POLYFILLS + "npm-css-engine-stub.js" }),
      );
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
  // Only ever referenced behind a runtime feature check (`runtimeFeatures.has('sqlite')`)
  // that this bundle never triggers -- undici/lib/util/runtime-features.js's lazy factory
  // map still needs it to resolve at bundle time, so external it rather than polyfill.
  external: ["node:sqlite"],
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

// Mini's own snapshot scripts, bundled so they can `import` each other instead of sharing
// state through globalThis. Kept readable (no minify): stack frames from these land here.
await build({
  entryPoints: [_JS + "snapshot-entry.js"],
  bundle: true,
  format: "iife",
  platform: "browser",
  target: "es2022",
  keepNames: true,
  outfile: _JS + "_generated/snapshot-bundle.js",
  logLevel: "info",
});
