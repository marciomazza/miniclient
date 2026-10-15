//! `getComputedStyle`, answered by stylo, on hand-written pages and against a Chromium recording
//! of the CSS corpus. The cost test is a report: run it with `-- --ignored --nocapture`.

mod common;

use std::collections::BTreeMap;

use common::EvalExt;
use common::fetch_mock::{MockFetchBackend, runtime_with_mock};
use serde::Deserialize;

const HARNESS: &str = r#"
globalThis.__computed = (el, props) => {
    const cs = getComputedStyle(el);
    return props.map((p) => cs.getPropertyValue(p));
};
0
"#;

fn runtime() -> common::Runtime {
    let rt = runtime_with_mock(&MockFetchBackend::new());
    rt.run(HARNESS);
    rt
}

/// `getComputedStyle(document.querySelector(selector))` for each of `props`, after `markup`
/// replaces the document's content.
fn computed(rt: &common::Runtime, markup: &str, selector: &str, props: &[&str]) -> Vec<String> {
    let markup = serde_json::to_string(markup).unwrap();
    let args = serde_json::to_string(&(selector, props)).unwrap();
    rt.eval::<Vec<String>>(&format!(
        "document.documentElement.innerHTML = {markup}; \
         ((sel, props) => __computed(document.querySelector(sel), props))(...{args})"
    ))
}

const BASICS: &str = r#"<head><style>
        :root { --gap: 4px }
        .a { color: rgb(0, 0, 255); margin-top: var(--gap) }
        #b.a { color: green }
        p { display: inline-block !important }
        .c { & span { visibility: hidden } }
        @media (min-width: 600px) { .a { padding-left: 3px } }
        input:checked { outline-style: solid }
    </style></head>
    <body><div class="a"><p id="b" class="a" style="display: block">x</p>
    <div class="c"><span>y</span></div><input type="checkbox" checked></div></body>"#;

#[test]
fn cascade_basics() {
    let rt = runtime();
    let p = [
        "color",
        "display",
        "margin-top",
        "padding-left",
        "margin-block",
    ];
    assert_eq!(
        computed(&rt, BASICS, "#b", &p),
        ["rgb(0, 128, 0)", "inline-block", "4px", "3px", "4px 16px"]
    );
    assert_eq!(computed(&rt, BASICS, "span", &["visibility"]), ["hidden"]);
    assert_eq!(
        computed(&rt, BASICS, "input", &["outline-style"]),
        ["solid"]
    );
    // camelCase and `webkit*` accessors read the same values
    let camel: Vec<String> = rt.eval(
        r##"const cs = getComputedStyle(document.querySelector("#b"));
        [cs.marginTop, cs.webkitTransform, cs.getPropertyValue("-webkit-transform")]"##,
    );
    assert_eq!(camel, ["4px", "none", "none"]);
}

#[test]
fn reflects_mutations() {
    let rt = runtime();
    let markup = "<head><style>.on { color: rgb(1, 2, 3) }</style></head><body><p>x</p></body>";
    assert_eq!(computed(&rt, markup, "p", &["color"]), ["rgb(0, 0, 0)"]);
    let color: String = rt.eval(
        r#"const p = document.querySelector("p"); p.className = "on"; getComputedStyle(p).color"#,
    );
    assert_eq!(color, "rgb(1, 2, 3)");
}

#[test]
fn hidden_ancestor_is_not_focusable() {
    let rt = runtime();
    let focused: Vec<bool> = rt.eval(
        r#"document.body.innerHTML =
            "<style>.off { display: none }</style><div class=off><button>a</button></div><button>b</button>";
        [...document.querySelectorAll("button")].map((b) => (b.focus(), document.activeElement === b))"#,
    );
    assert_eq!(focused, [false, true]);
}

#[test]
fn inner_text_of_a_hidden_element_is_empty() {
    let rt = runtime();
    let text: Vec<String> = rt.eval(
        r#"document.body.innerHTML = "<style>.off { display: none }</style><p class=off>a</p><p>b</p>";
        [...document.querySelectorAll("p")].map((p) => p.innerText)"#,
    );
    assert_eq!(text, ["", "b"]);
}

#[test]
fn outside_a_document_reads_empty() {
    let rt = runtime();
    let color: String = rt.eval(r#"getComputedStyle(document.createElement("p")).color"#);
    assert_eq!(color, "");
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

/// Run with `cargo test --release --test stylo_css cost -- --ignored --nocapture`.
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
    rt.run(r#"r.first = time(() => getComputedStyle(el).color); 0"#);
    let rss = rss_kb().saturating_sub(before);
    let json: String = rt.eval(
        r#"(() => {
            r.warm = avg(200, () => time(() => getComputedStyle(el).color));
            r.other = avg(200, (i) => time(() => getComputedStyle(els[i]).color));
            r.afterMutation = avg(20, (i) => {
                els[i].setAttribute("data-x", i);
                return time(() => getComputedStyle(el).color);
            });
            return JSON.stringify(r);
        })()"#,
    );
    println!("elements 1001, rules 100; times in ms: {json}");
    // jsdom's cascade on the same page; it no longer runs, so these numbers are fixed.
    println!(
        r#"jsdom's cascade was:            {{"first":7.81,"warm":0.0051,"afterMutation":0.750}}"#
    );
    println!("rss growth over the first call: +{rss} KiB");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    chromium: String,
    viewport: (u32, u32),
    color_scheme: String,
    properties: Vec<String>,
    defaults: BTreeMap<String, String>,
    files: BTreeMap<String, Vec<RecordedElement>>,
}

/// An element's local name and the properties where Chromium's value differs from `defaults`.
type RecordedElement = (String, BTreeMap<String, String>);

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
                [...document.querySelectorAll("*")].map((el) => [el.localName, __computed(el, props)])
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
