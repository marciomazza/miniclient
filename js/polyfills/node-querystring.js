// Only `stringify` is imported (undici's `core/util.js`, for a client-request path this
// runtime never takes -- Request/Response construction never calls it). Approximate with
// URLSearchParams instead of matching Node's querystring encoding exactly.
export const stringify = (obj) => new URLSearchParams(obj).toString();
export default { stringify };
