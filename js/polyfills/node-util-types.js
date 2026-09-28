// Node's real util.types.* are native bindings that inspect V8 internals; these are pure-JS
// approximations, good enough for undici's webidl argument-shape checks (used by Request,
// Response, Headers, FormData construction). isProxy has no equivalent at all in plain JS
// (proxies are designed to be undetectable) -- always false, safe since nothing here ever
// passes a real Proxy as a body/init value.
export const isUint8Array = (v) => v instanceof Uint8Array;
export const isArrayBuffer = (v) => v instanceof ArrayBuffer;
export const isSharedArrayBuffer = (v) => v instanceof SharedArrayBuffer;
export const isAnyArrayBuffer = (v) => isArrayBuffer(v) || isSharedArrayBuffer(v);
export const isDataView = (v) => v instanceof DataView;
export const isArrayBufferView = (v) => ArrayBuffer.isView(v);
export const isTypedArray = (v) => ArrayBuffer.isView(v) && !isDataView(v);
export const isProxy = () => false;

const types = {
    isUint8Array,
    isArrayBuffer,
    isSharedArrayBuffer,
    isAnyArrayBuffer,
    isDataView,
    isArrayBufferView,
    isTypedArray,
    isProxy,
};
export default types;
