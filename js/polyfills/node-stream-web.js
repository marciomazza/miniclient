// The `stream/web` bundle alias. undici's web/fetch/body.js does
// `require('node:stream/web')` and pulls `ReadableStreamTee` from it (downstream
// `instanceof ReadableStream` checks need the exact global). The real classes come
// from deno_web's 06_streams.js, assigned onto globalThis by pre_globals.js in the
// cold snapshot pass.
export const ReadableStream = globalThis.ReadableStream;
export const WritableStream = globalThis.WritableStream;
export const TransformStream = globalThis.TransformStream;

export default { ReadableStream, WritableStream, TransformStream };
