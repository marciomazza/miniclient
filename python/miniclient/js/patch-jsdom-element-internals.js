// jsdom exposes attachInternals(), but does not yet implement the form-associated part of
// ElementInternals. Keep the state on the implementation object so it is not page-visible.
import { implForWrapper, wrapperForImpl } from "jsdom/lib/generated/idl/utils.js";

export function patchElementInternals(win) {
    const formValue = Symbol("mini.formValue");
    const formOwner = (element) => {
        const id = element.getAttribute("form");
        if (id !== null) {
            if (!id) return null;
            const owner = element.ownerDocument.getElementById(id);
            return owner instanceof win.HTMLFormElement ? owner : null;
        }
        return element.closest("form");
    };
    const customControls = (form) => {
        const controls = [...form.querySelectorAll("*")];
        if (form.id) {
            controls.push(
                ...form.ownerDocument.querySelectorAll(`[form="${CSS.escape(form.id)}"]`),
            );
        }
        return controls.filter(
            (element) =>
                element.constructor.formAssociated === true && formOwner(element) === form,
        );
    };
    const elementFromImpl = (impl) => wrapperForImpl(impl);

    Object.defineProperty(win.ElementInternals.prototype, "setFormValue", {
        value(value) {
            const internals = implForWrapper(this);
            internals._targetElement[formValue] = value;
        },
        configurable: true,
    });
    Object.defineProperty(win.ElementInternals.prototype, "form", {
        get() {
            const internals = implForWrapper(this);
            return formOwner(elementFromImpl(internals._targetElement));
        },
        configurable: true,
    });

    const nativeElements = Object.getOwnPropertyDescriptor(
        win.HTMLFormElement.prototype,
        "elements",
    ).get;
    Object.defineProperty(win.HTMLFormElement.prototype, "elements", {
        get() {
            const elements = [...nativeElements.call(this), ...customControls(this)].sort(
                (a, b) => {
                    const position = a.compareDocumentPosition(b);
                    return position & win.Node.DOCUMENT_POSITION_FOLLOWING
                        ? -1
                        : position & win.Node.DOCUMENT_POSITION_PRECEDING
                          ? 1
                          : 0;
                },
            );
            if (elements.length === 0) return nativeElements.call(this);
            return new Proxy(elements, {
                get(target, property) {
                    if (property === "length") return target.length;
                    if (property === Symbol.iterator)
                        return target[Symbol.iterator].bind(target);
                    if (property === "namedItem") {
                        return (name) =>
                            target.find(
                                (element) =>
                                    element.id === name ||
                                    element.getAttribute("name") === name,
                            ) ?? null;
                    }
                    return Reflect.get(target, property, target);
                },
            });
        },
        configurable: true,
    });

    const NativeFormData = win.FormData;
    win.FormData = class FormData extends NativeFormData {
        constructor(form, submitter) {
            super(form, submitter);
            if (!(form instanceof win.HTMLFormElement)) return;
            for (const element of customControls(form)) {
                const name = element.getAttribute("name");
                const value = implForWrapper(element)[formValue];
                if (name && value != null) this.append(name, value);
            }
        }
    };
    globalThis.FormData = win.FormData;
}
