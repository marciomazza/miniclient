//! Keeps `python/miniclient/MPL_NOTICE.md` in step with `Cargo.lock`.
//! Regenerate with `just notice`.

#[derive(Debug, PartialEq)]
enum Verdict {
    Listed,
    Ignored,
}

/// Names of crates whose licence offers MPL-2.0 as one option and whose other option was
/// chosen. An entry is a deliberate decision, so it lives in the source where review sees it.
const ELECTED: &[(&str, &str)] = &[];

/// Whether MPL-2.0 binds the crate: always, never, or only if the licensee picks it.
#[derive(Clone, Copy, PartialEq)]
enum Binds {
    Always,
    Never,
    Choice,
}

fn and(a: Binds, b: Binds) -> Binds {
    match (a, b) {
        (Binds::Always, _) | (_, Binds::Always) => Binds::Always,
        (Binds::Choice, _) | (_, Binds::Choice) => Binds::Choice,
        _ => Binds::Never,
    }
}

fn or(a: Binds, b: Binds) -> Binds {
    if a == b { a } else { Binds::Choice }
}

fn leaf(id: &str) -> Result<Binds, String> {
    match id {
        "MPL-2.0" => Ok(Binds::Always),
        _ if id.to_ascii_uppercase().starts_with("MPL") => Err(format!(
            "unsupported MPL licence `{id}`; the notice carries MPL-2.0 only"
        )),
        _ => Ok(Binds::Never),
    }
}

type Tokens = std::iter::Peekable<std::vec::IntoIter<String>>;

/// Recursive descent over an SPDX expression: `or := and (OR and)*`, `and := atom (AND atom)*`.
fn parse_or(tokens: &mut Tokens) -> Result<Binds, String> {
    let mut acc = parse_and(tokens)?;
    while tokens.next_if(|t| t == "OR").is_some() {
        acc = or(acc, parse_and(tokens)?);
    }
    Ok(acc)
}

fn parse_and(tokens: &mut Tokens) -> Result<Binds, String> {
    let mut acc = parse_atom(tokens)?;
    while tokens.next_if(|t| t == "AND").is_some() {
        acc = and(acc, parse_atom(tokens)?);
    }
    Ok(acc)
}

fn parse_atom(tokens: &mut Tokens) -> Result<Binds, String> {
    match tokens.next().as_deref() {
        Some("(") => {
            let inner = parse_or(tokens)?;
            match tokens.next().as_deref() {
                Some(")") => Ok(inner),
                other => Err(format!("expected `)`, found {other:?}")),
            }
        }
        Some(")" | "AND" | "OR" | "WITH") | None => Err("expected a licence id".into()),
        Some(id) => {
            let binds = leaf(id)?;
            if tokens.next_if(|t| t == "WITH").is_some() {
                tokens.next().ok_or("expected an exception id after WITH")?;
            }
            Ok(binds)
        }
    }
}

/// Decides whether a crate belongs in the notice. Anything it cannot decide is an error, so a
/// crate never drops out silently.
fn classify(name: &str, license: Option<&str>) -> Result<Verdict, String> {
    let expr = license.ok_or_else(|| format!("{name}: no `license` field"))?;
    // Old manifests spell OR as `/`.
    let spaced = expr
        .replace('/', " OR ")
        .replace('(', " ( ")
        .replace(')', " ) ");
    let mut tokens = spaced
        .split_whitespace()
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .peekable();
    let binds = parse_or(&mut tokens).map_err(|e| format!("{name}: `{expr}`: {e}"))?;
    if let Some(extra) = tokens.next() {
        return Err(format!("{name}: `{expr}`: unexpected `{extra}`"));
    }
    match binds {
        Binds::Always => Ok(Verdict::Listed),
        Binds::Never => Ok(Verdict::Ignored),
        Binds::Choice if ELECTED.iter().any(|(n, _)| *n == name) => Ok(Verdict::Ignored),
        Binds::Choice => Err(format!(
            "{name}: `{expr}` offers MPL-2.0 as one option; decide which applies and add the \
             crate to ELECTED in tests/third_party_notice.rs"
        )),
    }
}

