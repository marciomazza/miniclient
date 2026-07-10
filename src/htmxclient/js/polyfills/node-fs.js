function statSync(path) {
    const opId = globalThis.__FS_STAT_OP_ID__;
    if (opId === undefined) throw new Error("fs.statSync is not available in this runtime");
    const info = __host_op_sync__(opId, path);
    return {
        isDirectory: () => info.isDirectory,
        isFile: () => !info.isDirectory,
    };
}

function readFileSync(path, encoding) {
    const opId = globalThis.__FS_READ_OP_ID__;
    if (opId === undefined) throw new Error("fs.readFileSync is not available in this runtime");
    const bytes = __host_op_sync__(opId, path);
    const buf = Buffer.from(new Uint8Array(bytes));
    return encoding ? buf.toString(encoding) : buf;
}

export { statSync, readFileSync };
export default { statSync, readFileSync };
