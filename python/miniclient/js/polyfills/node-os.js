// Only jsdom's NavigatorConcurrentHardware-impl.js needs this, for
// `navigator.hardwareConcurrency = os.cpus().length`. No real CPU count is available inside
// the isolate; report a plausible constant like most headless setups do.
function cpus() {
    return new Array(4).fill({});
}
export { cpus };
export default { cpus };
