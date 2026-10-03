//! Rust-backed DOM tree: the node arena held in `OpState`, answering the `op_dom_*` ops
//! (`src/dom_ops.rs`). Structure, attributes and character data live here.

use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet};

use ego_tree::{NodeId, NodeRef, Tree};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tokenizer::{BufferQueue, Tokenizer, TokenizerOpts};
use html5ever::tree_builder::{
    ElementFlags, NodeOrText, QuirksMode, TreeBuilder, TreeBuilderOpts, TreeSink,
};
use html5ever::{
    Attribute, LocalName, Namespace, Prefix, QualName, TokenizerResult, local_name, ns,
};
use serde::Serialize;

use crate::dom_query::{self, HTML_NS, MatchSelector, QueryMode, QueryState};
use crate::node::{Element, NO_WIRE, Node};

/// Bounds the parsed-selector cache, which pages that build selectors from data would grow
/// without limit.
const SELECTOR_CACHE_SIZE: usize = 256;

/// Minimal node record crossing the op boundary. No attributes, no text data, no structure --
/// those are fetched lazily (`op_dom_data`, `op_dom_attrs`), so a record JS caches stays
/// correct after any mutation.
#[derive(Serialize)]
pub struct NodeRecord {
    /// Wire id -- an index into `DomState::ids`, not the raw `ego_tree::NodeId` (its index
    /// accessor is private).
    pub id: u32,
    #[serde(rename = "type")]
    pub node_type: u8,
    pub name: Option<String>,
    pub ns: Option<String>,
}

/// One attribute with its full qualified name. `ns` and `prefix` are `None` for the common
/// no-namespace attribute.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AttrRecord {
    pub ns: Option<String>,
    pub prefix: Option<String>,
    pub local: String,
    pub value: String,
}

/// One node of a subtree JS creates impls for: what jsdom's constructors need. `name` is an
/// element's local name or a doctype's name.
#[derive(Serialize)]
pub struct TreeRecord {
    pub id: u32,
    #[serde(rename = "type")]
    pub node_type: u8,
    pub name: String,
    pub ns: String,
    /// A `<template>`'s content fragment.
    pub contents: Option<u32>,
}

/// A node the document parser added since its last pause, detached again so that JS inserts it
/// itself, childless, before `before` under `parent`.
#[derive(Serialize)]
pub struct ParsedRecord {
    pub node: TreeRecord,
    pub parent: u32,
    /// The first sibling that was already there, or `None` to append.
    pub before: Option<u32>,
    /// Popped off the stack of open elements, so JS runs its end-of-element steps. html5ever
    /// does not report every pop, but it does for `<script>` and `<style>`, the only elements
    /// with such steps.
    pub popped: bool,
    pub doctype: Option<DoctypeIds>,
}

#[derive(Serialize)]
pub struct DoctypeIds {
    pub public_id: String,
    pub system_id: String,
}

#[derive(Serialize)]
pub struct ParseStep {
    pub records: Vec<ParsedRecord>,
    /// Attributes the parser added to elements JS already has (a later `<html>` or `<body>`
    /// tag), left for JS to set.
    pub attrs: Vec<(u32, AttrRecord)>,
    /// Text the parser appended to text nodes JS already has, left for JS to append.
    pub text: Vec<(u32, String)>,
    pub done: bool,
    /// The document's mode, reported once, in jsdom's terms.
    pub quirks: Option<&'static str>,
}

/// One `ego-tree` arena per isolate, held in `OpState`. Every parsed document and detached
/// fragment shares this single id space. Detached nodes keep their slot: the runtime lives for
/// one navigation only, so nothing needs reclaiming.
pub struct DomState {
    pub arena: Tree<Node>,
    // ponytail: side table maps a small wire integer to a NodeId, because ego-tree keeps its
    // own index accessor private. Drop it if ego-tree ever exposes `from_index`.
    ids: Vec<NodeId>,
    /// Bumps on every structural change (parse / create / insert / remove).
    pub mutations: u32,
    /// `id` attribute value -> every element bearing it, connected or not. `getElementById`
    /// filters this by document (each candidate's topmost ancestor) at read time instead of
    /// tracking connect/disconnect -- that's exactly what "topmost ancestor" already answers.
    id_index: HashMap<String, Vec<NodeId>>,
    /// Parsed selectors by source text; `None` caches a selector that does not parse natively.
    selectors: HashMap<String, Option<MatchSelector>>,
}

impl DomState {
    pub fn new() -> Self {
        Self {
            arena: Tree::new(Node::document(false)),
            ids: Vec::new(),
            mutations: 0,
            id_index: HashMap::new(),
            selectors: HashMap::new(),
        }
    }

    /// Read access to the arena for the native selector walk.
    pub fn arena(&self) -> &Tree<Node> {
        &self.arena
    }

    /// The wire ids `scope` selects in `mode` (see `dom_query::query`); `None` when the
    /// selector does not parse natively.
    pub fn query(
        &mut self,
        scope: u32,
        selector: &str,
        mode: QueryMode,
        state: &QueryState,
    ) -> Option<Vec<u32>> {
        self.parsed(selector)?;
        let quirks = state.quirks;
        let parsed = self.selectors.get(selector)?.as_ref()?;
        let Some(node) = self.resolve(scope).and_then(|id| self.arena.get(id)) else {
            return Some(Vec::new());
        };
        let bearers = match mode {
            QueryMode::First | QueryMode::All => {
                parsed.subject_id().map(|id| self.id_bearers(id, quirks))
            }
            _ => None,
        };
        let hits = match bearers.as_deref() {
            // Several bearers need the walk to come out in document order; duplicate ids are rare.
            None | Some([_, _, ..]) => dom_query::query(node, parsed, mode, state),
            Some(bearers) => {
                let inside = bearers
                    .iter()
                    .filter_map(|&b| self.arena.get(b))
                    .filter(|b| b.ancestors().any(|a| a.id() == node.id()));
                dom_query::select(node, inside, parsed, mode, state)
            }
        };
        Some(hits.into_iter().map(|id| self.wire(id)).collect())
    }

    fn parsed(&mut self, selector: &str) -> Option<&MatchSelector> {
        if !self.selectors.contains_key(selector) {
            // ponytail: dropping the whole cache when full beats LRU bookkeeping on every hit.
            if self.selectors.len() >= SELECTOR_CACHE_SIZE {
                self.selectors.clear();
            }
            self.selectors
                .insert(selector.to_owned(), MatchSelector::parse(selector));
        }
        self.selectors.get(selector)?.as_ref()
    }

    /// `focus` and `target` are wires, negative for none.
    pub fn query_state(&self, html: bool, quirks: bool, focus: i32, target: i32) -> QueryState {
        let node = |w: i32| u32::try_from(w).ok().and_then(|w| self.resolve(w));
        let focus_path = node(focus)
            .and_then(|id| self.arena.get(id))
            .map(|n| {
                std::iter::once(n)
                    .chain(n.ancestors())
                    .map(|a| a.id())
                    .collect()
            })
            .unwrap_or_default();
        QueryState {
            html,
            quirks,
            focus_path,
            target: node(target),
        }
    }

