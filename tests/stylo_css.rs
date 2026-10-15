//! stylo's computed values against jsdom's and against a Chromium recording of the CSS corpus.
//! The cost test is a report: run it with `-- --ignored --nocapture`.

#![cfg(feature = "stylo-css")]

mod common;

use std::collections::BTreeMap;

use common::EvalExt;
use common::fetch_mock::{MockFetchBackend, runtime_with_mock};
use serde::Deserialize;

/// `compare()` diffs every element of the page on every property jsdom lists for it.
const HARNESS: &str = r#"
globalThis.__stylo = (el, props) => {
    const impl = __jsdomImplForWrapper(el);
    const focus = impl._ownerDocument._lastFocusedElement?._wire ?? -1;
    return Deno.core.ops.op_css_computed_values(impl._wire, focus, -1, props);
};
globalThis.__compare = () => {
    const els = [...document.querySelectorAll("*")];
    const oracle = els.map((el) => {
        const cs = getComputedStyle(el);
        return [...cs].filter((p) => !p.startsWith("--")).map((p) => [p, cs.getPropertyValue(p)]);
    });
    const ours = els.map((el, i) => __stylo(el, oracle[i].map(([p]) => p)));
    const diffs = [];
    let pairs = 0;
    oracle.forEach((props, i) => props.forEach(([p, v], j) => {
        pairs++;
        if (v !== ours[i][j]) diffs.push([els[i].localName, p, v, ours[i][j]]);
    }));
    return JSON.stringify({ pairs, diffs });
};
0
"#;

#[derive(Deserialize)]
struct Outcome {
    pairs: usize,
    diffs: Vec<(String, String, String, String)>,
}

fn compare(rt: &common::Runtime, markup: &str) -> Outcome {
    let markup = serde_json::to_string(markup).unwrap();
    let json = rt.eval::<String>(&format!(
        "document.documentElement.innerHTML = {markup}; __compare()"
    ));
    serde_json::from_str(&json).unwrap()
}

fn runtime() -> common::Runtime {
    let rt = runtime_with_mock(&MockFetchBackend::new());
    rt.run(HARNESS);
    rt
}

#[test]
fn cascade_basics_agree() {
    let rt = runtime();
    let out = compare(
        &rt,
        r#"<head><style>
            :root { --gap: 4px }
            .a { color: rgb(0, 0, 255); margin-top: var(--gap) }
            #b.a { color: green }
            p { display: inline-block !important }
            .c { & span { visibility: hidden } }
            @media (min-width: 600px) { .a { padding-left: 3px } }
            input:checked { outline-style: solid }
        </style></head>
        <body><div class="a"><p id="b" class="a" style="display: block">x</p>
        <div class="c"><span>y</span></div><input type="checkbox" checked></div></body>"#,
    );
    assert!(out.pairs > 0);
    let diff = |el: &str, p: &str, jsdom: &str, stylo: &str| {
        (el.into(), p.into(), jsdom.into(), stylo.into())
    };
    // jsdom leaves var() unresolved and echoes specified values; stylo matches browsers there.
    assert_eq!(
        out.diffs,
        vec![
            diff("div", "margin-top", "var(--gap)", "4px"),
            diff("p", "margin-block", "1em", "4px 16px"),
            diff("p", "margin-top", "var(--gap)", "4px"),
            diff("input", "word-spacing", "normal", "0px"),
            diff("input", "text-indent", "0", "0px"),
            diff("input", "text-shadow", "rgba(0, 0, 0, 0)", "none"),
        ]
    );
}

#[test]
fn reflects_mutations() {
    let rt = runtime();
    compare(
        &rt,
        "<head><style>.on { color: rgb(1, 2, 3) }</style></head><body><p>x</p></body>",
    );
    let color: String = rt.eval(
        r#"const p = document.querySelector("p"); p.className = "on"; __stylo(p, ["color"])[0]"#,
    );
    assert_eq!(color, "rgb(1, 2, 3)");
}

const WPT: &str = "vendor/jsdom/test/web-platform-tests";

fn strip_scripts(html: &str) -> String {
    let re = regex::Regex::new(r"(?is)<script\b.*?</script\s*>").unwrap();
    re.replace_all(html, "").into_owned()
}