const NOTICE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/python/miniclient/MPL_NOTICE.md"
);
const MPL_TEXT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/licenses/MPL-2.0.txt");

/// Lists every registry crate in the lock graph, all features on: a feature-gated crate must
/// be covered before the feature is switched on.
fn render() -> Result<String, String> {
    let out = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--all-features",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let mut rows = vec![];
    let mut errors = vec![];
    for pkg in meta["packages"].as_array().ok_or("no packages")? {
        if pkg["source"].is_null() {
            continue;
        }
        let (name, version) = (
            pkg["name"].as_str().unwrap(),
            pkg["version"].as_str().unwrap(),
        );
        match classify(name, pkg["license"].as_str()) {
            Ok(Verdict::Listed) => rows.push(format!(
                "- `{name}` {version} — <https://crates.io/crates/{name}/{version}>, source: {}",
                pkg["repository"].as_str().unwrap_or("not declared")
            )),
            Ok(Verdict::Ignored) => {}
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    rows.sort();
    // The repo's whitespace hook trims lines, so the generated file must already be trimmed.
    let text = std::fs::read_to_string(MPL_TEXT).map_err(|e| e.to_string())?;
    let text: String = text
        .lines()
        .map(|l| format!("{}\n", l.trim_end()))
        .collect();
    Ok(format!(
        "# MPL-2.0 components\n\nThis package links the following crates, which are licensed under the Mozilla\n\
         Public License 2.0 and are distributed unmodified. Their source is available at the\n\
         addresses below.\n\n{}\n\n## Mozilla Public License 2.0\n\n```text\n{text}```\n",
        rows.join("\n")
    ))
}

#[test]
fn notice_lists_every_mpl_crate_in_the_lock() {
    let want = render().unwrap_or_else(|e| panic!("{e}"));
    if std::env::var_os("UPDATE_NOTICE").is_some() {
        std::fs::write(NOTICE, &want).unwrap();
    }
    let have = std::fs::read_to_string(NOTICE).unwrap_or_default();
    assert!(have == want, "MPL_NOTICE.md is stale; run `just notice`");
}

#[cfg(test)]
mod classify_tests {
    use super::*;

    fn verdict(license: &str) -> Result<Verdict, String> {
        classify("krate", Some(license))
    }

    #[test]
    fn sole_mpl_is_listed() {
        assert_eq!(verdict("MPL-2.0"), Ok(Verdict::Listed));
    }

    #[test]
    fn mpl_and_another_licence_is_listed() {
        assert_eq!(verdict("MIT AND MPL-2.0"), Ok(Verdict::Listed));
        assert_eq!(
            verdict("MPL-2.0 AND (MIT OR Apache-2.0)"),
            Ok(Verdict::Listed)
        );
    }

    #[test]
    fn licences_without_mpl_are_ignored() {
        assert_eq!(verdict("MIT/Apache-2.0"), Ok(Verdict::Ignored));
        assert_eq!(
            verdict("(MIT OR Apache-2.0) AND Unicode-3.0"),
            Ok(Verdict::Ignored)
        );
    }

    #[test]
    fn mpl_as_one_option_fails_until_elected() {
        for expr in [
            "MIT OR MPL-2.0",
            "MPL-2.0/MIT",
            "(MPL-2.0 OR MIT) AND BSD-3-Clause",
        ] {
            let err = verdict(expr).unwrap_err();
            assert!(
                err.contains("krate") && err.contains("ELECTED"),
                "{expr}: {err}"
            );
        }
    }

    #[test]
    fn mpl_chosen_in_both_options_is_listed() {
        assert_eq!(verdict("MPL-2.0 OR MPL-2.0"), Ok(Verdict::Listed));
    }

    #[test]
    fn unreadable_licences_fail() {
        assert!(classify("krate", None).is_err());
        assert!(verdict("").is_err());
        assert!(verdict("MIT OR").is_err());
        assert!(verdict("(MIT").is_err());
    }

    #[test]
    fn other_mpl_versions_fail() {
        assert!(verdict("MPL-1.1").is_err());
    }
}
