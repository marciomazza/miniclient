// The `url` bundle alias. happy-dom's `url/URL.js` extends this and its `index.js`
// re-exports URLSearchParams from here, so it must resolve to the exact globals.
// pre_globals.js loads deno_web's 00_url.js and assigns these onto globalThis in the
// cold snapshot pass, before the happy-dom bundle evals.
export const URL = globalThis.URL;
export const URLSearchParams = globalThis.URLSearchParams;

export default { URL, URLSearchParams };
