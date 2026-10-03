//! Rust-backed DOM tree: the node arena held in `OpState`, answering the `op_dom_*` ops
//! (`src/dom_ops.rs`). Structure, attributes and character data live here.

use std::collections::{HashMap, HashSet};

use ego_tree::{NodeId, Tree};
use html5ever::{Attribute, LocalName, Namespace, Prefix, QualName};
use scraper::node::{Comment, Element, Text};
use scraper::{ElementRef, Html, Node};
use serde::Serialize;

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
#[derive(Serialize, Debug, PartialEq)]
pub struct AttrRecord {
    pub ns: Option<String>,
    pub prefix: Option<String>,
    pub local: String,
    pub value: String,
}

/// One `ego-tree` arena per isolate, held in `OpState`. Every parsed document and detached
/// fragment shares this single id space. Detached nodes keep their slot: the runtime lives for
/// one navigation only, so nothing needs reclaiming.
pub struct DomState {
    pub arena: Tree<Node>,
    // ponytail: side table maps a small wire integer to a NodeId, because ego-tree keeps its
    // own index accessor private. Drop it if ego-tree ever exposes `from_index`.
    ids: Vec<NodeId>,
    lookup: HashMap<NodeId, u32>,
    /// Bumps on every structural change (parse / create / insert / remove).
    pub mutations: u32,
    /// `id` attribute value -> every element bearing it, connected or not. `getElementById`
    /// filters this by document (each candidate's topmost ancestor) at read time instead of
    /// tracking connect/disconnect -- that's exactly what "topmost ancestor" already answers.
    id_index: HashMap<String, Vec<NodeId>>,
    /// `Document` nodes that are XML documents; every other document is HTML. Selector
    /// matching needs this for its case rules.
    xml_documents: HashSet<NodeId>,
}

impl DomState {
    pub fn new() -> Self {
        Self {
            arena: Tree::new(Node::Document),
            ids: Vec::new(),
            lookup: HashMap::new(),
            mutations: 0,
            id_index: HashMap::new(),
            xml_documents: HashSet::new(),
        }
    }

    /// Read access to the arena for the native selector walk.
    pub fn arena(&self) -> &Tree<Node> {
        &self.arena
    }

