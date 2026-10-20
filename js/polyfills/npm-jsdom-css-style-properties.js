"use strict";

// jsdom's webidl2js wrapper for CSSStyleProperties, generated from an IDL that lists only
// cssFloat; the install loop below adds the other property attributes.

const conversions = require("webidl-conversions");
const utils = require("../../node_modules/jsdom/lib/generated/idl/utils.js");

const ceReactionsPreSteps_jsdom_living_helpers_custom_elements =
    require("../../node_modules/jsdom/lib/jsdom/living/helpers/custom-elements.js").ceReactionsPreSteps;
const ceReactionsPostSteps_jsdom_living_helpers_custom_elements =
    require("../../node_modules/jsdom/lib/jsdom/living/helpers/custom-elements.js").ceReactionsPostSteps;
const ctorRegistrySymbol = utils.ctorRegistrySymbol;
const CSSStyleDeclaration = require("../../node_modules/jsdom/lib/generated/idl/CSSStyleDeclaration.js");
const propertyNames = require("../../node_modules/jsdom/lib/jsdom/living/css/style-property-names.json");

const interfaceName = "CSSStyleProperties";

const $interfaceDescriptor = utils.createInterfaceDescriptor(
    () => CSSStyleDeclaration.interfaceDescriptor,
);
exports.interfaceDescriptor = $interfaceDescriptor;

function $requireImpl(wrapper, globalObject, context) {
    return utils.requireImplForWrapper(
        wrapper,
        $interfaceDescriptor,
        globalObject,
        interfaceName,
        context,
    );
}

exports.is = (value) => {
    return utils.implForWrapperWithInterface(value, $interfaceDescriptor) !== null;
};
exports.isImpl = (value) => {
    return utils.isObject(value) && value instanceof Impl.implementation;
};
exports.convert = (globalObject, value, { context = "The provided value" } = {}) => {
    const impl = utils.implForWrapperWithInterface(value, $interfaceDescriptor);
    if (impl !== null) {
        return impl;
    }
    throw new globalObject.TypeError(`${context} is not of type 'CSSStyleProperties'.`);
};

function makeWrapper(globalObject, newTarget) {
    let proto;
    if (newTarget !== undefined) {
        proto = newTarget.prototype;
    }

    if (!utils.isObject(proto)) {
        proto = globalObject[ctorRegistrySymbol]["CSSStyleProperties"].prototype;
    }

    return Object.create(proto);
}

function makeProxy(wrapper, impl, globalObject) {
    let proxyHandler = proxyHandlerCache.get(globalObject);
    if (proxyHandler === undefined) {
        proxyHandler = new ProxyHandler(globalObject);
        proxyHandlerCache.set(globalObject, proxyHandler);
    }
    // The target needs an implementation for proxy traps, but only the final proxy gets the interface brand.
    utils.registerWrapper(wrapper, impl, undefined);
    return new Proxy(wrapper, proxyHandler);
}

exports.create = (globalObject, constructorArgs, privateData) => {
    const wrapper = makeWrapper(globalObject);
    return exports.setup(wrapper, globalObject, constructorArgs, privateData);
};

exports.createImpl = (globalObject, constructorArgs, privateData) => {
    const wrapper = exports.create(globalObject, constructorArgs, privateData);
    return utils.implForWrapper(wrapper);
};

exports._internalSetup = (wrapper, globalObject) => {
    CSSStyleDeclaration._internalSetup(wrapper, globalObject);
};

exports.setup = (wrapper, globalObject, constructorArgs = [], privateData = {}) => {
    privateData.wrapper = wrapper;

    exports._internalSetup(wrapper, globalObject);
    const impl = new Impl.implementation(globalObject, constructorArgs, privateData);

    wrapper = makeProxy(wrapper, impl, globalObject);

    utils.registerWrapper(wrapper, impl, $interfaceDescriptor);
    impl[utils.wrapperSymbol] = wrapper;
    if (Impl.init) {
        Impl.init(impl);
    }
    return wrapper;
};