fn rss_kb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// Run with `cargo test --release --features stylo-css --test stylo_css cost -- --ignored
/// --nocapture`.
#[test]
#[ignore = "report, not a gate"]
fn cost_report() {
    let rt = runtime();
    let rules: String = (0..50)
        .map(|i| {
            format!(
                ".c{i} {{ margin-left: {i}px }} .c{i} > .c{} span {{ color: red }}\n",
                i + 1
            )
        })
        .collect();
    let body: String = (0..500)
        .map(|i| format!("<div class=\"c{}\"><span>{i}</span></div>", i % 50))
        .collect();
    let markup = serde_json::to_string(&format!(
        "<head><style>{rules}</style></head><body>{body}</body>"
    ))
    .unwrap();
    rt.run(&format!("document.documentElement.innerHTML = {markup}; 0"));
    rt.run(
        r#"var els = [...document.querySelectorAll("span")], el = els[250], r = {};
        var time = (f) => { const t = performance.now(); f(); return performance.now() - t; };
        var avg = (n, f) => { let s = 0; for (let i = 0; i < n; i++) s += f(i); return s / n; };
        0"#,
    );
    let before = rss_kb();
    rt.run(r#"r.styloCold = time(() => __stylo(el, ["color"])); 0"#);
    let stylo_rss = rss_kb().saturating_sub(before);
    let json: String = rt.eval(
        r#"(() => {
            r.styloWarm = avg(200, () => time(() => __stylo(el, ["color"])));
            r.styloOther = avg(200, (i) => time(() => __stylo(els[i], ["color"])));
            r.jsdomFirst = time(() => getComputedStyle(el).color);
            r.jsdomWarm = avg(200, () => time(() => getComputedStyle(el).color));
            r.styloAfterMutation = avg(20, (i) => {
                els[i].setAttribute("data-x", i);
                return time(() => __stylo(el, ["color"]));
            });
            r.jsdomAfterMutation = avg(20, (i) => {
                els[i].setAttribute("data-y", i);
                return time(() => getComputedStyle(el).color);
            });
            return JSON.stringify(r);
        })()"#,
    );
    println!("elements 1001, rules 100; times in ms: {json}");
    println!("rss growth over the first stylo call: +{stylo_rss} KiB");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    chromium: String,
    viewport: (u32, u32),
    color_scheme: String,
    properties: Vec<String>,
    defaults: BTreeMap<String, String>,
    files: BTreeMap<String, Vec<(String, BTreeMap<String, String>)>>,
}

/// Where jsdom's own cascade agreed with Chromium; that cascade no longer runs, so this cannot be
/// re-recorded. Per element, an index into `patterns`, each a hex bitset over
/// `Golden::properties`.
#[derive(Deserialize)]
struct JsdomVerdicts {
    patterns: Vec<String>,
    files: BTreeMap<String, Vec<usize>>,
}

impl JsdomVerdicts {
    fn agreed(&self, file: &str, element: usize, prop: usize) -> bool {
        let hex = &self.patterns[self.files[file][element]];
        let byte = u8::from_str_radix(&hex[prop / 8 * 2..prop / 8 * 2 + 2], 16).unwrap();
        byte & (1 << (prop % 8)) != 0
    }
}

/// `property [file]` lines; either ending in `*` is a prefix, no file means every file.
fn allow_list(text: &str) -> Vec<(String, Option<String>)> {
    text.lines()
        .map(|line| line.split('#').next().unwrap().trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut words = line.split_whitespace();
            let prop = words.next().unwrap().to_owned();
            (prop, words.next().map(str::to_owned))
        })
        .collect()
}

/// Index of the first entry covering the pair.
fn allowed(allow: &[(String, Option<String>)], file: &str, prop: &str) -> Option<usize> {
    let matches = |pattern: &str, s: &str| match pattern.strip_suffix('*') {
        Some(prefix) => s.starts_with(prefix),
        None => s == pattern,
    };
    allow.iter().position(|(prop_pattern, file_pattern)| {
        matches(prop_pattern, prop) && file_pattern.as_deref().is_none_or(|f| matches(f, file))
    })
}

#[test]
fn allow_list_matching() {
    let allow = allow_list("# reason\nwidth\nscroll-snap-*  # gap\ncolor a.html\ndisplay d/*\n");
    assert!(allowed(&allow, "x.html", "width").is_some());
    assert!(allowed(&allow, "x.html", "min-width").is_none());
    assert!(allowed(&allow, "x.html", "scroll-snap-type").is_some());
    assert!(allowed(&allow, "a.html", "color").is_some());
    assert!(allowed(&allow, "b.html", "color").is_none());
    assert!(allowed(&allow, "d/x.html", "display").is_some());
    assert!(allowed(&allow, "e/x.html", "display").is_none());
}

