// @ts-check
/** @type {WeakMap<Element, number>} */
const ids = new WeakMap();
/** @type {Map<number, WeakRef<Element>>} */
const refs = new Map();
let nextId = 1;

/** @param {Element | null | undefined} el */
export function ref(el) {
    if (el == null) return null;
    let id = ids.get(el);
    if (id === undefined) {
        id = nextId++;
        ids.set(el, id);
        refs.set(id, new WeakRef(el));
    }
    return id;
}

/** @param {number} id */
export function deref(id) {
    const ref = refs.get(id);
    const el = ref && ref.deref();
    if (!el || !el.isConnected) {
        refs.delete(id);
        return null;
    }
    return el;
}
