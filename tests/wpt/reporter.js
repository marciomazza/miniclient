add_completion_callback((tests, status) => {
    window.__wpt = JSON.stringify({
        status: status.status,
        message: status.message === undefined ? null : status.message,
        tests: tests.map((t) => ({ name: t.name, status: t.status, message: t.message })),
    });
});
