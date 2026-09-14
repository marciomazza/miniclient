// jsdom's default ResourceLoader (real network fetch for <script src>/<link>/...) and native
// WebSocket both go through undici -- neither is used here: mini loads scripts itself via
// runtime.eval() and provides its own fetch() in bootstrap.js. Dispatcher/DecoratorHandler are
// only ever subclassed (safe with an empty base class); getGlobalDispatcher()'s return value is
// stored but never called unless something actually reaches the real ResourceLoader.
export class Dispatcher {}
export class DecoratorHandler {}
export class WebSocket {}
export function getGlobalDispatcher() {
    return new Dispatcher();
}
