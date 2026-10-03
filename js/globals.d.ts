// What the host and the V8 snapshot put on `globalThis`, for the classic scripts in this
// directory that share the realm instead of importing each other.

type JSDOMWindow = import("jsdom").DOMWindow;
type NavigationMode = "push" | "replace" | { traverseTo: number };
type HistoryEntries = { entries: { url: string; state: unknown }[]; current: number };

/** The Python<->JS bridge surface. Members are added by element_registry, submit, pre_globals,
 *  fetch and bootstrap, in that cold-pass order. */
interface Mini {
  ref(el: Element | null): number | null;
  deref(id: number): Element | null;
  awaitSettle(handle: number, doAction: (el: Element) => unknown): Promise<void>;
  formSubmitFetch(form: HTMLFormElement, submitter?: HTMLElement | null): void;
  submit(handle: number): Promise<void>;
  contentTypeFor(path: string): string;
  serializeMultipart(formData: FormData): { boundary: string; body: string };
  installXPathShortCircuit(win: JSDOMWindow): void;
  installInnerHtmlRadioFix(win: JSDOMWindow): void;
  installInnerText(win: JSDOMWindow): void;
  // fetch.js: the wire jsdom's dispatcher sits on, and the page's fetch for a window.
  baseDispatcher: any;
  makeFetch(win: JSDOMWindow): typeof fetch;
  // bootstrap.js
  documentWrite(html: string): Promise<void>;
  navigateFetch(window: JSDOMWindow, url: string, replacement?: boolean): void;
  historyLength(): number;
  navigateHistory(delta: number): void;
  navigateBlank(
    url: string,
    from?: { base: string; cookies: any; history: HistoryEntries },
  ): Promise<void>;
  goingFrom(): { base: string; cookies: any; history: HistoryEntries };
}

declare var __mini: Mini;

// Injected by boot_jsruntime (src/runtime.rs) before bootstrap.js runs.
declare var __BASE_URL__: string;
/** URL prefix -> directory. */
declare var __RESOURCE_MOUNTS__: Record<string, string>;
declare var __HOST_STATE__: { cookies: unknown; history: HistoryEntries | null } | undefined;
declare var __NAVIGATION_REQUEST__:
  | { method?: string; body?: unknown; headers?: unknown }
  | undefined;
declare var __NAVIGATION_BOOT__: boolean;

// Set by jsdom-entry.js (the esbuild bundle baked into the snapshot).
declare var __JSDOMCookieJar: typeof import("jsdom").CookieJar;
declare var __serializeURL: (url: any) => string;
declare var __clearDomBindings: () => void;
declare var __jsdomImplForWrapper: (wrapper: any) => any;
declare var __jsdomWrapperForImpl: (impl: any) => any;
declare var __patchJsdom: (win: JSDOMWindow) => void;
declare var __innerText: (el: HTMLElement, options?: any) => string;
declare var __createDataTransfer: (...args: any[]) => any;
declare var __createRealFileList: (win: JSDOMWindow, files: File[]) => FileList;

// pre_globals.js: deno_fetch's classes, kept apart from the page-visible jsdom ones.
declare var __denoFetch: {
  Headers: typeof Headers;
  Request: typeof Request;
  Response: typeof Response;
  FormData: typeof FormData;
  Blob: typeof Blob;
  File: typeof File;
  InnerBody: any;
  newInnerResponse: any;
  fromInnerResponse: any;
};

declare namespace Deno {
  const core: {
    loadExtScript(specifier: string): any;
    print(message: string, isErr?: boolean): void;
    ops: {
      op_request_navigation(
        url: string,
        mode: NavigationMode,
        method?: string,
        body?: string,
        headers?: Record<string, string>,
      ): void;
      op_fetch_hop(...args: any[]): any;
      op_fetch_abort(id: string): void;
      op_fs_read(path: string): Uint8Array;
      op_fs_stat(path: string): any;
      op_crypto_random_uuid(): `${string}-${string}-${string}-${string}-${string}`;
      op_crypto_random_bytes(n: number): Uint8Array;
      op_now(): number;
    };
  };
}