    /// Elements whose `id` is `id`, connected or not; ASCII case-insensitively in quirks mode,
    /// as `#id` matches there.
    fn id_bearers(&self, id: &str, quirks: bool) -> Vec<NodeId> {
        if !quirks {
            return self.id_index.get(id).cloned().unwrap_or_default();
        }
        self.id_index
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(id))
            .flat_map(|(_, nodes)| nodes.iter().copied())
            .collect()
    }

    /// Parses `html` and folds it into the arena as an orphan subtree. Returns the roots only
    /// (the document or fragment root); descendants come via `children` on demand. `context`
    /// is the fragment context element's tag name (`""` falls back to `<body>`); `None` parses
    /// a full document.
    #[cfg(test)]
    pub fn parse(&mut self, html: &str, context: Option<&str>) -> Vec<NodeRecord> {
        let root = match context {
            None => self.parse_document(html),
            Some(ctx) => {
                let ctx = if ctx.is_empty() { "body" } else { ctx };
                let context = self.create(1, Some(ctx), Some("html"), None);
                let wire = self.parse_fragment(html, context, None, true);
                self.resolve(wire).expect("fresh fragment")
            }
        };
        self.record(root).into_iter().collect()
    }

    /// The fragment parsing algorithm: `html` parsed in the context of the element `context`,
    /// into a detached fragment whose wire id is returned. `form` is the nearest `<form>`
    /// among the context's inclusive ancestors, which makes a nested `<form>` tag a no-op;
    /// `scripting` is the document's flag that keeps `<noscript>` content raw. The parser reads
    /// the context's name and namespace, so `<td>` / `<tr>` fragments land under the right
    /// insertion mode and SVG contexts parse as foreign content.
    pub fn parse_fragment(
        &mut self,
        html: &str,
        context: u32,
        form: Option<u32>,
        scripting: bool,
    ) -> u32 {
        let context = self.resolve(context).expect("fragment context");
        let form = form.and_then(|f| self.resolve(f));
        let root = self.arena.orphan(Node::fragment()).id();
        let sink = ArenaSink::new(root);
        std::mem::swap(self, &mut sink.dom.borrow_mut());
        let opts = html5ever::driver::ParseOpts {
            tree_builder: TreeBuilderOpts {
                scripting_enabled: scripting,
                ..Default::default()
            },
            ..Default::default()
        };
        *self = html5ever::driver::parse_fragment_for_element(sink, opts, context, scripting, form)
            .one(html);
        self.mutations += 1;
        // The tree builder puts the content in an `<html>` element under the fragment root.
        let wrapper = self
            .arena
            .get(root)
            .and_then(|r| r.first_child())
            .map(|w| w.id());
        if let Some(wrapper) = wrapper {
            let mut root_mut = self.arena.get_mut(root).expect("fresh fragment");
            root_mut.reparent_from_id_append(wrapper);
            self.arena.get_mut(wrapper).expect("fresh wrapper").detach();
        }
        self.wire(root)
    }

    /// Parses a whole document into a fresh, detached `Document` node.
    #[cfg(test)]
    fn parse_document(&mut self, html: &str) -> NodeId {
        let root = self.arena.orphan(Node::document(false)).id();
        let wire = self.wire(root);
        let parser = DocumentParser::new(self, wire, html, true, false).expect("fresh document");
        parser.feed(self);
        self.mutations += 1;
        root
    }

    /// Walks a freshly parsed subtree once, indexing every element's `id` attribute.
    fn index_ids_in_subtree(&mut self, root: NodeId) {
        let found: Vec<(String, NodeId)> = match self.arena.get(root) {
            Some(node) => node
                .descendants()
                .filter_map(|n| match n.value() {
                    Node::Element(e) => id_of(e).map(|v| (v, n.id())),
                    _ => None,
                })
                .collect(),
            None => return,
        };
        for (id, node) in found {
            self.id_index.entry(id).or_default().push(node);
        }
    }

    /// Adds/removes `node` from the `id` index. A no-op when the value didn't actually change.
    fn reindex_id(&mut self, node: NodeId, old: Option<&str>, new: Option<&str>) {
        if old == new {
            return;
        }
        if let Some(old) = old
            && let Some(v) = self.id_index.get_mut(old)
        {
            v.retain(|&n| n != node);
            if v.is_empty() {
                self.id_index.remove(old);
            }
        }
        if let Some(new) = new {
            self.id_index.entry(new.to_string()).or_default().push(node);
        }
    }

    /// `document.getElementById` (`scope` = an element's wire id -- typically
    /// `documentElement` -- used only to find the document's root node). Candidates in the `id`
    /// index are filtered to that document; with more than one, the first in document order
    /// wins, resolved by a single pre-order walk from the root rather than a
    /// `compareDocumentPosition`-style pairwise comparator (duplicate ids are rare, so this
    /// stays a cold path).
    pub fn get_by_id(&mut self, scope: u32, id: &str) -> Option<u32> {
        if id.is_empty() {
            return None;
        }
        let scope_id = self.resolve(scope)?;
        let doc_root = self.topmost(scope_id)?;
        let matches: Vec<NodeId> = self
            .id_index
            .get(id)?
            .iter()
            .copied()
            .filter(|&n| self.topmost(n) == Some(doc_root))
            .collect();
        let hit = match matches.as_slice() {
            [] => return None,
            [only] => *only,
            many => {
                let wanted: HashSet<NodeId> = many.iter().copied().collect();
                self.arena
                    .get(doc_root)?
                    .descendants()
                    .find(|n| wanted.contains(&n.id()))?
                    .id()
            }
        };
        Some(self.wire(hit))
    }

    fn topmost(&self, id: NodeId) -> Option<NodeId> {
        let node = self.arena.get(id)?;
        Some(node.ancestors().last().map(|a| a.id()).unwrap_or(id))
    }

    /// Detaches `child` from wherever it currently sits, then re-attaches it under `parent`
    /// (before `before`, or last if `None`). An unknown wire id is skipped.
    pub fn insert(&mut self, parent: u32, child: u32, before: Option<u32>) {
        let (Some(parent_id), Some(child_id)) = (self.resolve(parent), self.resolve(child)) else {
            return;
        };
        if let Some(mut n) = self.arena.get_mut(child_id) {
            n.detach();
        }
        // `insert_id_before` panics on an orphan reference node; fall back to append if the
        // arena never linked `before` under a parent.
        let before_id = before
            .and_then(|b| self.resolve(b))
            .filter(|&b| self.arena.get(b).and_then(|n| n.parent()).is_some());
        match before_id {
            Some(before_id) => {
                if let Some(mut n) = self.arena.get_mut(before_id) {
                    n.insert_id_before(child_id);
                }
            }
            None => {
                if let Some(mut n) = self.arena.get_mut(parent_id) {
                    n.append_id(child_id);
                }
            }
        }
        self.mutations += 1;
    }

    /// Detaches the node so it stops appearing in the arena's traversal. The slot stays.
    pub fn remove(&mut self, child: u32) {
        if let Some(id) = self.resolve(child) {
            if let Some(mut n) = self.arena.get_mut(id) {
                n.detach();
            }
            self.mutations += 1;
        }
    }

    /// Creates a detached node (JS `createElement` / `createTextNode` / `createComment` /
    /// `createDocumentFragment`) as an arena orphan and returns its wire id. `name` is the
    /// element's local name, kept as given, the text/comment data, the PI target or the
    /// doctype name. `ns` / `prefix` are only read for elements; `ns` accepts the short forms
    /// `ns_short` emits, and `None` is the null namespace (`createElementNS(null, ...)`).
    pub fn create(
        &mut self,
        node_type: u8,
        name: Option<&str>,
        ns: Option<&str>,
        prefix: Option<&str>,
    ) -> u32 {
        let name = name.unwrap_or_default();
        let node = match node_type {
            3 | 4 => Node::text(name, node_type == 4),
            7 => Node::processing_instruction(name, ""),
            8 => Node::comment(name),
            10 => Node::doctype(name, "", ""),
            11 => Node::fragment(),
            // element -- anything else is a caller bug, treat as element
            _ => {
                let ns = ns.map(|n| ns_full(Some(n))).unwrap_or_default();
                let qual = QualName::new(prefix.map(Prefix::from), ns, LocalName::from(name));
                Node::element(qual, Vec::new())
            }
        };
        let id = self.arena.orphan(node).id();
        self.mutations += 1;
        self.wire(id)
    }

    pub fn set_template_contents(&mut self, template: u32, contents: u32) {
        if let Some(c) = self.resolve(contents) {
            self.edit_element(template, |e| e.template_contents = Some(c));
        }
    }

    /// An empty `value` clears it, as jsdom skips a falsy `_isValue`.
    pub fn set_is_value(&mut self, wire: u32, value: &str) {
        self.edit_element(wire, |e| {
            e.is_value = (!value.is_empty()).then(|| value.into())
        });
    }

    /// Creates a detached `Document` (JS `createDocument` / `createHTMLDocument` /
    /// `DOMParser`), flagged HTML or XML.
    pub fn create_document(&mut self, html: bool) -> u32 {
        let id = self.arena.orphan(Node::document(!html)).id();
        self.mutations += 1;
        self.wire(id)
    }

    /// Whether `wire` is an HTML document; `None` if it is not a document at all.
    pub fn is_html_document(&self, wire: u32) -> Option<bool> {
        let id = self.resolve(wire)?;
        match self.arena.get(id)?.value() {
            Node::Document(d) => Some(!d.xml),
            _ => None,
        }
    }

    /// Identity is the namespace plus local name, as in the DOM spec. Writing over an existing
    /// attribute replaces it in place, prefix included: jsdom's `replaceAttribute` can swap in
    /// an `Attr` with another prefix, and a plain value change passes the current one.
    pub fn set_attr(
        &mut self,
        wire: u32,
        ns: Option<&str>,
        prefix: Option<&str>,
        local: &str,
        value: &str,
    ) {
        let ns = Namespace::from(ns.unwrap_or_default());
        self.reindex_id_attr(wire, &ns, local, Some(value));
        self.mutate_attrs(wire, |attrs| {
            let attr = Attribute {
                name: QualName::new(prefix.map(Prefix::from), ns, LocalName::from(local)),
                value: value.into(),
            };
            match attrs
                .iter_mut()
                .find(|a| same_attr(&a.name, &attr.name.ns, local))
            {
                Some(a) => *a = attr,
                None => attrs.push(attr),
            }
        });
    }

    pub fn remove_attr(&mut self, wire: u32, ns: Option<&str>, local: &str) {
        let ns = Namespace::from(ns.unwrap_or_default());
        self.reindex_id_attr(wire, &ns, local, None);
        self.mutate_attrs(wire, |attrs| {
            attrs.retain(|a| !same_attr(&a.name, &ns, local));
        });
    }

    /// Keeps `id_index` in step with a write to the element's no-namespace `id` attribute.
    fn reindex_id_attr(&mut self, wire: u32, ns: &Namespace, local: &str, new: Option<&str>) {
        if !ns.is_empty() || local != "id" {
            return;
        }
        let Some(id) = self.resolve(wire) else { return };
        let Some(Node::Element(e)) = self.arena.get(id).map(|n| n.value()) else {
            return;
        };
        let old = id_of(e);
        self.reindex_id(id, old.as_deref(), new);
    }

    fn template_contents(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(id)?.value().as_element()?.template_contents
    }

    /// Edits an element in place; `None` when `wire` is not an element.
    fn edit_element<R>(&mut self, wire: u32, edit: impl FnOnce(&mut Element) -> R) -> Option<R> {
        let id = self.resolve(wire)?;
        let mut node = self.arena.get_mut(id)?;
        node.value().as_element_mut().map(edit)
    }

    fn mutate_attrs(&mut self, wire: u32, edit: impl FnOnce(&mut Vec<Attribute>)) {
        if self.edit_element(wire, |e| edit(&mut e.attrs)).is_some() {
            self.mutations += 1;
        }
    }

    /// Text / comment / processing-instruction `data` write. No-op for anything else.
    pub fn set_data(&mut self, wire: u32, data: &str) {
        let Some(id) = self.resolve(wire) else { return };
        let Some(mut node) = self.arena.get_mut(id) else {
            return;
        };
        match node.value() {
            Node::Text(t) => t.text = data.into(),
            Node::Comment(c) => c.comment = data.into(),
            Node::ProcessingInstruction(p) => p.data = data.into(),
            _ => return,
        }
        self.mutations += 1;
    }

    pub fn children(&mut self, wire: u32) -> Vec<NodeRecord> {
        let Some(id) = self.resolve(wire) else {
            return Vec::new();
        };
        let kids: Vec<NodeId> = match self.arena.get(id) {
            Some(node) => node.children().map(|c| c.id()).collect(),
            None => return Vec::new(),
        };
        kids.into_iter().filter_map(|k| self.record(k)).collect()
    }

    /// Previous/next sibling of `wire`; `None` at a tree edge or for an unknown / orphan node.
    fn sibling(&mut self, wire: u32, forward: bool) -> Option<u32> {
        let id = self.resolve(wire)?;
        let node = self.arena.get(id)?;
        let hit = if forward {
            node.next_sibling()
        } else {
            node.prev_sibling()
        }?;
        Some(self.wire(hit.id()))
    }

    pub fn next_sibling(&mut self, wire: u32) -> Option<u32> {
        self.sibling(wire, true)
    }

    pub fn prev_sibling(&mut self, wire: u32) -> Option<u32> {
        self.sibling(wire, false)
    }

    /// Wire-id form of `parent`, for jsdom's tree reads.
    pub fn parent_wire(&mut self, wire: u32) -> Option<u32> {
        let hit = self.arena.get(self.resolve(wire)?)?.parent()?.id();
        Some(self.wire(hit))
    }

    pub fn first_child(&mut self, wire: u32) -> Option<u32> {
        let hit = self.arena.get(self.resolve(wire)?)?.first_child()?.id();
        Some(self.wire(hit))
    }

    pub fn last_child(&mut self, wire: u32) -> Option<u32> {
        let hit = self.arena.get(self.resolve(wire)?)?.last_child()?.id();
        Some(self.wire(hit))
    }

    pub fn child_count(&self, wire: u32) -> u32 {
        self.resolve(wire)
            .and_then(|id| self.arena.get(id))
            .map_or(0, |n| n.children().count() as u32)
    }

    /// Position among its siblings; `None` for a node without a parent.
    pub fn index(&self, wire: u32) -> Option<u32> {
        let node = self.arena.get(self.resolve(wire)?)?;
        node.parent()?;
        Some(node.prev_siblings().count() as u32)
    }

    pub fn last_inclusive_descendant(&mut self, wire: u32) -> Option<u32> {
        let mut node = self.arena.get(self.resolve(wire)?)?;
        while let Some(last) = node.last_child() {
            node = last;
        }
        let hit = node.id();
        Some(self.wire(hit))
    }

    /// Next node in tree order, never leaving `root`'s subtree; `skip_children` jumps over
    /// `wire`'s own descendants.
    pub fn following(&mut self, wire: u32, root: Option<u32>, skip_children: bool) -> Option<u32> {
        let root = root.and_then(|r| self.resolve(r));
        let node = self.arena.get(self.resolve(wire)?)?;
        let hit = match node.first_child().filter(|_| !skip_children) {
            Some(child) => child.id(),
            None => std::iter::once(node)
                .chain(node.ancestors())
                .take_while(|n| Some(n.id()) != root)
                .find_map(|n| n.next_sibling())?
                .id(),
        };
        Some(self.wire(hit))
    }

    /// Previous node in tree order, `None` once `root` is reached.
    pub fn preceding(&mut self, wire: u32, root: Option<u32>) -> Option<u32> {
        let id = self.resolve(wire)?;
        if root.and_then(|r| self.resolve(r)) == Some(id) {
            return None;
        }
        let node = self.arena.get(id)?;
        match node.prev_sibling() {
            Some(prev) => {
                let prev = self.wire(prev.id());
                self.last_inclusive_descendant(prev)
            }
            None => {
                let parent = node.parent()?.id();
                Some(self.wire(parent))
            }
        }
    }

    pub fn child_wires(&mut self, wire: u32) -> Vec<u32> {
        self.walk_wires(wire, |n| n.children().map(|c| c.id()).collect())
    }

    /// `wire` and its descendants, in tree order.
    pub fn tree_wires(&mut self, wire: u32) -> Vec<u32> {
        self.walk_wires(wire, |n| n.descendants().map(|c| c.id()).collect())
    }

    /// The HTML elements of `wire`'s inclusive subtree the window lists as named properties,
    /// with their non-empty `id` and, for `NAMED_ELEMENTS`, `name`: one crossing per connect or
    /// disconnect instead of attribute reads per element.
    pub fn named_keys(&mut self, wire: u32) -> Vec<(u32, Option<String>, Option<String>)> {
        let found: Vec<_> = match self.resolve(wire).and_then(|id| self.arena.get(id)) {
            Some(root) => root
                .descendants()
                .filter_map(|n| {
                    let e = n.value().as_element().filter(|e| &*e.name.ns == HTML_NS)?;
                    let non_empty = |v: Option<&str>| v.filter(|v| !v.is_empty()).map(String::from);
                    let id = non_empty(e.attr("id"));
                    let name = non_empty(e.attr("name"))
                        .filter(|_| NAMED_ELEMENTS.contains(&&*e.name.local));
                    (id.is_some() || name.is_some()).then(|| (n.id(), id, name))
                })
                .collect(),
            None => return Vec::new(),
        };
        found
            .into_iter()
            .map(|(n, id, name)| (self.wire(n), id, name))
            .collect()
    }

    fn walk_wires(
        &mut self,
        wire: u32,
        walk: impl FnOnce(ego_tree::NodeRef<Node>) -> Vec<NodeId>,
    ) -> Vec<u32> {
        let Some(node) = self.resolve(wire).and_then(|id| self.arena.get(id)) else {
            return Vec::new();
        };
        walk(node).into_iter().map(|id| self.wire(id)).collect()
    }

    /// `wire`'s descendants in tree order, without template content.
    pub fn tree_records(&mut self, wire: u32) -> Vec<TreeRecord> {
        let Some(root) = self.resolve(wire).and_then(|id| self.arena.get(id)) else {
            return Vec::new();
        };
        let nodes: Vec<NodeId> = root.descendants().skip(1).map(|n| n.id()).collect();
        nodes.into_iter().map(|id| self.tree_record(id)).collect()
    }

    /// The nodes of `created` the parser linked under a node that was already there, each with
    /// its new descendants, in tree order. They are detached and leave `created`; a template's
    /// content leaves with its template. Nodes still detached stay for a later step.
    fn take_parsed(
        &mut self,
        created: &mut HashSet<NodeId>,
        popped: &mut HashSet<NodeId>,
    ) -> Vec<ParsedRecord> {
        let parent_of = |id: NodeId| self.arena.get(id)?.parent().map(|p| p.id());
        // Keyed by tree, then by position in it.
        let mut roots: Vec<(NodeId, Vec<usize>, NodeId)> = created
            .iter()
            .filter(|&&id| parent_of(id).is_some_and(|p| !created.contains(&p)))
            .map(|&id| {
                let node = self.arena.get(id).expect("created");
                let mut path: Vec<usize> = std::iter::once(node)
                    .chain(node.ancestors())
                    .map(|n| n.prev_siblings().count())
                    .collect();
                path.reverse();
                let tree = node.ancestors().last().expect("linked").id();
                (tree, path, id)
            })
            .collect();
        roots.sort();
        let mut order = Vec::new();
        for (_, _, root) in roots {
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                if !created.remove(&id) {
                    continue;
                }
                order.push(id);
                let node = self.arena.get(id).expect("created");
                stack.extend(node.children().rev().map(|c| c.id()));
            }
        }
        for &id in &order {
            if let Some(contents) = self.template_contents(id) {
                self.forget_subtree(created, contents);
            }
        }
        let emitted: HashSet<NodeId> = order.iter().copied().collect();
        let records = order
            .iter()
            .map(|&id| {
                let node = self.arena.get(id).expect("created");
                let parent = node.parent().expect("linked").id();
                let before = node
                    .next_siblings()
                    .find(|s| !emitted.contains(&s.id()))
                    .map(|s| s.id());
                let doctype = match node.value() {
                    Node::Doctype(d) => Some(DoctypeIds {
                        public_id: d.public_id.to_string(),
                        system_id: d.system_id.to_string(),
                    }),
                    _ => None,
                };
                ParsedRecord {
                    node: self.tree_record(id),
                    parent: self.wire(parent),
                    before: before.map(|b| self.wire(b)),
                    popped: popped.remove(&id),
                    doctype,
                }
            })
            .collect();
        for id in order {
            self.arena.get_mut(id).expect("created").detach();
        }
        self.mutations += 1;
        records
    }

    /// Drops `root`'s subtree, template content included, from `created`.
    fn forget_subtree(&self, created: &mut HashSet<NodeId>, root: NodeId) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            created.remove(&id);
            let Some(node) = self.arena.get(id) else {
                continue;
            };
            stack.extend(node.children().map(|c| c.id()));
            stack.extend(self.template_contents(id));
        }
    }

    fn tree_record(&mut self, id: NodeId) -> TreeRecord {
        let (node_type, name, ns) = match self.arena.get(id).expect("listed").value() {
            Node::Element(e) => (1, e.name.local.to_string(), e.name.ns.to_string()),
            Node::Doctype(d) => (10, d.name.to_string(), String::new()),
            other => (classify(other).0, String::new(), String::new()),
        };
        let contents = self.template_contents(id);
        TreeRecord {
            id: self.wire(id),
            node_type,
            name,
            ns,
            contents: contents.map(|c| self.wire(c)),
        }
    }

    #[cfg(test)]
    pub fn nodes(&mut self, wires: &[u32]) -> Vec<NodeRecord> {
        let ids: Vec<NodeId> = wires.iter().filter_map(|&w| self.resolve(w)).collect();
        ids.into_iter().filter_map(|id| self.record(id)).collect()
    }

    /// Attributes of an element, in insertion order; empty for non-elements.
    pub fn attrs(&self, wire: u32) -> Vec<AttrRecord> {
        let Some(id) = self.resolve(wire) else {
            return Vec::new();
        };
        match self.arena.get(id).map(|n| n.value()) {
            Some(Node::Element(e)) => e
                .attrs
                .iter()
                .map(|a| attr_record(&a.name, &a.value))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn attr_value(&self, wire: u32, ns: Option<&str>, local: &str) -> Option<String> {
        let ns = Namespace::from(ns.unwrap_or_default());
        match self.arena.get(self.resolve(wire)?)?.value() {
            Node::Element(e) => e
                .attrs
                .iter()
                .find(|a| same_attr(&a.name, &ns, local))
                .map(|a| a.value.to_string()),
            _ => None,
        }
    }

    /// The attribute with namespace `ns` and local name `local`.
    pub fn attr(&self, wire: u32, ns: Option<&str>, local: &str) -> Option<AttrRecord> {
        let ns = Namespace::from(ns.unwrap_or_default());
        self.find_attr(wire, |k| same_attr(k, &ns, local))
    }

    /// The first attribute whose qualified name (`prefix:local`, or just `local`) is `name`.
    pub fn attr_by_name(&self, wire: u32, name: &str) -> Option<AttrRecord> {
        self.find_attr(wire, |k| match &k.prefix {
            Some(p) => name.strip_prefix(&**p).and_then(|r| r.strip_prefix(':')) == Some(&*k.local),
            None => *k.local == *name,
        })
    }

    fn find_attr(&self, wire: u32, matches: impl Fn(&QualName) -> bool) -> Option<AttrRecord> {
        match self.arena.get(self.resolve(wire)?)?.value() {
            Node::Element(e) => e
                .attrs
                .iter()
                .find(|a| matches(&a.name))
                .map(|a| attr_record(&a.name, &a.value)),
            _ => None,
        }
    }

    /// `innerHTML`, or with `outer` `outerHTML`, of `wire` in an HTML document: what parse5
    /// writes through jsdom's tree adapter, so names are qualified and processing instructions
    /// and CDATA are dropped. `scripting` is the document's flag that keeps `<noscript>` raw.
    pub fn serialize(&self, wire: u32, outer: bool, scripting: bool) -> Option<String> {
        let node = self.arena.get(self.resolve(wire)?)?;
        let mut out = String::new();
        if outer {
            self.write_node(node, scripting, &mut out);
        } else {
            self.write_children(node, scripting, &mut out);
        }
        Some(out)
    }

    fn write_children(&self, node: NodeRef<'_, Node>, scripting: bool, out: &mut String) {
        let Some(e) = node.value().as_element() else {
            return self.write_nodes(node, scripting, out);
        };
        if is_void(e) {
            return;
        }
        let container = match html_name(e) {
            Some("template") => e
                .template_contents
                .and_then(|c| self.arena.get(c))
                .unwrap_or(node),
            _ => node,
        };
        self.write_nodes(container, scripting, out);
    }

    fn write_nodes(&self, parent: NodeRef<'_, Node>, scripting: bool, out: &mut String) {
        for child in parent.children() {
            self.write_node(child, scripting, out);
        }
    }

    fn write_node(&self, node: NodeRef<'_, Node>, scripting: bool, out: &mut String) {
        match node.value() {
            Node::Element(e) => {
                let name = qualified(&e.name);
                out.push('<');
                out.push_str(&name);
                if let Some(is) = &e.is_value
                    && !e.attrs.iter().any(|a| qualified(&a.name) == "is")
                {
                    write_attr(out, "is", is);
                }
                for a in &e.attrs {
                    write_attr(out, &qualified(&a.name), &a.value);
                }
                out.push('>');
                if !is_void(e) {
                    self.write_children(node, scripting, out);
                    out.push_str("</");
                    out.push_str(&name);
                    out.push('>');
                }
            }
            Node::Text(t) if !t.cdata => {
                let raw = node
                    .parent()
                    .and_then(|p| p.value().as_element().and_then(html_name))
                    .is_some_and(|n| {
                        RAW_TEXT_ELEMENTS.contains(&n) || (scripting && n == "noscript")
                    });
                if raw {
                    out.push_str(&t.text);
                } else {
                    escape(out, &t.text, false);
                }
            }
            Node::Comment(c) => {
                out.push_str("<!--");
                out.push_str(&c.comment);
                out.push_str("-->");
            }
            Node::Doctype(d) => {
                out.push_str("<!DOCTYPE ");
                out.push_str(&d.name);
                out.push('>');
            }
            _ => {}
        }
    }

    /// `data` for a text, comment or processing-instruction node; `None` for anything else.
    pub fn data(&self, wire: u32) -> Option<String> {
        let id = self.resolve(wire)?;
        match self.arena.get(id)?.value() {
            Node::Text(t) => Some(t.text.to_string()),
            Node::Comment(c) => Some(c.comment.to_string()),
            Node::ProcessingInstruction(p) => Some(p.data.to_string()),
            _ => None,
        }
    }

    fn record(&mut self, id: NodeId) -> Option<NodeRecord> {
        let (node_type, name, ns) = classify(self.arena.get(id)?.value());
        let wire = self.wire(id);
        Some(NodeRecord {
            id: wire,
            node_type,
            name,
            ns,
        })
    }

    fn wire(&mut self, id: NodeId) -> u32 {
        let mut node = self.arena.get_mut(id).expect("node in arena");
        let slot = node.value().wire_mut();
        if *slot != NO_WIRE {
            return *slot;
        }
        let w = self.ids.len() as u32;
        *slot = w;
        self.ids.push(id);
        w
    }

    fn resolve(&self, wire: u32) -> Option<NodeId> {
        self.ids.get(wire as usize).copied()
    }
}

impl Default for DomState {
    fn default() -> Self {
        Self::new()
    }
}

/// html5ever's tree sink over the arena itself, so the parser builds the document in place and
/// sees whatever a script changed while it was paused. It notes the nodes it creates and pops,
/// which JS then materializes. Nodes JS already has are left unchanged: their edits go to JS,
/// which runs jsdom's own steps for them.
struct ArenaSink {
    /// The arena, moved in for the length of a parse step only.
    dom: RefCell<DomState>,
    document: NodeId,
    created: RefCell<HashSet<NodeId>>,
    popped: RefCell<HashSet<NodeId>>,
    quirks: Cell<Option<QuirksMode>>,
    attrs: RefCell<Vec<(u32, AttrRecord)>>,
    text: RefCell<Vec<(u32, String)>>,
}

impl ArenaSink {
    /// A sink for `document`, holding a placeholder arena until the caller swaps the real one in.
    fn new(document: NodeId) -> Self {
        Self {
            dom: RefCell::new(DomState::new()),
            document,
            created: RefCell::default(),
            popped: RefCell::default(),
            quirks: Cell::new(None),
            attrs: RefCell::default(),
            text: RefCell::default(),
        }
    }

    fn orphan(&self, node: Node) -> NodeId {
        let id = self.dom.borrow_mut().arena.orphan(node).id();
        self.created.borrow_mut().insert(id);
        id
    }

    /// Appends text to `parent`, or before `sibling` when given, merging with the text node
    /// already there.
    fn insert_text(&self, parent: NodeId, sibling: Option<NodeId>, text: &str) {
        let mut dom = self.dom.borrow_mut();
        let neighbour = match sibling {
            Some(s) => dom.arena.get(s).and_then(|s| s.prev_sibling()),
            None => dom.arena.get(parent).and_then(|p| p.last_child()),
        }
        .map(|n| n.id());
        if let Some(n) = neighbour
            && let Node::Text(t) = dom.arena.get_mut(n).expect("sibling").value()
        {
            if self.created.borrow().contains(&n) {
                t.text.push_str(text);
            } else {
                let wire = dom.wire(n);
                self.text.borrow_mut().push((wire, text.into()));
            }
            return;
        }
        let node = Node::text(text, false);
        let id = match sibling {
            Some(s) => dom
                .arena
                .get_mut(s)
                .expect("sibling")
                .insert_before(node)
                .id(),
            None => dom.arena.get_mut(parent).expect("parent").append(node).id(),
        };
        self.created.borrow_mut().insert(id);
    }
}

impl TreeSink for ArenaSink {
    type Handle = NodeId;
    type Output = DomState;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> DomState {
        self.dom.into_inner()
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        self.document
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> Ref<'a, QualName> {
        Ref::map(self.dom.borrow(), |dom| {
            &dom.arena
                .get(*target)
                .and_then(|n| n.value().as_element())
                .expect("element")
                .name
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, _: ElementFlags) -> NodeId {
        let template = name.ns == ns!(html) && name.local == local_name!("template");
        let id = self.orphan(Node::element(name, attrs));
        self.dom.borrow_mut().index_ids_in_subtree(id);
        if template {
            let contents = self.orphan(Node::fragment());
            let mut dom = self.dom.borrow_mut();
            if let Some(mut node) = dom.arena.get_mut(id)
                && let Some(e) = node.value().as_element_mut()
            {
                e.template_contents = Some(contents);
            }
        }
        id
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.orphan(Node::comment(&text))
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
        self.orphan(Node::processing_instruction(&target, &data))
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(id) => {
                let mut dom = self.dom.borrow_mut();
                dom.arena.get_mut(*parent).expect("parent").append_id(id);
            }
            NodeOrText::AppendText(text) => self.insert_text(*parent, None, &text),
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self
            .dom
            .borrow()
            .arena
            .get(*element)
            .is_some_and(|n| n.parent().is_some());
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, name: StrTendril, public: StrTendril, system: StrTendril) {
        let id = self.orphan(Node::doctype(&name, &public, &system));
        self.append(&self.document, NodeOrText::AppendNode(id));
    }

    fn pop(&self, node: &NodeId) {
        self.popped.borrow_mut().insert(*node);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        self.dom
            .borrow()
            .template_contents(*target)
            .expect("template")
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks.set(Some(mode));
    }

    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        match new_node {
            NodeOrText::AppendNode(id) => {
                let mut dom = self.dom.borrow_mut();
                dom.arena.get_mut(id).expect("node").detach();
                let mut sibling = dom.arena.get_mut(*sibling).expect("sibling");
                if sibling.parent().is_some() {
                    sibling.insert_id_before(id);
                }
            }
            NodeOrText::AppendText(text) => {
                let parent = self
                    .dom
                    .borrow()
                    .arena
                    .get(*sibling)
                    .and_then(|s| s.parent())
                    .map(|p| p.id());
                if let Some(parent) = parent {
                    self.insert_text(parent, Some(*sibling), &text);
                }
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        let mut dom = self.dom.borrow_mut();
        let wire = dom.wire(*target);
        let created = self.created.borrow().contains(target);
        for attr in attrs {
            let QualName { prefix, ns, local } = attr.name;
            let ns = (!ns.is_empty()).then_some(&*ns);
            if dom.attr_value(wire, ns, &local).is_some() {
                continue;
            }
            if created {
                dom.set_attr(wire, ns, prefix.as_deref(), &local, &attr.value);
            } else {
                let record = AttrRecord {
                    ns: ns.map(Into::into),
                    prefix: prefix.map(|p| p.to_string()),
                    local: local.to_string(),
                    value: attr.value.to_string(),
                };
                self.attrs.borrow_mut().push((wire, record));
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        let mut dom = self.dom.borrow_mut();
        dom.arena.get_mut(*target).expect("node").detach();
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut dom = self.dom.borrow_mut();
        let mut new_parent = dom.arena.get_mut(*new_parent).expect("parent");
        new_parent.reparent_from_id_append(*node);
    }
}

/// An HTML document being parsed into the arena. A step runs the parser up to the next
/// `<script>` in the document, so the script runs before anything after it is parsed.
pub struct DocumentParser {
    tokenizer: Tokenizer<TreeBuilder<NodeId, ArenaSink>>,
    input: BufferQueue,
    pause: bool,
}

impl DocumentParser {
    /// `document` is the wire id of the (empty) document node; `scripting` is its scripting
    /// flag, and `pause` makes each step stop after a `<script>` in the document.
    pub fn new(
        dom: &DomState,
        document: u32,
        html: &str,
        scripting: bool,
        pause: bool,
    ) -> Option<Self> {
        let sink = ArenaSink::new(dom.resolve(document)?);
        let opts = TreeBuilderOpts {
            scripting_enabled: scripting,
            ..Default::default()
        };
        let input = BufferQueue::default();
        input.push_back(StrTendril::from_slice(html));
        Some(Self {
            tokenizer: Tokenizer::new(TreeBuilder::new(sink, opts), TokenizerOpts::default()),
            input,
            pause,
        })
    }

    pub fn step(&self, dom: &mut DomState) -> ParseStep {
        let sink = &self.tokenizer.sink.sink;
        let done = self.feed(dom);
        let records = dom.take_parsed(
            &mut sink.created.borrow_mut(),
            &mut sink.popped.borrow_mut(),
        );
        let quirks = sink.quirks.take().map(|mode| match mode {
            QuirksMode::Quirks => "quirks",
            QuirksMode::LimitedQuirks => "limited-quirks",
            QuirksMode::NoQuirks => "no-quirks",
        });
        ParseStep {
            records,
            attrs: sink.attrs.take(),
            text: sink.text.take(),
            done,
            quirks,
        }
    }

    /// Runs the parser with `dom` swapped in, up to the next pause or the end of the input;
    /// whether it reached the end.
    fn feed(&self, dom: &mut DomState) -> bool {
        let sink = &self.tokenizer.sink.sink;
        std::mem::swap(dom, &mut sink.dom.borrow_mut());
        let done = loop {
            match self.tokenizer.feed(&self.input) {
                TokenizerResult::Done => {
                    self.tokenizer.end();
                    break true;
                }
                TokenizerResult::Script(node) if self.pause && self.in_document(node) => {
                    break false;
                }
                _ => {}
            }
        };
        std::mem::swap(dom, &mut sink.dom.borrow_mut());
        done
    }

    fn in_document(&self, node: NodeId) -> bool {
        let sink = &self.tokenizer.sink.sink;
        sink.dom.borrow().topmost(node) == Some(sink.document)
    }
}

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "meta", "param", "source", "track", "wbr",
];

const RAW_TEXT_ELEMENTS: &[&str] = &[
    "style",
    "script",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "plaintext",
];

/// Elements the window exposes by `name` as well as by `id`; jsdom also tracks frames here.
const NAMED_ELEMENTS: &[&str] = &["embed", "form", "img", "object", "iframe", "frame"];

/// The local name of an HTML element without a prefix. parse5 checks the qualified tag name,
/// so `h:br` is neither void nor raw text.
fn html_name(e: &Element) -> Option<&str> {
    (e.name.prefix.is_none() && &*e.name.ns == HTML_NS).then_some(&*e.name.local)
}

fn is_void(e: &Element) -> bool {
    html_name(e).is_some_and(|n| VOID_ELEMENTS.contains(&n))
}

fn qualified(name: &QualName) -> String {
    match &name.prefix {
        Some(p) => format!("{p}:{}", name.local),
        None => name.local.to_string(),
    }
}

fn write_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape(out, value, true);
    out.push('"');
}

/// parse5's escaping: `<` and `>` stay raw in attribute values.
fn escape(out: &mut String, text: &str, attr: bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// Attribute identity per the DOM spec: namespace plus local name, prefix ignored.
fn attr_record(name: &QualName, value: &str) -> AttrRecord {
    AttrRecord {
        ns: (!name.ns.is_empty()).then(|| name.ns.to_string()),
        prefix: name.prefix.as_ref().map(|p| p.to_string()),
        local: name.local.to_string(),
        value: value.to_string(),
    }
}

fn same_attr(name: &QualName, ns: &Namespace, local: &str) -> bool {
    name.ns == *ns && &*name.local == local
}

fn id_of(e: &Element) -> Option<String> {
    let no_ns = Namespace::from("");
    e.attrs
        .iter()
        .find(|a| same_attr(&a.name, &no_ns, "id"))
        .map(|a| a.value.to_string())
}

fn classify(node: &Node) -> (u8, Option<String>, Option<String>) {
    match node {
        Node::Document(_) => (9, None, None),
        Node::Fragment(_) => (11, None, None),
        Node::Doctype(d) => (10, Some(d.name.to_string()), None),
        Node::Comment(_) => (8, None, None),
        Node::Text(_) => (3, None, None),
        Node::ProcessingInstruction(_) => (7, None, None),
        // ponytail: uppercases every tag name (HTML `tagName` convention); SVG/MathML
        // camelCase fixup can come when the query patch needs it.
        Node::Element(e) => (1, Some(e.name.local.to_uppercase()), ns_short(&e.name.ns)),
    }
}

/// Inverse of `ns_short`: the create path takes the short form (or a full URL) and needs a
/// real `Namespace`. `None` / unknown short forms default to XHTML, matching html5ever's own
/// default element namespace.
fn ns_full(ns: Option<&str>) -> Namespace {
    match ns {
        None | Some("html") => Namespace::from("http://www.w3.org/1999/xhtml"),
        Some("svg") => Namespace::from("http://www.w3.org/2000/svg"),
        Some("mathml") => Namespace::from("http://www.w3.org/1998/Math/MathML"),
        Some(other) => Namespace::from(other),
    }
}

fn ns_short(ns: &str) -> Option<String> {
    match ns {
        "" => None,
        "http://www.w3.org/1999/xhtml" => Some("html".into()),
        "http://www.w3.org/2000/svg" => Some("svg".into()),
        "http://www.w3.org/1998/Math/MathML" => Some("mathml".into()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XLINK: &str = "http://www.w3.org/1999/xlink";

    /// `cargo test --release bench_mutate_then_query -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_mutate_then_query() {
        let items: String = (0..1240)
            .map(|i| format!("<li id=i{i} class='membro x' title=t>{i}</li>"))
            .collect();
        let mut dom = DomState::new();
        let doc = dom.parse(&format!("<ul>{items}</ul>"), None)[0].id;
        let state = dom.query_state(true, false, -1, -1);
        let first = dom.query(doc, "li", QueryMode::First, &state).unwrap()[0];
        for run in 0..5 {
            let start = std::time::Instant::now();
            for _ in 0..100 {
                for n in 0..40 {
                    dom.set_attr(first, None, None, "data-n", &n.to_string());
                    let hits = dom.query(doc, ".membro", QueryMode::All, &state);
                    assert_eq!(hits.unwrap().len(), 1240);
                }
            }
            println!(
                "run {run}: {:.3} ms per 40x",
                start.elapsed().as_secs_f64() * 10.0
            );
        }
    }

    fn child_named(dom: &mut DomState, wire: u32, name: &str) -> u32 {
        dom.children(wire)
            .into_iter()
            .find(|r| r.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no <{name}> child of {wire}"))
            .id
    }

    fn plain(local: &str, value: &str) -> AttrRecord {
        AttrRecord {
            ns: None,
            prefix: None,
            local: local.into(),
            value: value.into(),
        }
    }

    #[test]
    fn parse_document_and_walk_the_spine() {
        let mut dom = DomState::new();
        let roots = dom.parse(
            "<!DOCTYPE html><html><body><div id='x'>hi<!-- c --></div></body></html>",
            None,
        );
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].node_type, 9); // document

        let doc = roots[0].id;
        let html = child_named(&mut dom, doc, "HTML");
        assert_eq!(
            dom.children(doc)
                .iter()
                .find(|r| r.node_type == 10)
                .map(|r| r.node_type),
            Some(10),
            "doctype kept as a child"
        );

        let body = child_named(&mut dom, html, "BODY");
        let div = child_named(&mut dom, body, "DIV");
        let div_rec = dom.nodes(&[div]).pop().unwrap();
        assert_eq!(div_rec.ns.as_deref(), Some("html"));
        assert_eq!(dom.attrs(div), vec![plain("id", "x")]);

        let kids = dom.children(div);
        assert_eq!(
            kids.iter().map(|r| r.node_type).collect::<Vec<_>>(),
            vec![3, 8]
        );
        assert_eq!(dom.data(kids[0].id).as_deref(), Some("hi"));

        // a data write reaches serialization
        dom.set_data(kids[0].id, "bye");
        assert_eq!(dom.data(kids[0].id).as_deref(), Some("bye"));
        dom.set_data(div, "ignored"); // element -> no-op
        assert_eq!(
            dom.serialize(div, true, true).as_deref(),
            Some("<div id=\"x\">bye<!-- c --></div>")
        );
        assert_eq!(dom.data(kids[1].id).as_deref(), Some(" c "));
        assert_eq!(dom.data(div), None);

        let pi = dom.create(7, Some("target"), None, None);
        dom.set_data(pi, "d");
        assert_eq!(dom.data(pi).as_deref(), Some("d"));

        // parent walks back up, identity is stable through the wire table
        assert_eq!(dom.parent_wire(div).unwrap(), body);
        assert_eq!(dom.parent_wire(body).unwrap(), html);
        assert!(dom.parent_wire(doc).is_none());
    }

    #[test]
    fn serialize_walks_the_arena_subtree() {
        let mut dom = DomState::new();
        let roots = dom.parse(
            "<div id='a' class='b'>hi &amp; <span>x</span><br></div>",
            Some(""),
        );
        let wrapper = roots[0].id;
        let a = child_named(&mut dom, wrapper, "DIV");

        assert_eq!(
            dom.serialize(a, true, true).as_deref(),
            Some(r#"<div id="a" class="b">hi &amp; <span>x</span><br></div>"#),
        );
        assert_eq!(
            dom.serialize(a, false, true).as_deref(),
            Some("hi &amp; <span>x</span><br>"),
        );
        let text = dom.children(a)[0].id;
        assert_eq!(
            dom.serialize(text, true, true).as_deref(),
            Some("hi &amp; ")
        );
        assert_eq!(dom.serialize(9999, true, true), None);
    }

    #[test]
    fn create_orphans_round_trip_through_classify() {
        let mut dom = DomState::new();

        let div = dom.create(1, Some("div"), Some("html"), None);
        assert!(dom.attrs(div).is_empty());
        let rec = dom.nodes(&[div]).pop().unwrap();
        assert_eq!(rec.node_type, 1);
        assert_eq!(rec.name.as_deref(), Some("DIV"));
        assert_eq!(rec.ns.as_deref(), Some("html"));

        let svg = dom.create(1, Some("svg"), Some("svg"), None);
        assert_eq!(dom.nodes(&[svg]).pop().unwrap().ns.as_deref(), Some("svg"));

        let text = dom.create(3, Some("hello"), None, None);
        assert_eq!(dom.data(text).as_deref(), Some("hello"));
        assert_eq!(dom.nodes(&[text]).pop().unwrap().node_type, 3);

        let comment = dom.create(8, Some(" c "), None, None);
        assert_eq!(dom.data(comment).as_deref(), Some(" c "));

        let frag = dom.create(11, None, None, None);
        assert_eq!(dom.nodes(&[frag]).pop().unwrap().node_type, 11);

        // orphans: no parent, distinct wire ids
        assert!(dom.parent_wire(div).is_none());
        assert_ne!(div, text);
    }

    #[test]
    fn document_parse_pauses_after_each_script_in_the_document() {
        let mut dom = DomState::new();
        let doc = dom.create_document(true);
        let html =
            "<style>p{}</style><p>a<script>1</script><template><script>2</script></template>b";
        let parser = DocumentParser::new(&dom, doc, html, true, true).unwrap();
        let names = |step: &ParseStep| -> Vec<String> {
            step.records.iter().map(|r| r.node.name.clone()).collect()
        };
        let popped = |step: &ParseStep, name: &str| {
            step.records.iter().any(|r| r.node.name == name && r.popped)
        };
        let first = parser.step(&mut dom);
        assert!(!first.done);
        assert_eq!(
            names(&first),
            ["html", "head", "style", "", "body", "p", "", "script", ""]
        );
        assert!(popped(&first, "style") && popped(&first, "script"));
        // Detached for JS to insert, with their parents and anchors recorded.
        assert_eq!(dom.child_count(doc), 0);
        let second = parser.step(&mut dom);
        assert!(second.done);
        assert_eq!(names(&second), ["template", ""]);
        assert!(second.records[0].node.contents.is_some());
    }

    #[test]
    fn parse_fragment_has_a_fragment_root() {
        let mut dom = DomState::new();
        let roots = dom.parse("<p>one</p><p>two</p>", Some(""));
        assert_eq!(roots[0].node_type, 11);
        let names: Vec<_> = dom
            .children(roots[0].id)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, vec![Some("P".into()), Some("P".into())]);
    }

    /// Parses `html` in a fresh `ctx_local` context element, inside a `<form>` when `in_form`.
    fn fragment(
        dom: &mut DomState,
        html: &str,
        ctx_local: &str,
        ctx_ns: &str,
        in_form: bool,
        scripting: bool,
    ) -> u32 {
        let context = dom.create(1, Some(ctx_local), Some(ctx_ns), None);
        let form = in_form.then(|| {
            let form = dom.create(1, Some("form"), Some("html"), None);
            dom.insert(form, context, None);
            form
        });
        dom.parse_fragment(html, context, form, scripting)
    }

    #[test]
    fn fragment_parse_reuses_the_context_nodes() {
        let mut dom = DomState::new();
        let form = dom.create(1, Some("form"), Some("html"), None);
        let context = dom.create(1, Some("div"), Some("html"), None);
        dom.insert(form, context, None);
        let before = dom.arena.nodes().count();
        dom.parse_fragment("<p>x</p>", context, Some(form), true);
        // The fragment root, the tree builder's `<html>` wrapper, `<p>` and its text.
        assert_eq!(dom.arena.nodes().count(), before + 4);
    }

    #[test]
    fn template_content_is_its_own_root() {
        let mut dom = DomState::new();
        let frag = fragment(
            &mut dom,
            "<template id='t'><b>x</b><template><i></i></template></template>",
            "div",
            HTML_NS,
            false,
            true,
        );
        let outer = child_named(&mut dom, frag, "TEMPLATE");
        assert!(dom.children(outer).is_empty());
        assert_eq!(
            dom.serialize(frag, false, true).as_deref(),
            Some(r#"<template id="t"><b>x</b><template><i></i></template></template>"#)
        );
        let contents = dom.template_contents(dom.resolve(outer).unwrap()).unwrap();
        let contents = dom.wire(contents);
        assert!(dom.parent_wire(contents).is_none());
        let inner = child_named(&mut dom, contents, "TEMPLATE");
        assert!(dom.children(inner).is_empty());
    }

    #[test]
    fn fragment_context_namespace_form_and_scripting() {
        let mut dom = DomState::new();
        let svg = "http://www.w3.org/2000/svg";
        let frag = fragment(&mut dom, "<circle/><a>x</a>", "svg", svg, false, true);
        let ns: Vec<_> = dom.children(frag).into_iter().map(|r| r.ns).collect();
        assert_eq!(ns, vec![Some("svg".into()), Some("svg".into())]);

        let html = "<form id='inner'><input></form>";
        let outside = fragment(&mut dom, html, "div", HTML_NS, false, true);
        assert!(
            dom.serialize(outside, false, true)
                .unwrap()
                .contains("<form")
        );
        let inside = fragment(&mut dom, html, "div", HTML_NS, true, true);
        assert_eq!(
            dom.serialize(inside, false, true).as_deref(),
            Some("<input>")
        );

        let noscript = "<noscript><b>x</b></noscript>";
        let on = fragment(&mut dom, noscript, "div", HTML_NS, false, true);
        let off = fragment(&mut dom, noscript, "div", HTML_NS, false, false);
        let noscript_kids = |dom: &mut DomState, frag| {
            let n = child_named(dom, frag, "NOSCRIPT");
            dom.children(n)
                .into_iter()
                .map(|r| r.node_type)
                .collect::<Vec<_>>()
        };
        assert_eq!(noscript_kids(&mut dom, on), vec![3]);
        assert_eq!(noscript_kids(&mut dom, off), vec![1]);
    }

    #[test]
    fn insert_and_remove_mirror_the_js_tree() {
        let mut dom = DomState::new();
        let roots = dom.parse("<div id='a'></div>", Some(""));
        let wrapper = roots[0].id;
        let a = child_named(&mut dom, wrapper, "DIV");

        let span = dom.create(1, Some("span"), Some("html"), None);
        let text = dom.create(3, Some("hi"), None, None);

        dom.insert(a, span, None);
        assert_eq!(
            dom.children(a).iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![span]
        );

        // text lands before the span
        dom.insert(a, text, Some(span));
        assert_eq!(
            dom.children(a)
                .iter()
                .map(|r| r.node_type)
                .collect::<Vec<_>>(),
            vec![3, 1]
        );
        assert_eq!(dom.parent_wire(span).unwrap(), a);

        // moving span to a fresh parent detaches it from `a`
        let b = dom.create(1, Some("b"), Some("html"), None);
        dom.insert(b, span, None);
        assert_eq!(
            dom.children(a).iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![text]
        );
        assert_eq!(dom.parent_wire(span).unwrap(), b);

        // an orphan `before` reference falls back to append rather than panicking
        let orphan = dom.create(1, Some("i"), Some("html"), None);
        let late = dom.create(1, Some("u"), Some("html"), None);
        dom.insert(a, late, Some(orphan));
        assert_eq!(
            dom.children(a).iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![text, late]
        );

        let before = dom.mutations;
        dom.remove(text);
        assert!(dom.parent_wire(text).is_none());
        assert_eq!(
            dom.children(a).iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![late]
        );
        assert_eq!(dom.mutations, before + 1);
    }

    #[test]
    fn sibling_nav_crosses_non_elements() {
        let mut dom = DomState::new();
        let roots = dom.parse("<p>a</p>text<!--c--><span>b</span>", Some(""));
        let wrapper = roots[0].id;
        let kids = dom.children(wrapper);
        let (p, text, comment, span) = (kids[0].id, kids[1].id, kids[2].id, kids[3].id);

        assert_eq!(dom.next_sibling(p), Some(text));
        assert_eq!(dom.prev_sibling(span), Some(comment));
        assert_eq!(dom.next_sibling(span), None);
    }

    #[test]
    fn get_by_id_resolves_from_the_index() {
        let mut dom = DomState::new();
        let roots = dom.parse(
            "<!DOCTYPE html><html><body><div id='a'></div></body></html>",
            None,
        );
        let doc = roots[0].id;
        let html = child_named(&mut dom, doc, "HTML");
        let body = child_named(&mut dom, html, "BODY");
        let div = child_named(&mut dom, body, "DIV");

        assert_eq!(dom.get_by_id(html, "a"), Some(div));
        assert_eq!(dom.get_by_id(html, "missing"), None);

        // renaming the id follows the element
        dom.set_attr(div, None, None, "id", "b");
        assert_eq!(dom.get_by_id(html, "a"), None);
        assert_eq!(dom.get_by_id(html, "b"), Some(div));

        // removing the attribute drops it from the index
        dom.remove_attr(div, None, "id");
        assert_eq!(dom.get_by_id(html, "b"), None);
    }

    #[test]
    fn get_by_id_picks_first_in_document_order_and_stays_per_document() {
        let mut dom = DomState::new();
        // two documents in the same arena, sharing a duplicated id -- each must resolve within
        // its own tree, and the duplicate case must return the earlier one in document order.
        let roots_a = dom.parse(
            "<html><body><span id='dup'>first</span><i id='dup'>second</i></body></html>",
            None,
        );
        let html_a = child_named(&mut dom, roots_a[0].id, "HTML");
        let body_a = child_named(&mut dom, html_a, "BODY");
        let first = child_named(&mut dom, body_a, "SPAN");
        assert_eq!(dom.get_by_id(html_a, "dup"), Some(first));

        let roots_b = dom.parse("<html><body><b id='dup'></b></body></html>", None);
        let html_b = child_named(&mut dom, roots_b[0].id, "HTML");
        let body_b = child_named(&mut dom, html_b, "BODY");
        let only_b = child_named(&mut dom, body_b, "B");
        assert_eq!(dom.get_by_id(html_b, "dup"), Some(only_b));
        // document a is unaffected by document b sharing the same id string
        assert_eq!(dom.get_by_id(html_a, "dup"), Some(first));
    }

    #[test]
    fn named_keys_lists_only_what_names_a_window_property() {
        let mut dom = DomState::new();
        let roots = dom.parse(
            "<div id='d'><form id='f' name='n'></form><p name='no'></p><img id='' name='i'>\
             <svg><a id='svg'></a></svg></div>",
            Some(""),
        );
        let div = child_named(&mut dom, roots[0].id, "DIV");
        let keys: Vec<_> = dom
            .named_keys(div)
            .into_iter()
            .map(|(_, id, name)| (id, name))
            .collect();
        let s = |v: &str| Some(v.to_string());
        assert_eq!(keys, vec![(s("d"), None), (s("f"), s("n")), (None, s("i"))]);
    }

    #[test]
    fn fragment_context_keeps_table_rows() {
        let mut dom = DomState::new();
        // <body> context drops a bare <tr>; a <tbody> context keeps it.
        let roots = dom.parse("<tr><td>a</td></tr><tr><td>b</td></tr>", Some("tbody"));
        let rows: Vec<_> = dom
            .children(roots[0].id)
            .into_iter()
            .filter(|r| r.name.as_deref() == Some("TR"))
            .collect();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn attributes_keep_prefix_and_namespace() {
        let mut dom = DomState::new();
        let roots = dom.parse("<svg><a xlink:href='#x' id='p'></a></svg>", Some(""));
        let wrapper = roots[0].id;
        let svg = child_named(&mut dom, wrapper, "SVG");
        let a = child_named(&mut dom, svg, "A");
        let xlink_href = |value: &str| AttrRecord {
            ns: Some(XLINK.into()),
            prefix: Some("xlink".into()),
            local: "href".into(),
            value: value.into(),
        };
        assert_eq!(dom.attrs(a), vec![xlink_href("#x"), plain("id", "p")]);

        // identity is (namespace, local name): a different prefix replaces the attribute in
        // place, while a no-namespace `href` is a separate attribute
        dom.set_attr(a, Some(XLINK), Some("other"), "href", "#y");
        dom.set_attr(a, None, None, "href", "plain");
        let other_href = AttrRecord {
            prefix: Some("other".into()),
            ..xlink_href("#y")
        };
        assert_eq!(
            dom.attrs(a),
            vec![other_href.clone(), plain("id", "p"), plain("href", "plain")]
        );

        // lookups by qualified name see the prefix; by namespace they do not
        assert_eq!(dom.attr_by_name(a, "other:href"), Some(other_href.clone()));
        assert_eq!(dom.attr_by_name(a, "href"), Some(plain("href", "plain")));
        assert_eq!(dom.attr_by_name(a, "xlink:href"), None);
        assert_eq!(dom.attr(a, Some(XLINK), "href"), Some(other_href));
        assert_eq!(dom.attr(a, None, "nope"), None);

        // a namespaced `id` is not the element's id
        let html = dom.create(1, Some("div"), Some("html"), None);
        dom.insert(svg, html, None);
        dom.set_attr(html, Some(XLINK), Some("xlink"), "id", "ns-id");
        assert_eq!(dom.get_by_id(svg, "ns-id"), None);
        assert_eq!(dom.get_by_id(svg, "p"), Some(a));

        // only elements carry an id
        let text = dom.create(3, Some("t"), None, None);
        dom.insert(svg, text, None);
        dom.set_attr(text, None, None, "id", "on-text");
        assert_eq!(dom.get_by_id(svg, "on-text"), None);

        dom.remove_attr(a, Some(XLINK), "href");
        assert_eq!(dom.attrs(a), vec![plain("id", "p"), plain("href", "plain")]);
        assert_eq!(
            dom.serialize(a, true, true).as_deref(),
            Some(r#"<a id="p" href="plain"></a>"#)
        );
    }

    #[test]
    fn documents_carry_their_html_or_xml_flag() {
        let mut dom = DomState::new();
        let parsed = dom.parse("<p></p>", None)[0].id;
        let html = dom.create_document(true);
        let xml = dom.create_document(false);
        assert_eq!(dom.is_html_document(parsed), Some(true));
        assert_eq!(dom.is_html_document(html), Some(true));
        assert_eq!(dom.is_html_document(xml), Some(false));
        let div = dom.create(1, Some("div"), Some("html"), None);
        assert_eq!(dom.is_html_document(div), None);
    }

    #[test]
    fn create_keeps_name_namespace_and_prefix() {
        let mut dom = DomState::new();
        let fo = dom.create(1, Some("foreignObject"), Some("svg"), None);
        let bare = dom.create(1, Some("Item"), None, Some("x"));
        let el = |dom: &DomState, w: u32| match dom
            .arena
            .get(dom.resolve(w).unwrap())
            .unwrap()
            .value()
        {
            Node::Element(e) => e.name.clone(),
            _ => panic!("not an element"),
        };
        assert_eq!(&*el(&dom, fo).local, "foreignObject");
        let q = el(&dom, bare);
        assert_eq!(
            (&*q.local, q.ns.is_empty(), q.prefix.as_deref()),
            ("Item", true, Some("x"))
        );

        let pi = dom.create(7, Some("xml-stylesheet"), None, None);
        let doctype = dom.create(10, Some("html"), None, None);
        let cdata = dom.create(4, None, None, None);
        let types: Vec<u8> = dom
            .nodes(&[pi, doctype, cdata])
            .iter()
            .map(|r| r.node_type)
            .collect();
        assert_eq!(types, vec![7, 10, 3]);
    }

    #[test]
    fn tree_walks_follow_tree_order() {
        let mut dom = DomState::new();
        // root(a(b, c(d)), e)
        let mk = |dom: &mut DomState| dom.create(1, Some("x"), Some("html"), None);
        let (root, a, b, c, d, e) = (
            mk(&mut dom),
            mk(&mut dom),
            mk(&mut dom),
            mk(&mut dom),
            mk(&mut dom),
            mk(&mut dom),
        );
        dom.insert(root, a, None);
        dom.insert(a, b, None);
        dom.insert(a, c, None);
        dom.insert(c, d, None);
        dom.insert(root, e, None);

        assert_eq!(dom.tree_wires(root), vec![root, a, b, c, d, e]);
        assert_eq!(dom.child_wires(a), vec![b, c]);
        assert_eq!(
            (dom.first_child(a), dom.last_child(a), dom.parent_wire(d)),
            (Some(b), Some(c), Some(c))
        );
        assert_eq!(
            (dom.child_count(root), dom.index(c), dom.index(root)),
            (2, Some(1), None)
        );
        assert_eq!(dom.last_inclusive_descendant(a), Some(d));

        assert_eq!(dom.following(b, None, false), Some(c));
        assert_eq!(dom.following(d, None, false), Some(e));
        assert_eq!(dom.following(a, None, true), Some(e));
        assert_eq!(dom.following(d, Some(a), false), None);
        assert_eq!(dom.preceding(e, None), Some(d));
        assert_eq!(dom.preceding(b, None), Some(a));
        assert_eq!(dom.preceding(a, Some(a)), None);
    }
}
