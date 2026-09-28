// Forwards to the runtime's real crypto (pre_globals.js, op-backed CSPRNG).
const webcrypto = globalThis.crypto;
export { webcrypto };
export default globalThis.crypto;
