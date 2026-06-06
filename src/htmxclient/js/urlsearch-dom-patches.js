// Wrap win.URL to fix searchParams mutations not propagating back to url.search/href.
// happy-dom's URLSearchParams is a live object but its mutation methods don't call back
// into the URL to update its serialized search string.
// Patch URLSearchParams constructor to accept FormData and URLSearchParams as init.
// happy-dom's implementation ignores those two iterable forms.
export default function apply(win) {
    const _WinURL = win.URL;
    class _PatchedURL extends _WinURL {
        get searchParams() {
            const sp = super.searchParams;
            if (!sp._urlRef) {
                sp._urlRef = this;
                for (const m of ["append", "set", "delete", "sort"]) {
                    const orig = sp[m];
                    sp[m] = function (...args) {
                        const result = orig.apply(sp, args);
                        const s = sp.toString();
                        sp._urlRef.search = s ? "?" + s : "";
                        return result;
                    };
                }
            }
            return sp;
        }
    }
    for (const k of ["canParse", "createObjectURL", "revokeObjectURL"]) {
        if (_WinURL[k]) _PatchedURL[k] = _WinURL[k].bind(_WinURL);
    }
    globalThis.URL = _PatchedURL;

    const _WinUSP = win.URLSearchParams;
    class _PatchedUSP extends _WinUSP {
        constructor(init) {
            // happy-dom's URLSearchParams constructor ignores URLSearchParams and FormData
            // as init values.  Detect those two types and copy entries manually.
            // Plain strings, plain objects, and arrays are handled by the parent constructor.
            if (
                init instanceof _WinUSP ||
                (typeof FormData !== "undefined" && init instanceof FormData)
            ) {
                super();
                for (const [k, v] of init.entries()) this.append(String(k), String(v));
            } else {
                super(init);
            }
        }
    }
    globalThis.URLSearchParams = _PatchedUSP;
}
