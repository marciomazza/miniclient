// Pure JS FormData — does not rely on win.FormData (which has multiple happy-dom bugs).
// Implements: https://xhr.spec.whatwg.org/#interface-formdata
(function () {
    const _SUBMIT_TYPES = new Set(["submit", "image", "reset", "button"]);

    function _isDisabled(el) {
        if (el.disabled) return true;
        // fieldset-disabled: element is disabled if any ancestor <fieldset disabled> contains
        // it outside that fieldset's first <legend> child.
        let node = el.parentElement;
        while (node) {
            if (node.tagName === "FIELDSET" && node.disabled) {
                const legend = node.querySelector(":scope > legend");
                if (!legend || !legend.contains(el)) return true;
            }
            node = node.parentElement;
        }
        return false;
    }

    function _collectElement(el, data) {
        const name = el.name ?? el.getAttribute?.("name");
        if (!name || _isDisabled(el)) return;
        const tag = el.tagName;
        const type = (el.type || "text").toLowerCase();
        // buttons are only submitted when they are the active submitter (handled by constructor)
        if (tag === "BUTTON" || (tag === "INPUT" && _SUBMIT_TYPES.has(type))) return;

        if (tag === "INPUT") {
            if (type === "checkbox" || type === "radio") {
                if (el.checked) data.push([name, el.value !== "" ? el.value : "on"]);
            } else if (!_SUBMIT_TYPES.has(type) && type !== "file") {
                data.push([name, el.value ?? ""]);
            }
        } else if (tag === "TEXTAREA") {
            data.push([name, el.value ?? ""]);
        } else if (tag === "SELECT") {
            if (el.multiple) {
                for (const opt of el.options) {
                    if (opt.selected) data.push([name, opt.value]);
                }
            } else if (el.selectedIndex >= 0) {
                data.push([name, el.options[el.selectedIndex].value]);
            }
        } else if (
            typeof el.__internalsFormValue !== "undefined" &&
            el.__internalsFormValue != null
        ) {
            data.push([name, el.__internalsFormValue]);
        }
    }

    class FormData {
        #data = [];

        constructor(form, submitter) {
            if (form == null) return;
            const seen = new Set();
            for (const el of form.elements) {
                if (el === submitter) continue;
                seen.add(el);
                _collectElement(el, this.#data);
            }
            // form-associated custom elements inside the form (not in form.elements in happy-dom)
            for (const el of form.querySelectorAll("*")) {
                if (seen.has(el) || typeof el.__internalsFormValue === "undefined") continue;
                seen.add(el);
                _collectElement(el, this.#data);
            }
            // Elements outside the form that reference it via form="id"
            const formId = form.id;
            if (formId) {
                const root = form.getRootNode?.() ?? document;
                for (const el of root.querySelectorAll(`[form="${CSS.escape(formId)}"]`)) {
                    if (el === submitter || seen.has(el)) continue;
                    seen.add(el);
                    _collectElement(el, this.#data);
                }
            }
            if (submitter) {
                const name = submitter.name;
                if (name && !_isDisabled(submitter)) this.#data.push([name, submitter.value ?? ""]);
            }
        }

        append(name, value) {
            this.#data.push([name, value]);
        }

        set(name, value) {
            this.#data = this.#data.filter(([k]) => k !== name);
            this.#data.push([name, value]);
        }

        delete(name) {
            this.#data = this.#data.filter(([k]) => k !== name);
        }

        get(name) {
            return this.#data.find(([k]) => k === name)?.[1] ?? null;
        }

        getAll(name) {
            return this.#data.filter(([k]) => k === name).map(([, v]) => v);
        }

        has(name) {
            return this.#data.some(([k]) => k === name);
        }

        keys() {
            return this.#data.map(([k]) => k)[Symbol.iterator]();
        }

        values() {
            return this.#data.map(([, v]) => v)[Symbol.iterator]();
        }

        entries() {
            return this.#data.map(([k, v]) => [k, v])[Symbol.iterator]();
        }

        forEach(cb) {
            for (const [k, v] of this.#data) cb(v, k, this);
        }

        [Symbol.iterator]() {
            return this.entries();
        }
    }

    globalThis.FormData = FormData;
})();
