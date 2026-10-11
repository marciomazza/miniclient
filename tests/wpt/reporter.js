// testharness never clears its harness timer, so a page that finishes early still has it pending
// and miniclient's goto() waits for it. Run the same timer here and clear it on completion.
setup({ explicit_timeout: true });
const harnessTimer = setTimeout(
    timeout,
    document.querySelector('meta[name="timeout"][content="long"]') ? 60000 : 10000,
);
add_completion_callback((tests, status) => {
    clearTimeout(harnessTimer);
    window.__wpt = JSON.stringify({
        status: status.status,
        message: status.message === undefined ? null : status.message,
        tests: tests.map((t) => ({ name: t.name, status: t.status, message: t.message })),
    });
});
