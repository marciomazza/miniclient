// Only jsdom's synchronous-XMLHttpRequest code path needs this (Atomics.wait on a
// MessageChannel to block for a worker's response). Nothing here constructs a real
// XMLHttpRequest, sync or otherwise -- mini's fetch() (bootstrap.js) is the only network path
// exposed to page code, so these throw instead of silently pretending to support sync XHR.
class Worker {
    constructor() {
        throw new Error("node:worker_threads.Worker is not supported in this runtime");
    }
}
class MessageChannel {
    constructor() {
        throw new Error("node:worker_threads.MessageChannel is not supported in this runtime");
    }
}
function receiveMessageOnPort() {
    throw new Error("node:worker_threads.receiveMessageOnPort is not supported in this runtime");
}
// No-op: only marks an object so a later structuredClone()/postMessage would reject it --
// undici's Request/Response/FormData call this on construction, but nothing here ever
// clones one across a message channel, so there's nothing to actually mark.
function markAsUncloneable() {}
export { Worker, MessageChannel, receiveMessageOnPort, markAsUncloneable };
export default { Worker, MessageChannel, receiveMessageOnPort, markAsUncloneable };
