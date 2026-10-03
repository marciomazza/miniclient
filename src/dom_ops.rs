//! `op_dom_*`: the JS boundary of `src/dom.rs`. Kept out of `ops.rs` because `build.rs`
//! compiles this file too, to register the same ops in the snapshot, and `ops.rs` needs pyo3.

use deno_core::{Extension, OpState, op2};

use crate::dom::{AttrRecord, DomState, NodeRecord};

/// Registered right after `deno_web` by both the runtime and the snapshot build, so the
/// snapshot's extension list stays a prefix of the runtime's.
pub fn dom_extension() -> Extension {
    Extension {
        name: "miniclient_dom",
        ops: std::borrow::Cow::Owned(vec![
            op_dom_children(),
            op_dom_data(),
            op_dom_attrs(),
            op_dom_create(),
            op_dom_create_document(),
            op_dom_insert(),
            op_dom_remove(),
            op_dom_set_attr(),
            op_dom_remove_attr(),
            op_dom_set_data(),
            op_dom_serialize(),
            op_dom_next_sibling(),
            op_dom_prev_sibling(),
            op_dom_get_by_id(),
        ]),
        op_state_fn: Some(Box::new(|state| state.put(DomState::new()))),
        ..Default::default()
    }
}

fn dom(state: &mut OpState) -> &mut DomState {
    state.borrow_mut::<DomState>()
}

/// Empty string stands for JS `null`, so the string-taking ops can stay fast.
fn opt(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

/// `-1` stands for "no node", so the node-returning ops can stay fast.
fn wire_or_none(o: Option<u32>) -> i32 {
    o.map(|w| w as i32).unwrap_or(-1)
}

#[op2]
#[serde]
pub fn op_dom_children(state: &mut OpState, id: u32) -> Vec<NodeRecord> {
    dom(state).children(id)
}

#[op2]
#[string]
pub fn op_dom_data(state: &mut OpState, id: u32) -> Option<String> {
    state.borrow::<DomState>().data(id)
}

#[op2]
#[serde]
pub fn op_dom_attrs(state: &mut OpState, id: u32) -> Vec<AttrRecord> {
    state.borrow::<DomState>().attrs(id)
}

/// `name` carries the tag name for elements, the data for text/comment nodes.
#[op2(fast)]
pub fn op_dom_create(
    state: &mut OpState,
    node_type: u8,
    #[string] name: &str,
    #[string] ns: &str,
) -> u32 {
    dom(state).create(node_type, opt(name), opt(ns))
}

#[op2(fast)]
pub fn op_dom_create_document(state: &mut OpState, html: bool) -> u32 {
    dom(state).create_document(html)
}

/// `before` is the reference child's wire id, or `-1` to append last.
#[op2(fast)]
pub fn op_dom_insert(state: &mut OpState, parent: u32, child: u32, before: i32) {
    dom(state).insert(parent, child, u32::try_from(before).ok())
}

#[op2(fast)]
pub fn op_dom_remove(state: &mut OpState, id: u32) {
    dom(state).remove(id)
}

#[op2(fast)]
pub fn op_dom_set_attr(
    state: &mut OpState,
    id: u32,
    #[string] ns: &str,
    #[string] prefix: &str,
    #[string] local: &str,
    #[string] value: &str,
) {
    dom(state).set_attr(id, opt(ns), opt(prefix), local, value)
}

#[op2(fast)]
pub fn op_dom_remove_attr(state: &mut OpState, id: u32, #[string] ns: &str, #[string] local: &str) {
    dom(state).remove_attr(id, opt(ns), local)
}

#[op2(fast)]
pub fn op_dom_set_data(state: &mut OpState, id: u32, #[string] data: &str) {
    dom(state).set_data(id, data)
}

#[op2]
#[string]
pub fn op_dom_serialize(state: &mut OpState, id: u32) -> Option<String> {
    state.borrow::<DomState>().serialize(id)
}

#[op2(fast)]
pub fn op_dom_next_sibling(state: &mut OpState, id: u32) -> i32 {
    wire_or_none(dom(state).next_sibling(id))
}

#[op2(fast)]
pub fn op_dom_prev_sibling(state: &mut OpState, id: u32) -> i32 {
    wire_or_none(dom(state).prev_sibling(id))
}

/// `scope` is any node's wire id in the target document; the document root is resolved
/// from it.
#[op2(fast)]
pub fn op_dom_get_by_id(state: &mut OpState, scope: u32, #[string] id: &str) -> i32 {
    wire_or_none(dom(state).get_by_id(scope, id))
}
