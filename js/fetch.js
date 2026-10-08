// The page's `fetch`: deno_fetch's Request/Response/Headers over the window's JSDOMDispatcher.
// jsdom's dispatcher is the HTTP-fetch layer (cookies per hop, redirect rewrites, CORS and
// preflight checks); `baseDispatcher` below is the wire, one hop per `op_fetch_hop`. This file
// adds what sits above them: mode and credentials rules, Origin/Referer, response filtering.
(() => {
    const D = globalThis.__denoFetch;
    const mini = (globalThis.__mini ??= {});

    // Page-visible classes are the ones fetch() returns, so `instanceof Response` holds.
    globalThis.Headers = D.Headers;
    // deno resolves a relative URL against a base fixed at boot; the page's moves with pushState.
    globalThis.Request = class Request extends D.Request {
        constructor(input, init) {
            super(typeof input === "string" ? new URL(input, location.href).href : input, init);
        }
    };
    globalThis.Response = D.Response;

    const header = (headers, name) => {
        const key = Object.keys(headers).find((k) => k.toLowerCase() === name);
        return key === undefined ? undefined : headers[key];
    };
    const omit = (headers, name) =>
        Object.fromEntries(Object.entries(headers).filter(([k]) => k.toLowerCase() !== name));

    // ---- wire: one hop, no redirects, no cookie jar of its own -------------------------

    function makeController(onAbort) {
        const controller = {
            aborted: false,
            reason: undefined,
            paused: false,
            rawHeaders: [],
            rawTrailers: [],
            pause() {},
            resume() {},
            abort(reason) {
                if (controller.aborted) return;
                controller.aborted = true;
                controller.reason = reason;
                onAbort();
            },
        };
        return controller;
    }

    function emit(handler, controller, status, statusText, headers, body) {
        handler.onResponseStart?.(controller, status, headers, statusText);
        if (body?.length) handler.onResponseData?.(controller, body);
        handler.onResponseEnd?.(controller, {});
    }

    function fromMount(url) {
        const mounts = globalThis.__RESOURCE_MOUNTS__;
        const prefix = Object.keys(mounts).find((p) => url.startsWith(p));
        if (prefix === undefined) return null;
        const path = `${mounts[prefix]}/${url.slice(prefix.length)}`;
        try {
            const bytes = Deno.core.ops.op_fs_read(path);
            return {
                status: 200,
                statusText: "OK",
                headers: { "content-type": mini.contentTypeFor(path) },
                bytes,
            };
        } catch {
            return { status: 404, statusText: "Not Found", headers: {}, bytes: null };
        }
    }

    async function hop(opts, handler, controller, id) {
        const { url, hops, credentials, clientOrigin, mode } = opts.opaque;
        if (opts.signal?.aborted) throw opts.signal.reason;
        if (opts.upgrade) throw new TypeError(`${opts.upgrade} is not supported`);
        // jsdom's dispatcher knows nothing of same-origin mode, so a redirect target must be
        // refused here, before it reaches the wire.
        const hopUrl = new URL(url);
        if (mode === "same-origin" && hopUrl.protocol !== "data:" && hopUrl.origin !== clientOrigin)
            throw new TypeError("cross-origin redirect in same-origin mode");

        // A preflight is not a hop of the request's own redirect chain.
        const preflight =
            opts.method === "OPTIONS" && header(opts.headers, "access-control-request-method");
        if (!preflight) hops?.push(url);

        const mounted = fromMount(url);
        if (mounted) {
            emit(
                handler,
                controller,
                mounted.status,
                mounted.statusText,
                mounted.headers,
                mounted.bytes,
            );
            return;
        }

        // The jar lives in JSDOMDispatcher, above this layer, and applies the cookie rules of
        // the CORS flags it was given. `credentials` is the fetch rule on top: "omit" never
        // sends or stores, "same-origin" only while every hop so far stayed on the page's origin (a
        // chain that left it is tainted for good; no-cors requests carry no origin for jsdom).
        const useCookies =
            credentials === undefined ||
            credentials === "include" ||
            (credentials === "same-origin" &&
                (hops ?? [url]).every((u) => new URL(u).origin === clientOrigin));
        const body =
            typeof opts.body === "string" ? new TextEncoder().encode(opts.body) : opts.body;
        const res = await Deno.core.ops.op_fetch_hop({
            url,
            method: opts.method,
            headers: useCookies ? opts.headers : omit(opts.headers, "cookie"),
            body: body ?? null,
            id,
        });

        const headers = {};
        for (const [name, value] of res.headers) {
            const key = name.toLowerCase();
            if (key === "set-cookie") {
                if (useCookies) (headers[key] ??= []).push(value);
            } else {
                headers[key] = key in headers ? `${headers[key]}, ${value}` : value;
            }
        }
        emit(
            handler,
            controller,
            res.status,
            res.statusText ?? "",
            headers,
            res.body && new Uint8Array(res.body),
        );
    }

    // jsdom reports stylesheet failures through the <link> element, so a transport failure
    // there becomes a 404; a failed script must still fail loudly.
    const failsAsNotFound = (opaque) =>
        opaque.element?.localName === "link" && opaque.element.relList?.contains("stylesheet");

    mini.baseDispatcher = {
        dispatch(opts, handler) {
            const id = crypto.randomUUID();
            const controller = makeController(() => Deno.core.ops.op_fetch_abort(id));
            const onAbort = () => controller.abort(opts.signal.reason);
            handler.onRequestStart?.(controller, {});
            opts.signal?.addEventListener("abort", onAbort);
            hop(opts, handler, controller, id)
                .catch((error) => {
                    // Cancelling the Python-side task surfaces as a generic failure.
                    error = controller.aborted ? controller.reason : error;
                    if (failsAsNotFound(opts.opaque) && !controller.aborted) {
                        emit(handler, controller, 404, "Not Found", {}, null);
                    } else {
                        handler.onResponseError?.(controller, error);
                    }
                })
                .finally(() => opts.signal?.removeEventListener("abort", onAbort));
            return true;
        },
        close: () => Promise.resolve(),
        destroy: () => Promise.resolve(),
    };

    // ---- request side ------------------------------------------------------------------

    // https://fetch.spec.whatwg.org/#forbidden-request-header
    const FORBIDDEN_HEADERS = new Set([
        "accept-charset",
        "accept-encoding",
        "access-control-request-headers",
        "access-control-request-method",
        "connection",
        "content-length",
        "cookie",
        "cookie2",
        "date",
        "dnt",
        "expect",
        "host",
        "keep-alive",
        "origin",
        "referer",
        "set-cookie",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "via",
    ]);
    const FORBIDDEN_METHODS = /^(connect|trace|track)$/i;
    const isForbiddenHeader = (name, value) => {
        name = name.toLowerCase();
        if (FORBIDDEN_HEADERS.has(name) || name.startsWith("proxy-") || name.startsWith("sec-"))
            return true;
        return (
            /^x-http-method(-override)?$|^x-method-override$/.test(name) &&
            value.split(",").some((m) => FORBIDDEN_METHODS.test(m.trim()))
        );
    };

    // https://fetch.spec.whatwg.org/#cors-unsafe-request-header-names
    const SAFELISTED_VALUE = /^[^\x00-\x08\x0a-\x1f"():<>?@[\\\]{}\x7f]*$/;
    const SAFELISTED_CONTENT_TYPES =
        /^(application\/x-www-form-urlencoded|multipart\/form-data|text\/plain)$/i;
    function unsafeHeaderNames(headers) {
        const unsafe = new Set();
        for (const [name, value] of headers) {
            const safe =
                value.length <= 128 &&
                (((name === "accept" ||
                    name === "accept-language" ||
                    name === "content-language") &&
                    SAFELISTED_VALUE.test(value)) ||
                    (name === "content-type" &&
                        SAFELISTED_CONTENT_TYPES.test(value.split(";")[0].trim())) ||
                    (name === "range" && /^bytes=\d+-\d*$/.test(value)));
            if (!safe) unsafe.add(name);
        }
        return [...unsafe].sort();
    }

    // https://w3c.github.io/webappsec-referrer-policy/#determine-requests-referrer
    function referrerFor(req, location) {
        if (req.referrer === "" || !/^https?:$/.test(location.protocol)) return null;
        const full = new URL(req.referrer === "about:client" ? location.href : req.referrer);
        full.hash = "";
        full.username = full.password = "";
        const origin = `${full.origin}/`;
        const target = new URL(req.url);
        const sameOrigin = full.origin === target.origin;
        const downgrade = full.protocol === "https:" && target.protocol !== "https:";
        switch (req.referrerPolicy || "strict-origin-when-cross-origin") {
            case "no-referrer":
                return null;
            case "origin":
                return origin;
            case "unsafe-url":
                return full.href;
            case "same-origin":
                return sameOrigin ? full.href : null;
            case "origin-when-cross-origin":
                return sameOrigin ? full.href : origin;
            case "no-referrer-when-downgrade":
                return downgrade ? null : full.href;
            case "strict-origin":
                return downgrade ? null : origin;
            default:
                return sameOrigin ? full.href : downgrade ? null : origin;
        }
    }

    // deno's Request brand-checks its own Blob/FormData and reads anything else as a string
    // ("[object Blob]"), so the page's jsdom ones are converted to bodies it understands.
    async function denoBody(win, body) {
        const { Blob: DenoBlob, File: DenoFile } =
            Deno.core.loadExtScript("ext:deno_web/09_file.js");
        if (body instanceof win.FormData) {
            const form = new D.FormData();
            for (const [name, value] of body) {
                if (value instanceof win.File) {
                    const bytes = await value.arrayBuffer();
                    form.append(
                        name,
                        new DenoFile([bytes], value.name, { type: value.type }),
                        value.name,
                    );
                } else {
                    form.append(name, value);
                }
            }
            return { body: form };
        }
        if (body instanceof win.Blob) {
            return { body: new Uint8Array(await body.arrayBuffer()), type: body.type || null };
        }
        return { body };
    }

    async function buildRequest(win, input, init) {
        if (!(input instanceof D.Request)) input = new URL(input, win.location.href).href;
        if (init.body === undefined || init.body === null) return new D.Request(input, init);
        const { body, type } = await denoBody(win, init.body);
        const headers = new D.Headers(init.headers);
        if (type && !headers.has("content-type")) headers.set("content-type", type);
        return new D.Request(input, { ...init, body, headers });
    }

    // ---- response side -----------------------------------------------------------------

    // https://fetch.spec.whatwg.org/#cors-safelisted-response-header-name
    const CORS_SAFELISTED_RESPONSE = new Set([
        "cache-control",
        "content-language",
        "content-length",
        "content-type",
        "expires",
        "last-modified",
        "pragma",
    ]);
    function corsFilter(pairs, credentials) {
        const exposed = new Set();
        for (const [name, value] of pairs) {
            if (name === "access-control-expose-headers") {
                for (const item of value.split(",")) exposed.add(item.trim().toLowerCase());
            }
        }
        const all = exposed.has("*") && credentials !== "include";
        return pairs.filter(
            ([name]) => CORS_SAFELISTED_RESPONSE.has(name) || all || exposed.has(name),
        );
    }

    const NULL_BODY_STATUS = new Set([101, 204, 205, 304]);
    const REDIRECT_STATUS = new Set([301, 302, 303, 307, 308]);

    function buildResponse(req, result, hops, clientOrigin) {
        const inner = D.newInnerResponse(result.status, result.statusText);
        let pairs = Object.entries(result.headers)
            .filter(([name]) => name !== "set-cookie" && name !== "set-cookie2")
            // The list bypasses Headers' own value normalization.
            .map(([name, value]) => [name, value.replace(/^[\t\n\r ]+|[\t\n\r ]+$/g, "")]);
        const crossOrigin = hops.some((h) => {
            const hop = new URL(h);
            return hop.protocol !== "data:" && hop.origin !== clientOrigin;
        });
        inner.urlList = hops;
        inner.type = "basic";
        if (
            req.redirect === "manual" &&
            REDIRECT_STATUS.has(result.status) &&
            result.headers.location
        ) {
            inner.type = "opaqueredirect";
        } else if (req.mode === "no-cors" && crossOrigin) {
            inner.type = "opaque";
            inner.urlList = [];
        } else if (req.mode === "cors" && crossOrigin) {
            inner.type = "cors";
            pairs = corsFilter(pairs, req.credentials);
        }
        if (inner.type === "opaque" || inner.type === "opaqueredirect") {
            inner.status = 0;
            inner.statusMessage = "";
            pairs = [];
        } else if (!NULL_BODY_STATUS.has(result.status)) {
            inner.body = new D.InnerBody({ body: result.body, consumed: false });
        }
        inner.headerList = pairs;
        return D.fromInnerResponse(inner, "immutable");
    }

    function collect(dispatcher, opts) {
        return new Promise((resolve, reject) => {
            const chunks = [];
            let head;
            dispatcher.dispatch(opts, {
                onResponseStart: (_, status, headers, statusText) => {
                    head = { status, headers, statusText };
                },
                onResponseData: (_, chunk) => chunks.push(chunk),
                onResponseEnd: () => {
                    const body = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
                    let offset = 0;
                    for (const chunk of chunks) {
                        body.set(chunk, offset);
                        offset += chunk.length;
                    }
                    resolve({ ...head, body });
                },
                onResponseError: (_, error) => reject(error),
            });
        });
    }

    // ---- fetch -------------------------------------------------------------------------

    mini.makeFetch = (win) =>
        async function fetch(input, init = {}) {
            const req = await buildRequest(win, input, init ?? {});
            const { signal } = req;
            if (signal.aborted) throw signal.reason;

            const url = new URL(req.url);
            const clientOrigin = win.location.origin;
            const crossOrigin = url.origin !== clientOrigin;
            const failed = (message, cause) =>
                new TypeError(`Failed to fetch: ${message}`, { cause });
            if (url.protocol !== "http:" && url.protocol !== "https:" && url.protocol !== "data:") {
                throw failed(`unsupported scheme ${url.protocol}`);
            }
            if (url.protocol !== "data:" && req.mode === "same-origin" && crossOrigin) {
                throw failed("cross-origin request in same-origin mode");
            }
            if (req.mode === "no-cors" && !/^(GET|HEAD|POST)$/.test(req.method)) {
                throw failed(`method ${req.method} in no-cors mode`);
            }

            const cors = req.mode === "cors" && url.protocol !== "data:";
            const authorHeaders = [...req.headers].filter(
                ([name, value]) => !isForbiddenHeader(name, value),
            );
            const headers = Object.fromEntries(authorHeaders);
            if ((cors && crossOrigin) || !/^(GET|HEAD)$/.test(req.method))
                headers.origin = clientOrigin;
            const referrer = url.protocol === "data:" ? null : referrerFor(req, win.location);
            if (referrer) headers.referer = referrer;

            const unsafe = unsafeHeaderNames(authorHeaders);
            const needsPreflight =
                cors && crossOrigin && (!/^(GET|HEAD|POST)$/.test(req.method) || unsafe.length > 0);
            const hops = [];
            const opts = {
                method: req.method,
                headers,
                body: req.body ? new Uint8Array(await req.arrayBuffer()) : null,
                signal,
                opaque: {
                    url: url.href,
                    // no-cors responses are opaque, so jsdom's CORS check must not run on them.
                    origin: req.mode === "no-cors" ? undefined : clientOrigin,
                    corsMode: cors,
                    withCredentials: req.credentials === "include",
                    redirect: req.redirect,
                    preflight: needsPreflight ? { unsafeHeaders: unsafe } : undefined,
                    credentials: req.credentials,
                    clientOrigin,
                    mode: req.mode,
                    hops,
                },
            };

            // Reject with the signal's reason the moment it fires, even between redirect hops.
            let onAbort;
            const aborted = new Promise((_, reject) => {
                onAbort = () => reject(signal.reason);
                signal.addEventListener("abort", onAbort);
            });
            try {
                const result = await Promise.race([collect(win._dispatcher, opts), aborted]);
                return buildResponse(req, result, hops, clientOrigin);
            } catch (error) {
                if (signal.aborted) throw signal.reason;
                throw error instanceof TypeError
                    ? error
                    : failed(String(error?.message ?? error), error);
            } finally {
                signal.removeEventListener("abort", onAbort);
            }
        };
})();