exports.new = (globalObject, newTarget) => {
    let wrapper = makeWrapper(globalObject, newTarget);

    exports._internalSetup(wrapper, globalObject);
    const impl = Object.create(Impl.implementation.prototype);

    wrapper = makeProxy(wrapper, impl, globalObject);

    utils.registerWrapper(wrapper, impl, $interfaceDescriptor);
    impl[utils.wrapperSymbol] = wrapper;
    if (Impl.init) {
        Impl.init(impl);
    }
    return impl;
};

const exposed = new Set(["Window"]);

exports.install = (globalObject, globalNames) => {
    if (!globalNames.some((globalName) => exposed.has(globalName))) {
        return;
    }

    const ctorRegistry = utils.initCtorRegistry(globalObject);
    class CSSStyleProperties extends globalObject.CSSStyleDeclaration {
        constructor() {
            throw new globalObject.TypeError("Illegal constructor");
        }

        get cssFloat() {
            const $impl = $requireImpl(this ?? globalObject, globalObject, "get cssFloat");

            ceReactionsPreSteps_jsdom_living_helpers_custom_elements(globalObject);
            try {
                return utils.tryWrapperForImpl($impl["cssFloat"]);
            } finally {
                ceReactionsPostSteps_jsdom_living_helpers_custom_elements(globalObject);
            }
        }

        set cssFloat(V) {
            const $impl = $requireImpl(this ?? globalObject, globalObject, "set cssFloat");

            V = conversions["DOMString"](V, {
                context:
                    "Failed to set the 'cssFloat' property on 'CSSStyleProperties': The provided value",
                globals: globalObject,
                treatNullAsEmptyString: true,
            });

            ceReactionsPreSteps_jsdom_living_helpers_custom_elements(globalObject);
            try {
                $impl["cssFloat"] = V;
            } finally {
                ceReactionsPostSteps_jsdom_living_helpers_custom_elements(globalObject);
            }
        }
    }
    Object.defineProperties(CSSStyleProperties.prototype, {
        cssFloat: { enumerable: true },
        [Symbol.toStringTag]: { value: "CSSStyleProperties", configurable: true },
        [Symbol.iterator]: {
            value: globalObject.Array.prototype[Symbol.iterator],
            configurable: true,
            writable: true,
        },
    });
    // Each accessor repeats the steps webidl2js generates for cssFloat.
    for (const name of Object.keys(propertyNames)) {
        if (name === "cssFloat") {
            continue;
        }
        const accessors = {
            get [name]() {
                const $impl = $requireImpl(this ?? globalObject, globalObject, `get ${name}`);

                ceReactionsPreSteps_jsdom_living_helpers_custom_elements(globalObject);
                try {
                    return utils.tryWrapperForImpl($impl[name]);
                } finally {
                    ceReactionsPostSteps_jsdom_living_helpers_custom_elements(globalObject);
                }
            },

            set [name](V) {
                const $impl = $requireImpl(this ?? globalObject, globalObject, `set ${name}`);

                V = conversions["DOMString"](V, {
                    context: `Failed to set the '${name}' property on 'CSSStyleProperties': The provided value`,
                    globals: globalObject,
                    treatNullAsEmptyString: true,
                });

                ceReactionsPreSteps_jsdom_living_helpers_custom_elements(globalObject);
                try {
                    $impl[name] = V;
                } finally {
                    ceReactionsPostSteps_jsdom_living_helpers_custom_elements(globalObject);
                }
            },
        };
        const { get, set } = Object.getOwnPropertyDescriptor(accessors, name);
        Object.defineProperty(CSSStyleProperties.prototype, name, {
            get,
            set,
            enumerable: true,
            configurable: true,
        });
    }
    ctorRegistry[interfaceName] = CSSStyleProperties;

    Object.defineProperty(globalObject, interfaceName, {
        configurable: true,
        writable: true,
        value: CSSStyleProperties,
    });
};

