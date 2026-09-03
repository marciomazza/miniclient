import json


async def js_fetch_text(runtime, url, **opts):
    js_opts = json.dumps(opts, ensure_ascii=False) if opts else "{}"
    return await runtime.eval_async(f"fetch({json.dumps(url)}, {js_opts}).then(r => r.text())")


async def js_fetch_json(runtime, url, **opts):
    js_opts = json.dumps(opts, ensure_ascii=False) if opts else "{}"
    return await runtime.eval_async(f"fetch({json.dumps(url)}, {js_opts}).then(r => r.json())")