/// Gate against Chromium's recorded values: a pair jsdom got right (`css_golden/jsdom.json`)
/// must stay right under stylo, unless `css_golden/allow.txt` lists it. Re-record with
/// `scripts/record-chromium-css.py`; the jsdom verdicts cannot be re-recorded, so a new file
/// needs its entry built some other way.
#[test]
fn chromium_golden() {
    let golden: Golden =
        serde_json::from_str(&std::fs::read_to_string("tests/css_golden/chromium.json").unwrap())
            .unwrap();
    let jsdom: JsdomVerdicts =
        serde_json::from_str(&std::fs::read_to_string("tests/css_golden/jsdom.json").unwrap())
            .unwrap();
    let allow = allow_list(&std::fs::read_to_string("tests/css_golden/allow.txt").unwrap());
    let rt = runtime();
    let window: (u32, u32) =
        serde_json::from_str(&rt.eval::<String>("JSON.stringify([innerWidth, innerHeight])"))
            .unwrap();
    // A recording on another device would mismatch media queries and viewport units.
    assert_eq!(
        golden.viewport, window,
        "recorded viewport differs from mini's window"
    );
    assert_eq!(
        golden.color_scheme, "light",
        "mini's device prefers the light scheme"
    );
    let props = serde_json::to_string(&golden.properties).unwrap();
    rt.run(&format!(
        r#"globalThis.__golden = (markup) => {{
            document.documentElement.innerHTML = markup;
            const props = {props};
            return JSON.stringify(
                [...document.querySelectorAll("*")].map((el) => [el.localName, __stylo(el, props)])
            );
        }};
        0"#
    ));
    let (mut pairs, mut stylo_eq, mut jsdom_eq, mut both) = (0, 0, 0, 0);
    let mut unread = Vec::new();
    let mut regressions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut used = vec![false; allow.len()];
    for (file, expected) in &golden.files {
        let html = std::fs::read_to_string(format!("{WPT}/{file}")).unwrap();
        let markup = serde_json::to_string(&strip_scripts(&html)).unwrap();
        let json = match rt.try_eval(&format!("__golden({markup})")) {
            Ok(Some(json)) => serde_json::from_str::<String>(&json).unwrap(),
            other => {
                unread.push(format!("{file}: {other:?}"));
                continue;
            }
        };
        let ours: Vec<(String, Vec<String>)> = serde_json::from_str(&json).unwrap();
        if ours
            .iter()
            .map(|element| &element.0)
            .ne(expected.iter().map(|element| &element.0))
        {
            unread.push(format!("{file}: element tree differs from Chromium's"));
            continue;
        }
        assert_eq!(
            jsdom.files[file].len(),
            expected.len(),
            "{file}: jsdom.json is stale"
        );
        for (n, ((_, stylo), (el, deltas))) in ours.iter().zip(expected).enumerate() {
            for (i, prop) in golden.properties.iter().enumerate() {
                let want = deltas.get(prop).unwrap_or(&golden.defaults[prop]);
                let (stylo_ok, jsdom_ok) = (&stylo[i] == want, jsdom.agreed(file, n, i));
                pairs += 1;
                stylo_eq += stylo_ok as usize;
                jsdom_eq += jsdom_ok as usize;
                both += (stylo_ok && jsdom_ok) as usize;
                if !jsdom_ok || stylo_ok {
                    continue;
                }
                if let Some(entry) = allowed(&allow, file, prop) {
                    used[entry] = true;
                } else {
                    let lost = regressions.entry(prop.clone()).or_default();
                    lost.push(format!(
                        "{file} <{el}>: chromium {want:?} stylo {:?}",
                        stylo[i]
                    ));
                }
            }
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / pairs.max(1) as f64;
    println!(
        "Chromium {}: {} files ({} unread), {pairs} pairs",
        golden.chromium,
        golden.files.len(),
        unread.len()
    );
    println!("  stylo = Chromium {stylo_eq} ({:.1}%)", pct(stylo_eq));
    println!("  jsdom = Chromium {jsdom_eq} ({:.1}%)", pct(jsdom_eq));
    println!("  both             {both} ({:.1}%)", pct(both));
    for u in &unread {
        println!("  unread {u}");
    }
    for (prop, lost) in &regressions {
        println!("{:6} {prop}", lost.len());
        for sample in lost.iter().take(3) {
            println!("         {sample}");
        }
    }
    let stale: Vec<_> = allow
        .iter()
        .zip(&used)
        .filter(|(_, was_used)| !**was_used)
        .map(|(entry, _)| entry)
        .collect();
    for entry in &stale {
        println!("  stale allow entry {entry:?}");
    }
    assert!(unread.is_empty(), "{} files not compared", unread.len());
    // A fixed bug's entry would otherwise keep hiding a later regression.
    assert!(
        stale.is_empty(),
        "{} allow entries no longer cover a pair",
        stale.len()
    );
    assert!(
        regressions.is_empty(),
        "{} properties where stylo loses a value jsdom gets right",
        regressions.len()
    );
}
