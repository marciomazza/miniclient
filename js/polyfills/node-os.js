// Only jsdom's NavigatorConcurrentHardware-impl.js needs this, for
// `navigator.hardwareConcurrency = os.cpus().length`; no real CPU count is available.
function cpus() {
    return new Array(4).fill({});
}
export { cpus };
export default { cpus };
