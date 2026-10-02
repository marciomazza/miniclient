//! `<script src>`/`<link rel=stylesheet>` real-network resource fetching under jsdom, routed
//! through its `resources`/`requestInterceptor` option to `browser::Page`'s own
//! `FetchBackend` -- driven by `MockFetchBackend`, no pyo3/Python involved.
//!
//! All requests target the page's own origin (`http://localhost/`) rather than a cross-origin
//! URL: `<script>` fetches run in CORS mode by spec and would otherwise be blocked by jsdom's
//! Same-Origin Policy check -- a real, correct check living above our patch boundary, not
//! something this suite is testing.

mod common;

use std::sync::Arc;

use _miniclient::browser::Page;
use common::aeval_async;
use common::fetch_mock::MockFetchBackend;

async fn page_with_mock(mock: &MockFetchBackend) -> Page {
    let page = Page::new("http://localhost/", "{}");
    page.install_fetch_backend(Arc::new(mock.clone())).await;
    page
}

async fn assert_script_src_loads(attr_setup_js: &str) {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/x.js", "window.__ran = 1;");
    let page = page_with_mock(&mock).await;
    let ran: i64 = aeval_async(
        page.runtime(),
        &format!(
            r#"
            new Promise((resolve, reject) => {{
              const script = document.createElement('script');
              {attr_setup_js};
              script.src = 'http://localhost/x.js';
              script.onload = () => resolve(window.__ran);
              script.onerror = () => reject(new Error('script failed to load'));
              document.head.appendChild(script);
            }});
            "#
        ),
    )
    .await;
    assert_eq!(ran, 1, "{attr_setup_js}");
}

#[tokio::test]
async fn async_script_src_real_network_executes() {
    assert_script_src_loads("script.async = true;").await;
}

#[tokio::test]
async fn defer_script_src_real_network_executes() {
    assert_script_src_loads("script.defer = true;").await;
}

#[tokio::test]
async fn module_script_src_real_network_executes() {
    assert_script_src_loads("script.type = 'module';").await;
}

#[tokio::test]
async fn classic_script_src_real_network_executes() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/classic.js", "window.__ran = 1;");
    let page = page_with_mock(&mock).await;
    let ran: i64 = aeval_async(
        page.runtime(),
        r#"
        new Promise((resolve, reject) => {
          const script = document.createElement('script');
          script.src = 'http://localhost/classic.js';
          script.onload = () => resolve(window.__ran);
          script.onerror = () => reject(new Error('script failed to load'));
          document.head.appendChild(script);
        });
        "#,
    )
    .await;
    assert_eq!(ran, 1);
}

async fn assert_script_src_errors_on_404(script_setup_js: &str) {
    let mock = MockFetchBackend::new();
    mock.status("http://localhost/missing.js", 404);
    let page = page_with_mock(&mock).await;
    let outcome: String = aeval_async(
        page.runtime(),
        &format!(
            r#"
            new Promise(resolve => {{
              const script = document.createElement('script');
              {script_setup_js}
              script.src = 'http://localhost/missing.js';
              script.onload = () => resolve('loaded');
              script.onerror = () => resolve('errored');
              document.head.appendChild(script);
            }});
            "#
        ),
    )
    .await;
    assert_eq!(outcome, "errored");
}

#[tokio::test]
async fn script_src_non_ok_status_fires_error_not_exception() {
    assert_script_src_errors_on_404("script.async = true;").await;
}

#[tokio::test]
async fn classic_script_src_non_ok_status_fires_error_not_exception() {
    assert_script_src_errors_on_404("").await;
}

async fn assert_script_src_follows_redirect(script_setup_js: &str) {
    let mock = MockFetchBackend::new();
    mock.redirect("http://localhost/old.js", "http://localhost/new.js");
    mock.text("http://localhost/new.js", "window.__ran = 'redirected';");
    let page = page_with_mock(&mock).await;
    let ran: String = aeval_async(
        page.runtime(),
        &format!(
            r#"
            new Promise((resolve, reject) => {{
              const script = document.createElement('script');
              {script_setup_js}
              script.src = 'http://localhost/old.js';
              script.onload = () => resolve(window.__ran);
              script.onerror = () => reject(new Error('script failed to load'));
              document.head.appendChild(script);
            }});
            "#
        ),
    )
    .await;
    assert_eq!(ran, "redirected");
}

#[tokio::test]
async fn script_src_follows_redirect() {
    assert_script_src_follows_redirect("script.async = true;").await;
}

#[tokio::test]
async fn classic_script_src_follows_redirect() {
    assert_script_src_follows_redirect("").await;
}

#[tokio::test]
async fn link_stylesheet_real_network_loads() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/x.css", "body { color: red; }");
    let page = page_with_mock(&mock).await;
    let outcome: String = aeval_async(
        page.runtime(),
        r#"
        new Promise((resolve, reject) => {
          const link = document.createElement('link');
          link.rel = 'stylesheet';
          link.href = 'http://localhost/x.css';
          link.onload = () => resolve('loaded');
          link.onerror = () => reject(new Error('link failed to load'));
          document.head.appendChild(link);
        });
        "#,
    )
    .await;
    assert_eq!(outcome, "loaded");
}

/// No route registered for the stylesheet -- the mock errors the fetch exactly as an
/// unreachable host would on a real network.
#[tokio::test]
async fn unmocked_stylesheet_fires_error_not_exception() {
    let mock = MockFetchBackend::new();
    let page = page_with_mock(&mock).await;
    let outcome: String = aeval_async(
        page.runtime(),
        r#"
        new Promise(resolve => {
          const link = document.createElement('link');
          link.rel = 'stylesheet';
          link.href = 'http://localhost/unmocked.css';
          link.onload = () => resolve('loaded');
          link.onerror = () => resolve('errored');
          document.head.appendChild(link);
        });
        "#,
    )
    .await;
    assert_eq!(outcome, "errored");
}
