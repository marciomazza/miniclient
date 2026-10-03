//! `textContent` reads of elements and fragments come from the arena in one call.

mod common;

use common::{EvalExt, runtime};

#[test]
fn text_content_has_the_dom_values() {
    let rt = runtime();
    let got: Vec<String> = rt.eval(
        r#"const div = document.body.appendChild(document.createElement("div"));
        div.innerHTML = "a<!--c--><script>s</script><style>t</style><template>no</template>b";
        [div.textContent, div.firstChild.textContent, div.childNodes[1].textContent,
         document.createDocumentFragment().textContent]"#,
    );
    assert_eq!(got, ["astb", "a", "c", ""]);
}

#[test]
fn text_content_of_built_detached_and_shadow_trees() {
    let rt = runtime();
    let got: Vec<String> = rt.eval(
        r#"const div = document.createElement("div");
        const b = div.appendChild(document.createElement("b"));
        b.append("x", document.createComment("c"), "y");
        div.append("z");
        const before = div.textContent;
        b.firstChild.data = "X";
        const host = document.body.appendChild(document.createElement("p"));
        host.textContent = "light";
        host.attachShadow({ mode: "open" }).innerHTML = "<i>shadow</i>";
        const xml = new DOMParser().parseFromString("<r>a<![CDATA[b]]>c</r>", "application/xml");
        [before, div.textContent, host.textContent, host.shadowRoot.textContent,
         xml.documentElement.textContent]"#,
    );
    assert_eq!(got, ["xyz", "Xyz", "light", "shadow", "abc"]);
}

#[test]
fn text_content_writes_round_trip() {
    let rt = runtime();
    let got: Vec<String> = rt.eval(
        r#"const div = document.body.appendChild(document.createElement("div"));
        div.innerHTML = "<p>old</p>";
        div.textContent = "<b>new</b>";
        const first = [div.textContent, div.innerHTML, String(div.childNodes.length)];
        div.textContent = "";
        first.concat([div.textContent, String(div.childNodes.length)])"#,
    );
    assert_eq!(got, ["<b>new</b>", "&lt;b&gt;new&lt;/b&gt;", "1", "", "0"]);
}
