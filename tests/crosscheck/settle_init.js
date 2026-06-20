window.__htmxSettled = false;
window.__htmxWillRequest = false;
document.addEventListener("htmx:finally:request", () => {
    window.__htmxSettled = true;
});
document.addEventListener("htmx:before:request", () => {
    window.__htmxWillRequest = true;
});
