// undici's vendored fetch classes only ever call this as a bare invariant check
// (`assert(value)`), never `.ok`/`.strictEqual`/etc.
//
// Written as CommonJS on purpose: `require('node:assert')` must hand back the function
// itself, not a namespace object with a `.default` property -- which is what esbuild's
// ESM interop would produce from an `export default`.
function assert(value, message) {
    if (!value) throw new Error(message || "assertion failed");
}
module.exports = assert;
