mod common;

use common::{EvalExt, runtime};

#[test]
fn inner_text_works_for_domparser_fragments() {
    let rt = runtime();
    assert_eq!(
        rt.eval::<Vec<String>>(
            r#"(() => {
                const doc = new DOMParser().parseFromString(
                    '<template><div>Test</div><table><tr><td>Cell</td></tr></table></template>',
                    'text/html',
                );
                const content = doc.querySelector('template').content;
                return [
                    content.querySelector('div').innerText,
                    content.querySelector('td').innerText,
                ];
            })()"#,
        ),
        ["Test", "Cell"],
    );
}