    /// Parses `html` and folds it into the arena as an orphan subtree. Returns the roots only
    /// (the document or fragment root); descendants come via `children` on demand. `context`
    /// is the fragment context element's tag name (`""` falls back to `<body>`); `None` parses
    /// a full document.
    #[cfg(test)]
    pub fn parse(&mut self, html: &str, context: Option<&str>) -> Vec<NodeRecord> {
        let parsed = match context {
            None => Html::parse_document(html),
            Some(ctx) => parse_fragment(html, if ctx.is_empty() { "body" } else { ctx }),
        };
        let root = self.arena.extend_tree(parsed.tree).id();
        self.mutations += 1;
        self.index_ids_in_subtree(root);
        self.record(root).into_iter().collect()
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
    /// `createDocumentFragment`) as an arena orphan and returns its wire id. `name` / `ns`
    /// are only read for elements; `ns` accepts the short forms `ns_short` emits.
    pub fn create(&mut self, node_type: u8, name: Option<&str>, ns: Option<&str>) -> u32 {
        let node = match node_type {
            3 => Node::Text(Text {
                text: name.unwrap_or_default().into(),
            }),
            8 => Node::Comment(Comment {
                comment: name.unwrap_or_default().into(),
            }),
            11 => Node::Fragment,
            // element -- anything else is a caller bug, treat as element
            _ => {
                let local = LocalName::from(name.unwrap_or_default().to_ascii_lowercase());
                let qual = QualName::new(None, ns_full(ns), local);
                Node::Element(Element::new(qual, Vec::new()))
            }
        };
        let id = self.arena.orphan(node).id();
        self.mutations += 1;
        self.wire(id)
    }

    /// Creates a detached `Document` (JS `createDocument` / `createHTMLDocument` /
    /// `DOMParser`), flagged HTML or XML.
    pub fn create_document(&mut self, html: bool) -> u32 {
        let id = self.arena.orphan(Node::Document).id();
        if !html {
            self.xml_documents.insert(id);
        }
        self.mutations += 1;
        self.wire(id)
    }

    /// Whether `wire` is an HTML document; `None` if it is not a document at all.
    pub fn is_html_document(&self, wire: u32) -> Option<bool> {
        let id = self.resolve(wire)?;
        match self.arena.get(id)?.value() {
            Node::Document => Some(!self.xml_documents.contains(&id)),
            _ => None,
        }
    }

    /// `setAttributeNS`-style write. Identity is the namespace plus local name, as in the DOM
    /// spec: writing over an existing attribute changes its value and keeps its prefix.
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
            match attrs.iter_mut().find(|a| same_attr(&a.name, &ns, local)) {
                Some(a) => a.value = value.into(),
                None => attrs.push(Attribute {
                    name: QualName::new(prefix.map(Prefix::from), ns, LocalName::from(local)),
                    value: value.into(),
                }),
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

    /// Rebuilds the element node from its (mutated) attribute list. `scraper::Element` caches
    /// `id` / `classes` in private `OnceCell`s with no external reset, so a fresh
    /// `Element::new` is the only way to invalidate them after an attribute change.
    fn mutate_attrs(&mut self, wire: u32, edit: impl FnOnce(&mut Vec<Attribute>)) {
        let Some(id) = self.resolve(wire) else { return };
        let Some(mut node) = self.arena.get_mut(id) else {
            return;
        };
        let Node::Element(el) = node.value() else {
            return;
        };
        let mut attrs: Vec<Attribute> = el
            .attrs
            .iter()
            .map(|(k, v)| Attribute {
                name: k.clone(),
                value: v.clone(),
            })
            .collect();
        edit(&mut attrs);
        let name = el.name.clone();
        *node.value() = Node::Element(Element::new(name, attrs));
        self.mutations += 1;
    }

    /// Text / comment `data` write. No-op for anything else.
    pub fn set_data(&mut self, wire: u32, data: &str) {
        let Some(id) = self.resolve(wire) else { return };
        let Some(mut node) = self.arena.get_mut(id) else {
            return;
        };
        match node.value() {
            Node::Text(t) => t.text = data.into(),
            Node::Comment(c) => c.comment = data.into(),
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
                .map(|(k, v)| AttrRecord {
                    ns: (!k.ns.is_empty()).then(|| k.ns.to_string()),
                    prefix: k.prefix.as_ref().map(|p| p.to_string()),
                    local: k.local.to_string(),
                    value: v.to_string(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `outerHTML` of an element node -- html5ever serialization of the arena subtree via
    /// scraper's `ElementRef`. `None` for a non-element or unknown id.
    ///
    /// Also `None` whenever the subtree holds a `<template>` (its content lives in a separate
    /// fragment) or a `ProcessingInstruction` / `Document` / `Fragment` node (`scraper`'s
    /// serializer silently drops these instead of erroring -- see its `_ => ()` catch-all), at
    /// any depth: html5ever's recursion cannot special-case them once started.
    pub fn serialize(&self, wire: u32) -> Option<String> {
        let node = self.arena.get(self.resolve(wire)?)?;
        if node.descendants().any(|n| match n.value() {
            Node::ProcessingInstruction(_) | Node::Document | Node::Fragment => true,
            Node::Element(e) => &e.name.local == "template",
            _ => false,
        }) {
            return None;
        }
        ElementRef::wrap(node).map(|el| el.html())
    }

    /// `data` for a text or comment node; `None` for anything else.
    pub fn data(&self, wire: u32) -> Option<String> {
        let id = self.resolve(wire)?;
        match self.arena.get(id)?.value() {
            Node::Text(t) => Some(t.text.to_string()),
            Node::Comment(c) => Some(c.comment.to_string()),
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
        if let Some(&w) = self.lookup.get(&id) {
            return w;
        }
        let w = self.ids.len() as u32;
        self.ids.push(id);
        self.lookup.insert(id, w);
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

/// Attribute identity per the DOM spec: namespace plus local name, prefix ignored.
fn same_attr(name: &QualName, ns: &Namespace, local: &str) -> bool {
    name.ns == *ns && &*name.local == local
}

fn id_of(e: &Element) -> Option<String> {
    let no_ns = Namespace::from("");
    e.attrs
        .iter()
        .find(|(k, _)| same_attr(k, &no_ns, "id"))
        .map(|(_, v)| v.to_string())
}

/// `scraper::Html::parse_fragment` hardcodes a `<body>` context; this is the same call with
/// the context element's tag threaded through, so `<td>` / `<tr>` fragments land under the
/// right insertion mode instead of being dropped.
fn parse_fragment(html: &str, context: &str) -> Html {
    use html5ever::tendril::TendrilSink;

    let ctx = QualName::new(
        None,
        ns_full(None),
        LocalName::from(context.to_ascii_lowercase()),
    );
    html5ever::driver::parse_fragment(
        scraper::HtmlTreeSink::new(Html::new_fragment()),
        html5ever::driver::ParseOpts::default(),
        ctx,
        Vec::new(),
        false,
    )
    .one(html)
}

fn classify(node: &Node) -> (u8, Option<String>, Option<String>) {
    match node {
        Node::Document => (9, None, None),
        Node::Fragment => (11, None, None),
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
            dom.serialize(div).as_deref(),
            Some("<div id=\"x\">bye<!-- c --></div>")
        );
        assert_eq!(dom.data(kids[1].id).as_deref(), Some(" c "));
        assert_eq!(dom.data(div), None);

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
        let wrapper = dom.children(roots[0].id)[0].id;
        let a = child_named(&mut dom, wrapper, "DIV");

        assert_eq!(
            dom.serialize(a).as_deref(),
            Some(r#"<div id="a" class="b">hi &amp; <span>x</span><br></div>"#),
        );
        // non-element / unknown id -> None
        let text = dom.children(a)[0].id;
        assert_eq!(dom.serialize(text), None);
        assert_eq!(dom.serialize(9999), None);
    }

    #[test]
    fn create_orphans_round_trip_through_classify() {
        let mut dom = DomState::new();

        let div = dom.create(1, Some("div"), None);
        assert!(dom.attrs(div).is_empty());
        let rec = dom.nodes(&[div]).pop().unwrap();
        assert_eq!(rec.node_type, 1);
        assert_eq!(rec.name.as_deref(), Some("DIV"));
        assert_eq!(rec.ns.as_deref(), Some("html"));

        let svg = dom.create(1, Some("svg"), Some("svg"));
        assert_eq!(dom.nodes(&[svg]).pop().unwrap().ns.as_deref(), Some("svg"));

        let text = dom.create(3, Some("hello"), None);
        assert_eq!(dom.data(text).as_deref(), Some("hello"));
        assert_eq!(dom.nodes(&[text]).pop().unwrap().node_type, 3);

        let comment = dom.create(8, Some(" c "), None);
        assert_eq!(dom.data(comment).as_deref(), Some(" c "));

        let frag = dom.create(11, None, None);
        assert_eq!(dom.nodes(&[frag]).pop().unwrap().node_type, 11);

        // orphans: no parent, distinct wire ids
        assert!(dom.parent_wire(div).is_none());
        assert_ne!(div, text);
    }

    #[test]
    fn parse_fragment_has_a_fragment_root() {
        let mut dom = DomState::new();
        let roots = dom.parse("<p>one</p><p>two</p>", Some(""));
        assert_eq!(roots[0].node_type, 11);
        // scraper wraps fragment content in an <html> element under the Fragment root
        let wrapper = dom.children(roots[0].id);
        assert_eq!(wrapper[0].name.as_deref(), Some("HTML"));
        let ps = dom.children(wrapper[0].id);
        assert_eq!(
            ps.iter().filter(|r| r.name.as_deref() == Some("P")).count(),
            2
        );
    }

    #[test]
    fn insert_and_remove_mirror_the_js_tree() {
        let mut dom = DomState::new();
        let roots = dom.parse("<div id='a'></div>", Some(""));
        let wrapper = dom.children(roots[0].id)[0].id;
        let a = child_named(&mut dom, wrapper, "DIV");

        let span = dom.create(1, Some("span"), None);
        let text = dom.create(3, Some("hi"), None);

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
        let b = dom.create(1, Some("b"), None);
        dom.insert(b, span, None);
        assert_eq!(
            dom.children(a).iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![text]
        );
        assert_eq!(dom.parent_wire(span).unwrap(), b);

        // an orphan `before` reference falls back to append rather than panicking
        let orphan = dom.create(1, Some("i"), None);
        let late = dom.create(1, Some("u"), None);
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
        let wrapper = dom.children(roots[0].id)[0].id;
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
    fn fragment_context_keeps_table_rows() {
        let mut dom = DomState::new();
        // <body> context drops a bare <tr>; a <tbody> context keeps it.
        let roots = dom.parse("<tr><td>a</td></tr><tr><td>b</td></tr>", Some("tbody"));
        let wrapper = dom.children(roots[0].id);
        let rows: Vec<_> = dom
            .children(wrapper[0].id)
            .into_iter()
            .filter(|r| r.name.as_deref() == Some("TR"))
            .collect();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn attributes_keep_prefix_and_namespace() {
        let mut dom = DomState::new();
        let roots = dom.parse("<svg><a xlink:href='#x' id='p'></a></svg>", Some(""));
        let wrapper = dom.children(roots[0].id)[0].id;
        let svg = child_named(&mut dom, wrapper, "SVG");
        let a = child_named(&mut dom, svg, "A");
        let xlink_href = |value: &str| AttrRecord {
            ns: Some(XLINK.into()),
            prefix: Some("xlink".into()),
            local: "href".into(),
            value: value.into(),
        };
        assert_eq!(dom.attrs(a), vec![xlink_href("#x"), plain("id", "p")]);

        // identity is (namespace, local name): a different prefix still overwrites the value
        // and keeps the original prefix, while a no-namespace `href` is a separate attribute
        dom.set_attr(a, Some(XLINK), Some("other"), "href", "#y");
        dom.set_attr(a, None, None, "href", "plain");
        assert_eq!(
            dom.attrs(a),
            vec![xlink_href("#y"), plain("id", "p"), plain("href", "plain")]
        );

        // a namespaced `id` is not the element's id
        let html = dom.create(1, Some("div"), None);
        dom.insert(svg, html, None);
        dom.set_attr(html, Some(XLINK), Some("xlink"), "id", "ns-id");
        assert_eq!(dom.get_by_id(svg, "ns-id"), None);
        assert_eq!(dom.get_by_id(svg, "p"), Some(a));

        // only elements carry an id
        let text = dom.create(3, Some("t"), None);
        dom.insert(svg, text, None);
        dom.set_attr(text, None, None, "id", "on-text");
        assert_eq!(dom.get_by_id(svg, "on-text"), None);

        dom.remove_attr(a, Some(XLINK), "href");
        assert_eq!(dom.attrs(a), vec![plain("id", "p"), plain("href", "plain")]);
        assert_eq!(
            dom.serialize(a).as_deref(),
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
        let div = dom.create(1, Some("div"), None);
        assert_eq!(dom.is_html_document(div), None);
    }
}
