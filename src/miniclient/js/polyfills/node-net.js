function isIP(ip) {
    const v4 = /^(25[0-5]|2[0-4]\d|[01]?\d\d?)(\.(25[0-5]|2[0-4]\d|[01]?\d\d?)){3}$/;
    const v6 = /^([0-9A-Fa-f]{1,4}:){7}[0-9A-Fa-f]{1,4}$/;
    if (v4.test(ip)) return 4;
    if (v6.test(ip)) return 6;
    return 0;
}
export { isIP };
export default { isIP };
