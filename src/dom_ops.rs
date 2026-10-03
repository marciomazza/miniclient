//! `op_dom_*`: the JS boundary of `src/dom.rs`. Kept out of `ops.rs` because `build.rs`
//! compiles this file too, to register the same ops in the snapshot, and `ops.rs` needs pyo3.

use deno_core::{Extension, OpState, op2};

use crate::dom::{AttrRecord, DomState, NodeRecord};
use crate::dom_query::QueryMode;

/// Registered right after `deno_web` by both the runtime and the snapshot build, so the
/// snapshot's extension list stays a prefix of the runtime's.
pub fn dom_extension() -> Extension {
    Extension {
        name: "miniclient_dom",
        ops: std::borrow::Cow::Owned(vec![
            op_dom_children(),
            op_dom_data(),
            op_dom_attrs(),
            op_dom_attr_value(),
            op_dom_attr(),
            op_dom_attr_by_name(),
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
            op_dom_parent_id(),
            op_dom_first_child(),
            op_dom_last_child(),
            op_dom_child_count(),
            op_dom_index(),
            op_dom_following(),
            op_dom_preceding(),
            op_dom_child_ids(),
            op_dom_tree_ids(),
            op_dom_query(),
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

/// Batched reads fill a caller-owned `Uint32Array` instead of returning a JS array, so they
/// stay fast ops. The return value is the full length: when it exceeds `out`, nothing usable
/// was written and the caller retries with a bigger buffer.
fn fill(out: &mut [u32], wires: Vec<u32>) -> u32 {
    if let Some(dst) = out.get_mut(..wires.len()) {
        dst.copy_from_slice(&wires);
    }
    wires.len() as u32
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

/// One attribute's value by namespace (empty for none) and local name, without listing them
/// all.
#[op2]
#[string]
pub fn op_dom_attr_value(
    state: &mut OpState,
    id: u32,
    #[string] ns: &str,
    #[string] local: &str,
) -> Option<String> {
    state.borrow::<DomState>().attr_value(id, opt(ns), local)
}

/// One attribute by namespace (empty for none) and local name.
#[op2]
#[serde]
pub fn op_dom_attr(
    state: &mut OpState,
    id: u32,
    #[string] ns: &str,
    #[string] local: &str,
) -> Option<AttrRecord> {
    state.borrow::<DomState>().attr(id, opt(ns), local)
}

/// The first attribute whose qualified name is `name`.
#[op2]
#[serde]
pub fn op_dom_attr_by_name(
    state: &mut OpState,
    id: u32,
    #[string] name: &str,
) -> Option<AttrRecord> {
    state.borrow::<DomState>().attr_by_name(id, name)
}

/// `name` carries the local name for elements, the data for text/comment nodes, the PI
/// target or the doctype name.
#[op2(fast)]
pub fn op_dom_create(
    state: &mut OpState,
    node_type: u8,
    #[string] name: &str,
    #[string] ns: &str,
    #[string] prefix: &str,
) -> u32 {
    dom(state).create(node_type, opt(name), opt(ns), opt(prefix))
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

#[op2(fast)]
pub fn op_dom_parent_id(state: &mut OpState, id: u32) -> i32 {
    wire_or_none(dom(state).parent_wire(id))
}

#[op2(fast)]
pub fn op_dom_first_child(state: &mut OpState, id: u32) -> i32 {
    wire_or_none(dom(state).first_child(id))
}

#[op2(fast)]
pub fn op_dom_last_child(state: &mut OpState, id: u32) -> i32 {
    wire_or_none(dom(state).last_child(id))
}

#[op2(fast)]
pub fn op_dom_child_count(state: &mut OpState, id: u32) -> u32 {
    state.borrow::<DomState>().child_count(id)
}

#[op2(fast)]
pub fn op_dom_index(state: &mut OpState, id: u32) -> i32 {
    wire_or_none(state.borrow::<DomState>().index(id))
}

/// `root` is `-1` for an unbounded walk.
#[op2(fast)]
pub fn op_dom_following(state: &mut OpState, id: u32, root: i32, skip_children: bool) -> i32 {
    wire_or_none(dom(state).following(id, u32::try_from(root).ok(), skip_children))
}

#[op2(fast)]
pub fn op_dom_preceding(state: &mut OpState, id: u32, root: i32) -> i32 {
    wire_or_none(dom(state).preceding(id, u32::try_from(root).ok()))
}

#[op2(fast)]
pub fn op_dom_child_ids(state: &mut OpState, id: u32, #[buffer] out: &mut [u32]) -> u32 {
    fill(out, dom(state).child_wires(id))
}

#[op2(fast)]
pub fn op_dom_tree_ids(state: &mut OpState, id: u32, #[buffer] out: &mut [u32]) -> u32 {
    fill(out, dom(state).tree_wires(id))
}

/// `mode` is a `QueryMode` index (first, all, closest, matches); `focus` and `target` are the
/// focused and target elements' wires, `-1` for none. Returns `-1` when the selector does not
/// parse natively, so JS falls back to `dom-selector`.
#[op2(fast)]
pub fn op_dom_query(
    state: &mut OpState,
    scope: u32,
    #[string] selector: &str,
    mode: u8,
    html: bool,
    quirks: bool,
    focus: i32,
    target: i32,
    #[buffer] out: &mut [u32],
) -> i32 {
    let Some(mode) = QueryMode::from_u8(mode) else {
        return -1;
    };
    let dom = dom(state);
    let query_state = dom.query_state(html, quirks, focus, target);
    match dom.query(scope, selector, mode, &query_state) {
        Some(wires) => fill(out, wires) as i32,
        None => -1,
    }
}
