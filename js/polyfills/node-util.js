import types from "./node-util-types.js";

export const TextEncoder = globalThis.TextEncoder;
export const TextDecoder = globalThis.TextDecoder;
// Real Node well-known symbol -- undici's fetch classes use it as a computed class member
// name (`[inspect.custom](...)`) for their console-formatting method, never called here.
export const inspect = { custom: Symbol.for("nodejs.util.inspect.custom") };
export { types };
export default { TextEncoder, TextDecoder, inspect, types };
