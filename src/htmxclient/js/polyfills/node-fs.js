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

export { statSync, readFileSync };
export default { statSync, readFileSync };
