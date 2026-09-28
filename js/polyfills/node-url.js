// The `url` bundle alias. undici's dispatcher/socks5-proxy-agent.js does
// `require('node:url')` for its `URL` import. The real classes come from deno_web's
// 00_url.js, assigned onto globalThis by pre_globals.js in the cold snapshot pass.
export const URL = globalThis.URL;
export const URLSearchParams = globalThis.URLSearchParams;

export default { URL, URLSearchParams };
