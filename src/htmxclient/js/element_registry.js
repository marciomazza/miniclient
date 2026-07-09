(function () {
    const ids = new WeakMap();
    const refs = new Map();
    let nextId = 1;

    globalThis.__zzz_ref = function (el) {
        if (el == null) return null;
        let id = ids.get(el);
        if (id === undefined) {
            id = nextId++;
            ids.set(el, id);
            refs.set(id, new WeakRef(el));
        }
        return id;
    };

    globalThis.__zzz_deref = function (id) {
        const ref = refs.get(id);
        const el = ref && ref.deref();
        if (!el || !el.isConnected) {
            refs.delete(id);
            return null;
        }
        return el;
    };
})();