const proxyHandlerCache = new WeakMap();
class ProxyHandler {
    constructor(globalObject) {
        this._globalObject = globalObject;
    }

    get(target, P, receiver) {
        if (typeof P === "symbol") {
            return Reflect.get(target, P, receiver);
        }
        const impl = utils.implForWrapper(target);

        if (utils.isArrayIndexPropName(P)) {
            const index = P >>> 0;

            if (impl[utils.supportsPropertyIndex](index)) {
                const indexedValue = impl.item(index);
                return utils.tryWrapperForImpl(indexedValue);
            }
        }

        return Reflect.get(target, P, receiver);
    }

    has(target, P) {
        if (typeof P === "symbol") {
            return Reflect.has(target, P);
        }
        const desc = this.getOwnPropertyDescriptor(target, P);
        if (desc !== undefined) {
            return true;
        }
        const parent = Object.getPrototypeOf(target);
        if (parent !== null) {
            return Reflect.has(parent, P);
        }
        return false;
    }

    ownKeys(target) {
        const impl = utils.implForWrapper(target);
        const keys = new Set();

        for (const key of impl[utils.supportedPropertyIndices]) {
            keys.add(`${key}`);
        }

        for (const key of Reflect.ownKeys(target)) {
            keys.add(key);
        }
        return [...keys];
    }

    getOwnPropertyDescriptor(target, P) {
        if (typeof P === "symbol") {
            return Reflect.getOwnPropertyDescriptor(target, P);
        }
        const impl = utils.implForWrapper(target);

        if (utils.isArrayIndexPropName(P)) {
            const index = P >>> 0;

            if (impl[utils.supportsPropertyIndex](index)) {
                const indexedValue = impl.item(index);
                return {
                    writable: false,
                    enumerable: true,
                    configurable: true,
                    value: utils.tryWrapperForImpl(indexedValue),
                };
            }
        }

        return Reflect.getOwnPropertyDescriptor(target, P);
    }

    set(target, P, V, receiver) {
        if (typeof P === "symbol") {
            return Reflect.set(target, P, V, receiver);
        }
        const impl = utils.implForWrapper(target);
        // The `receiver` argument refers to the Proxy exotic object or an object
        // that inherits from it, whereas `target` refers to the Proxy target:
        if (utils.wrapperForImpl(impl) === receiver) {
            const globalObject = this._globalObject;
        }
        let ownDesc;

        if (utils.isArrayIndexPropName(P)) {
            const index = P >>> 0;

            if (impl[utils.supportsPropertyIndex](index)) {
                const indexedValue = impl.item(index);
                ownDesc = {
                    writable: false,
                    enumerable: true,
                    configurable: true,
                    value: utils.tryWrapperForImpl(indexedValue),
                };
            }
        }

        if (ownDesc === undefined) {
            ownDesc = Reflect.getOwnPropertyDescriptor(target, P);
        }
        return utils.ordinarySetWithOwnDescriptor(target, P, V, receiver, ownDesc);
    }

    defineProperty(target, P, desc) {
        if (typeof P === "symbol") {
            return Reflect.defineProperty(target, P, desc);
        }
        const impl = utils.implForWrapper(target);

        const globalObject = this._globalObject;

        if (utils.isArrayIndexPropName(P)) {
            return false;
        }

        return Reflect.defineProperty(target, P, desc);
    }

    deleteProperty(target, P) {
        if (typeof P === "symbol") {
            return Reflect.deleteProperty(target, P);
        }
        const impl = utils.implForWrapper(target);

        const globalObject = this._globalObject;

        if (utils.isArrayIndexPropName(P)) {
            const index = P >>> 0;
            return !impl[utils.supportsPropertyIndex](index);
        }

        return Reflect.deleteProperty(target, P);
    }

    preventExtensions() {
        return false;
    }
}

const Impl = require("../../node_modules/jsdom/lib/jsdom/living/css/CSSStyleProperties-impl.js");
