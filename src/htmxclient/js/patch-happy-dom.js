export default function apply(win) {
    // happy-dom innerHTML setter bugs:
    // (1) doesn't reflect `selected` attr onto .selected IDL property after innerHTML parse
    // (2) doesn't enforce radio mutual exclusion within a name group
    // Walk the internal prototype chain to find the real descriptor (not win.Element.prototype).
    const _probe = win.document.createElement("div");
    let _elProto = Object.getPrototypeOf(_probe);
    while (_elProto && !Object.getOwnPropertyDescriptor(_elProto, "innerHTML"))
        _elProto = Object.getPrototypeOf(_elProto);
    if (_elProto) {
        const _desc = Object.getOwnPropertyDescriptor(_elProto, "innerHTML");
        Object.defineProperty(_elProto, "innerHTML", {
            get: _desc.get,
            set(value) {
                _desc.set.call(this, value);
                this.querySelectorAll("option[selected]").forEach((opt) => {
                    opt.selected = true;
                });
                const groups = {};
                this.querySelectorAll("input[type=radio]").forEach((r) => {
                    (groups[r.name] ??= []).push(r);
                });
                for (const group of Object.values(groups)) {
                    const checked = group.filter((r) => r.checked);
                    if (checked.length > 1)
                        checked.slice(0, -1).forEach((r) => { r.checked = false; });
                }
            },
            configurable: true,
        });
    }
}
