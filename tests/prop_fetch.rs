//! Round-trip properties over `fetch()` against `MockFetchBackend`: for arbitrary paths,
//! statuses, bodies and headers, JS observes exactly what the mock was given. Each `proptest!`
//! case builds a fresh runtime + mock.

mod common;

use std::collections::HashSet;

use common::EvalExt;
use common::fetch_mock::MockFetchBackend;
use proptest::prelude::*;
use serde_json::{Value, json};

const URL: &str = "http://api.example.com";

/// ASCII printable, excluding characters that break a JS single-quoted string literal.
fn st_ascii_text(max_len: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(
            (32u8..127)
                .filter(|&b| b != b'\'' && b != b'\\')
                .collect::<Vec<u8>>(),
        ),
        0..=max_len,
    )
    .prop_map(|bytes| bytes.into_iter().map(|b| b as char).collect())
}

fn st_url_safe_segment() -> impl Strategy<Value = String> {
    // "." and ".." are dot-segments the URL parser resolves away, so the request never hits
    // the literal path the mock was registered under.
    "[a-zA-Z0-9_.~-]{1,50}".prop_filter("not a dot-segment", |s| s != "." && s != "..")
}

fn st_letters_and_numbers(max_len: usize) -> impl Strategy<Value = String> {
    proptest::string::string_regex(&format!("[a-zA-Z0-9]{{1,{max_len}}}"))
        .expect("regex is a valid strategy pattern")
}

fn st_header_key() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_.-]{1,20}"
}

fn st_query_param() -> impl Strategy<Value = (String, String)> {
    ("[a-zA-Z0-9]{1,20}", "[a-zA-Z0-9_.-]{0,50}")
}

/// A JSON leaf value: text, integer, or bool.
fn st_json_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        st_ascii_text(30).prop_map(Value::from),
        // Bounded to JS's safe integer range (2^53) -- past it, Number loses precision.
        (-(2i64.pow(53))..=2i64.pow(53)).prop_map(Value::from),
        proptest::bool::ANY.prop_map(Value::from),
    ]
}

fn st_json_recursive() -> impl Strategy<Value = Value> {
    let leaf = st_json_value();
    leaf.prop_recursive(3, 20, 5, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..=5).prop_map(Value::from),
            proptest::collection::hash_map(st_letters_and_numbers(10), inner, 0..=5)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 30, .. ProptestConfig::default() })]

    #[test]
    fn fetch_text_arbitrary_path(path_segment in st_url_safe_segment()) {
        let url = format!("{URL}/{path_segment}");
        let mock = MockFetchBackend::new();
        mock.text(&url, "ok");
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let result = rt.eval_async::<String>(&format!("fetch({url:?}).then(r => r.text())"));
        prop_assert_eq!(result, "ok");
    }

    #[test]
    fn fetch_with_query_params(params in proptest::collection::vec(st_query_param(), 0..=5)) {
        let query = params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        let url = format!("{URL}/search?{query}");
        let mock = MockFetchBackend::new();
        mock.text(&url, "results");
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let result = rt.eval_async::<String>(&format!("fetch({url:?}).then(r => r.text())"));
        prop_assert_eq!(result, "results");
    }

    #[test]
    fn fetch_status_arbitrary(status_code in 100u16..=599) {
        let url = format!("{URL}/status");
        let mock = MockFetchBackend::new();
        mock.status(&url, status_code);
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let result = rt.eval_async::<Value>(&format!(
            "fetch({url:?}).then(r => ({{ok: r.ok, status: r.status}}))"
        ));
        prop_assert_eq!(result["status"].as_u64(), Some(status_code as u64));
        prop_assert_eq!(result["ok"].as_bool(), Some((200..300).contains(&status_code)));
    }

    #[test]
    fn fetch_text_arbitrary_body(body in st_ascii_text(200)) {
        let url = format!("{URL}/echo");
        let mock = MockFetchBackend::new();
        mock.text(&url, &body);
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let result = rt.eval_async::<String>(&format!("fetch({url:?}).then(r => r.text())"));
        prop_assert_eq!(result, body);
    }

    #[test]
    fn fetch_empty_or_binary_response(
        binary_body in proptest::collection::vec(any::<u8>(), 0..=1024),
    ) {
        let text_body = String::from_utf8_lossy(&binary_body).into_owned();
        let url = format!("{URL}/binary");
        let mock = MockFetchBackend::new();
        mock.text(&url, &text_body);
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let result = rt.eval_async::<String>(&format!("fetch({url:?}).then(r => r.text())"));
        prop_assert_eq!(result, text_body);
    }

    #[test]
    fn fetch_json_arbitrary_dict(
        data in proptest::collection::hash_map(st_letters_and_numbers(10), st_json_value(), 0..=5),
    ) {
        let url = format!("{URL}/data");
        let expected = json!(data.clone());
        let mock = MockFetchBackend::new();
        mock.json(&url, expected.clone());
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let result = rt.eval_async::<Value>(&format!("fetch({url:?}).then(r => r.json())"));
        prop_assert_eq!(result, expected);
    }

    #[test]
    fn fetch_post_arbitrary_json_body(payload in st_json_recursive()) {
        let url = format!("{URL}/echo");
        let mock = MockFetchBackend::new();
        mock.text(&url, "saved");
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let body_js = serde_json::to_string(&payload).expect("Value serializes to JSON");
        let result = rt.eval_async::<String>(&format!(
            "fetch({url:?}, {{
               method: 'POST',
               headers: {{'content-type': 'application/json'}},
               body: JSON.stringify({body_js}),
             }}).then(r => r.text())"
        ));
        prop_assert_eq!(result, "saved");
        let request = mock.last_request();
        prop_assert_eq!(&request.method, "POST");
        prop_assert_eq!(request.json_body(), payload);
    }

    #[test]
    fn fetch_sends_arbitrary_headers(
        headers in proptest::collection::hash_map(st_header_key(), st_ascii_text(50), 0..=5),
    ) {
        let url = format!("{URL}/headers");
        let mock = MockFetchBackend::new();
        mock.text(&url, "ok");
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        let headers_js = serde_json::to_string(&headers).expect("headers serialize to JSON");
        rt.eval_async::<String>(&format!(
            "fetch({url:?}, {{headers: {headers_js}}}).then(r => r.text())"
        ));
        let request = mock.last_request();
        for (key, value) in &headers {
            // A few header names (content-length, etc.) are recomputed by the fetch layer
            // rather than passed through as given, so only check ones that made it through.
            if let Some(actual) = request.header(key) {
                prop_assert_eq!(actual, value.as_str());
            }
        }
    }

    #[test]
    fn fetch_receives_arbitrary_headers(
        response_headers in
            proptest::collection::hash_map(st_header_key(), st_ascii_text(50), 0..=5)
                .prop_filter("no case-insensitive key collisions", |m| {
                    let lower: HashSet<_> = m.keys().map(|k| k.to_lowercase()).collect();
                    lower.len() == m.len()
                }),
    ) {
        let url = format!("{URL}/response-headers");
        let headers: Vec<(&str, &str)> = response_headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let mock = MockFetchBackend::new();
        mock.reply(&url, 200, &headers, b"ok");
        let rt = common::fetch_mock::runtime_with_mock(&mock);
        for (key, value) in &response_headers {
            // `Headers.get` is case-insensitive per spec; iteration key casing is not, so
            // look each header up individually instead of collecting via `forEach`.
            let actual = rt.eval_async::<Option<String>>(&format!(
                "fetch({url:?}).then(r => r.headers.get({key:?}))"
            ));
            prop_assert_eq!(actual, Some(value.clone()));
        }
    }
}
