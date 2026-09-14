// happy-dom's module graph required "buffer" eagerly, which is what actually replaced
// pre_globals.js's placeholder `globalThis.Buffer` with the real polyfill; jsdom's minimal
// construction path (empty HTML, no scripts) never reaches a require for it. Import it here,
// before jsdom, purely for that side effect -- ESM evaluates imports in declaration order, so
// this runs (and installs the real Buffer) before any of jsdom's own module graph does.
import "buffer";
import { JSDOM } from "jsdom";

// The esbuild bundle wraps this entry point in an anonymous IIFE, so the import above stays
// scoped to it -- this is the one explicit hand-off point to the shared realm. Everywhere
// else (bootstrap.js, snapshot_warmup.js, ...) just references the bare `JSDOM` global.
globalThis.JSDOM = JSDOM;
