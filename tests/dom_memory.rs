//! Impls are held weakly by wire, so replaced subtrees are collected while a held node keeps
//! its whole detached tree reachable.

mod common;

use common::{EvalExt, runtime};

#[test]
fn swapped_out_subtrees_are_collected_and_held_ones_survive() {
    // V8 reads its flags once per process, and this binary holds only this test.
    unsafe {
        std::env::set_var(
            "MINICLIENT_V8_FLAGS",
            "--no-opt --min-semi-space-size=16 --initial-old-space-size=128 --expose-gc",
        )
    };
    let rt = runtime();
    rt.run(
        r#"globalThis.box = document.body.appendChild(document.createElement("div"));
        const items = Array.from({ length: 100 }, (_, i) => `<li id=i${i}>${i}</li>`);
        globalThis.markup = `<ul>${items.join("")}</ul>`;
        box.innerHTML = markup;
        globalThis.held = box.firstChild.children[50]; 0"#,
    );
    for _ in 0..20 {
        rt.run("for (let i = 0; i < 100; i++) { box.innerHTML = markup; }");
        // Kept WeakRef targets are only released at a microtask checkpoint.
        let _: f64 = rt.eval_promise("Promise.resolve(0)");
    }
    let got: Vec<String> = rt.eval(
        r##"gc();
        [String(Deno.core.memoryUsage().heapUsed < 80 * 1048576),
         held.textContent, String(held.parentNode.childNodes.length),
         held.previousElementSibling.id, held.nextSibling.textContent,
         held.parentNode.querySelector("#i7").textContent]"##,
    );
    assert_eq!(got, ["true", "50", "100", "i49", "51", "7"]);
}
