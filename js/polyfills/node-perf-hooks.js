// `node:perf_hooks` re-export for the bundle. undici's web/fetch/util.js does
// `require('node:perf_hooks')` and reads `performance` from it; the real classes come
// from deno_web's `performance` global, set on globalThis by pre_globals.js.
const { PerformanceObserver, PerformanceEntry } = globalThis;

export { PerformanceEntry, PerformanceObserver };
export default { PerformanceEntry, PerformanceObserver };
