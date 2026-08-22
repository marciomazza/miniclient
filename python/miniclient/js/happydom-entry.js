import { Browser, PropertySymbol } from "happy-dom";
import CookieStringUtility from "happy-dom/lib/cookie/urilities/CookieStringUtility.js";
import FetchCORSUtility from "happy-dom/lib/fetch/utilities/FetchCORSUtility.js";
import WindowBrowserContext from "happy-dom/lib/window/WindowBrowserContext.js";
import DetachedWindowAPI from "happy-dom/lib/window/DetachedWindowAPI.js";
import BrowserFrameNavigator from "happy-dom/lib/browser/utilities/BrowserFrameNavigator.js";
import patchHappyDom from "./patch-happy-dom.js";
import { __refreshStreamGlobals } from "./polyfills/node-stream-web.js";

export {
    Browser,
    PropertySymbol,
    CookieStringUtility,
    FetchCORSUtility,
    WindowBrowserContext,
    DetachedWindowAPI,
    BrowserFrameNavigator,
    patchHappyDom,
    __refreshStreamGlobals,
};
