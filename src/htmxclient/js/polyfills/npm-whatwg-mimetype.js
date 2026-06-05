class MIMEType {
    constructor(str) {
        const [type, ...rest] = (str ?? '').split('/');
        this.type = type ?? '';
        const [subtype] = (rest.join('/') ?? '').split(';');
        this.subtype = subtype?.trim() ?? '';
        this.parameters = new Map();
    }
    get essence() { return `${this.type}/${this.subtype}`; }
    toString() { return this.essence; }
    isHTML() { return this.type === 'text' && this.subtype === 'html'; }
    isXML() { return this.subtype === 'xml' || this.subtype.endsWith('+xml'); }
}
export default MIMEType;
