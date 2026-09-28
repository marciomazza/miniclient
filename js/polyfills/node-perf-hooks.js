// happy-dom's BrowserWindow imports these from node:perf_hooks. The real classes are
// deno_web's, installed on globalThis by pre_globals.js before this bundle evaluates.
const { PerformanceObserver, PerformanceEntry } = globalThis;

export { PerformanceEntry, PerformanceObserver };
export default { PerformanceEntry, PerformanceObserver };
