const constants = { Z_SYNC_FLUSH: 2, Z_FINISH: 4 };
const noop = () => ({ pipe: (s) => s, on: () => {} });
export default {
    constants,
    createGunzip: noop,
    createInflate: noop,
    createInflateRaw: noop,
    createBrotliDecompress: noop,
};
