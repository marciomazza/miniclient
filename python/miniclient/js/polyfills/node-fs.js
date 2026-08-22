function statSync(path) {
    const info = __host_fs_stat(path);
    return {
        isDirectory: () => info.isDirectory,
        isFile: () => !info.isDirectory,
    };
}

function readFileSync(path, encoding) {
    const buf = Buffer.from(new Uint8Array(__host_fs_read(path)));
    return encoding ? buf.toString(encoding) : buf;
}

// happy-dom's async virtual-server fetch (used by <script type="module"> and
// async/deferred <script src>) reads mounted files via fs.promises, unlike the
// sync fetch path (plain <script src>) which uses statSync/readFileSync above.
const promises = {
    stat: async (path) => statSync(path),
    readFile: async (path, encoding) => readFileSync(path, encoding),
};

export { statSync, readFileSync, promises };
export default { statSync, readFileSync, promises };
