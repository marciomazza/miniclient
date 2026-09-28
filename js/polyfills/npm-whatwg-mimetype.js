class MIMEType {
    constructor(str) {
        const [type, ...rest] = (str ?? "").split("/");
        this.type = type ?? "";
        const [subtype] = (rest.join("/") ?? "").split(";");
        this.subtype = subtype?.trim() ?? "";
        this.parameters = new Map();
    }
    get essence() {
        return `${this.type}/${this.subtype}`;
    }
    toString() {
        return this.essence;
    }
    isHTML() {
        return this.type === "text" && this.subtype === "html";
    }
    isXML() {
        return this.subtype === "xml" || this.subtype.endsWith("+xml");
    }
    // Real whatwg-mimetype validates types per the spec grammar; this stub only guards the
    // one failure mode callers actually branch on -- an absent/empty header, e.g. a script
    // response with no Content-Type at all.
    static parse(str) {
        if (!str) return null;
        return new MIMEType(str);
    }
}
export { MIMEType };
export default MIMEType;
