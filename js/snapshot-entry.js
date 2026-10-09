// Entry point of the bundle that runs first in the snapshot build. Import order is evaluation
// order: pre_globals installs the polyfills the later files, and jsdom's bundle after them,
// read at load time.
import "./pre_globals.js";
import "./element_registry.js";
import "./submit.js";
import "./fetch.js";
