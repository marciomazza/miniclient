() => {
    // Use prototype getters to avoid shadowing by named form controls (e.g. name="tagName").
    const _getDescriptor = Object.getOwnPropertyDescriptor;

    const _tagName = _getDescriptor(Element.prototype, 'tagName').get;
    const _nodeType = _getDescriptor(Node.prototype, 'nodeType').get;
    const _childNodes = _getDescriptor(Node.prototype, 'childNodes').get;
    const _attributes = _getDescriptor(Element.prototype, 'attributes').get;
    const _getAttribute = Element.prototype.getAttribute;
    const _value = _getDescriptor(HTMLInputElement.prototype, 'value').get;
    const _inputType = _getDescriptor(HTMLInputElement.prototype, 'type').get;
    const _checked = _getDescriptor(HTMLInputElement.prototype, 'checked').get;
    const _textareaValue = _getDescriptor(HTMLTextAreaElement.prototype, 'value').get;
    const _selectValue = _getDescriptor(HTMLSelectElement.prototype, 'value').get;
    const _optionSelected = _getDescriptor(window.HTMLOptionElement.prototype, 'selected').get;

    function serializeNode(node) {
        const nodeType = _nodeType.call(node);
        if (nodeType === Node.TEXT_NODE) {
            // node.data is the reliable property for text nodes
            //   in both happy-dom and real browsers;
            // Node.prototype.textContent getter returns '' for Text nodes in happy-dom.
            const data = node.data.trim();
            return data ? {type: "text", data} : null;
        }
        if (nodeType !== Node.ELEMENT_NODE) return null;
        const tag = _tagName.call(node).toLowerCase();
        if (tag === "script") return null;
        const attrList = _attributes.call(node);
        const attrNames = [...attrList].map(a => a.name).sort();
        const attrs = {};
        for (const name of attrNames) attrs[name] = _getAttribute.call(node, name);
        const result = {tag, attrs};
        if (tag === "input") {
            result.value = _value.call(node);
            const t = _inputType.call(node);
            if (t === "checkbox" || t === "radio") result.checked = _checked.call(node);
        } else if (tag === "textarea") {
            result.value = _textareaValue.call(node);
        } else if (tag === "select") {
            result.value = _selectValue.call(node);
        } else if (tag === "option") {
            result.selected = _optionSelected.call(node);
        }
        result.children = [..._childNodes.call(node)].map(serializeNode).filter(Boolean);
        return result;
    }
    return serializeNode(document.body);
};
