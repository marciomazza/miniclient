//! stylo's cascade and computed values over the arena, for `getComputedStyle`. A prototype
//! behind the `stylo-css` feature, with jsdom's own path kept as the oracle. `build.rs` never
//! compiles this file: its op sits in the runtime-only `miniclient` extension, so the snapshot
//! build stays free of stylo.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::sync::{LazyLock, Once};

use deno_core::{OpState, op2};
use ego_tree::{NodeId, NodeRef};
use html5ever::{local_name, ns};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::{BLOOM_HASH_MASK, BloomFilter};
use selectors::matching::{ElementSelectorFlags, MatchingContext, VisitedHandlingMode};
use selectors::sink::Push;
use selectors::{Element as _, OpaqueElement};
use style::applicable_declarations::ApplicableDeclarationBlock;
use style::bloom::each_relevant_element_hash;
use style::context::{
    QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext,
    StyleContext, ThreadLocalStyleContext,
};
use style::data::{ElementDataMut, ElementDataRef, ElementDataWrapper};
use style::device::Device;
use style::device::servo::FontMetricsProvider;
use style::dom::{LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot};
use style::font_metrics::FontMetrics;
use style::global_style_data::GLOBAL_STYLE_DATA;
use style::media_queries::{MediaList, MediaType};
use style::properties::style_structs::Font;
use style::properties::{
    ComputedValues, PropertyDeclarationBlock, PropertyDeclarationId, PropertyId, ShorthandId,
    parse_style_attribute,
};
use style::queries::values::PrefersColorScheme;
use style::selector_parser::{NonTSPseudoClass, PseudoElement, SelectorImpl, SnapshotMap};
use style::servo::media_features::PointerCapabilities;
use style::servo_arc::{Arc, ArcBorrow};
use style::shared_lock::{Locked, SharedRwLock, StylesheetGuards};
use style::stylesheets::{
    AllowImportRules, CssRuleType, DocumentStyleSheet, Origin, Stylesheet, UrlExtraData,
};
use style::stylist::{RuleInclusion, Stylist};
use style::thread_state::{self, ThreadState};
use style::traversal::resolve_style;
use style::traversal_flags::TraversalFlags;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{CSSPixelLength, Length};
use style::values::resolved;
use style::values::{AtomIdent, AtomString, GenericAtomIdent};
use style::{Atom, CaseSensitivityExt, LocalName, Namespace};
use stylo_dom::ElementState;

use crate::dom::DomState;
use crate::dom_query::{MiniElement, QueryState, STATE_PSEUDO_CLASSES, StatePseudo};
use crate::node::Node;

/// jsdom's window size, so media queries agree with the oracle.
const VIEWPORT: (f32, f32) = (1024.0, 768.0);

// One process-wide lock: stylo asserts a sheet is read through the lock it was created with,
// and the UA sheet is shared by every isolate.
fn lock() -> &'static SharedRwLock {
    &GLOBAL_STYLE_DATA.shared_lock
}

fn url_data() -> UrlExtraData {
    static URL: LazyLock<url::Url> =
        LazyLock::new(|| url::Url::parse("about:blank").expect("static URL"));
    UrlExtraData::from(URL.clone())
}

fn sheet(css: &str, origin: Origin, quirks: QuirksMode) -> DocumentStyleSheet {
    DocumentStyleSheet(Arc::new(Stylesheet::from_str(
        css,
        url_data(),
        origin,
        Arc::new(lock().wrap(MediaList::empty())),
        lock().clone(),
        None,
        None,
        quirks,
        AllowImportRules::Yes,
    )))
}

// Parsed once per process; jsdom's own UA sheet so both paths start from the same defaults.
// shortcut: no-quirks UA sheet for every document, parse a quirks copy once quirks pages matter.
static UA_SHEET: LazyLock<DocumentStyleSheet> = LazyLock::new(|| {
    sheet(
        include_str!("../vendor/jsdom/lib/jsdom/browser/default-stylesheet.css"),
        Origin::UserAgent,
        QuirksMode::NoQuirks,
    )
});

// Stylo gates selectors and properties browsers ship behind prefs read at parse time, so they
// are set before the first sheet; no thread pool, as styling runs on the isolate's thread.
fn enable_prefs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        stylo_static_prefs::set_pref!("layout.threads", 0);
        stylo_static_prefs::set_pref!("layout.css.has-selector.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.nth-child-of.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.at-scope.enabled", true);
        stylo_static_prefs::set_pref!("layout.container-queries.enabled", true);
        stylo_static_prefs::set_pref!("layout.columns.enabled", true);
        stylo_static_prefs::set_pref!("layout.grid.enabled", true);
        stylo_static_prefs::set_pref!("layout.writing-mode.enabled", true);
        stylo_static_prefs::set_pref!("layout.unimplemented", true);
    });
}

#[derive(Debug)]
struct NoFonts;

// shortcut: no font backend, so `ex`/`ch` units fall back to stylo's defaults; plug in real
// metrics with the layout seam.
impl FontMetricsProvider for NoFonts {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        _font: &Font,
        _base_size: CSSPixelLength,
        _flags: style::values::computed::font::QueryFontMetricsFlags,
    ) -> FontMetrics {
        FontMetrics::default()
    }

    fn base_size_for_generic(&self, generic: GenericFontFamily) -> Length {
        Length::new(if generic == GenericFontFamily::Monospace {
            13.0
        } else {
            16.0
        })
    }
}

struct NoPainters;

impl RegisteredSpeculativePainters for NoPainters {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

/// What stylo needs per element that the arena doesn't hold in its types.
struct Extra {
    data: ElementDataWrapper,
    flags: Cell<ElementSelectorFlags>,
    id: Option<Atom>,
    style_attr: Option<Arc<Locked<PropertyDeclarationBlock>>>,
    state: ElementState,
}

/// One document's styling inputs and results.
pub struct DocStyle {
    root: NodeId,
    key: (u32, i32, i32),
    quirks: QuirksMode,
    html: bool,
    sheets: Vec<String>,
    stylist: Stylist,
    extras: HashMap<NodeId, Extra>,
}

/// The op's state: the last document styled.
// shortcut: one document cached at a time; key by document once iframes read styles.
#[derive(Default)]
pub struct CssState {
    doc: Option<DocStyle>,
}

fn stylist(quirks: QuirksMode, sheets: &[String]) -> Stylist {
    let device = Device::new(
        MediaType::screen(),
        quirks,
        euclid::Size2D::new(VIEWPORT.0, VIEWPORT.1),
        euclid::Size2D::new(VIEWPORT.0, VIEWPORT.1),
        euclid::Scale::new(1.0),
        Box::new(NoFonts),
        ComputedValues::initial_values_with_font_override(Font::initial_values()),
        PrefersColorScheme::Light,
        PointerCapabilities::default(),
        PointerCapabilities::default(),
    );
    let mut stylist = Stylist::new(device, quirks);
    let guard = lock().read();
    stylist.append_stylesheet(UA_SHEET.clone(), &guard);
    for css in sheets {
        stylist.append_stylesheet(sheet(css, Origin::Author, quirks), &guard);
    }
    let guards = StylesheetGuards::same(&guard);
    let _ = stylist.flush(&guards);
    stylist
}

fn is_html(node: NodeRef<Node>, local: &LocalName) -> bool {
    node.value()
        .as_element()
        .is_some_and(|e| e.name.ns == ns!(html) && e.name.local == **local)
}

fn text_of(node: NodeRef<Node>) -> String {
    node.children()
        .filter_map(|c| match c.value() {
            Node::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect()
}

// shortcut: inline `<style>` only, no `media`/`type`/`disabled` filtering and no `<link>`;
// those come with the CSSOM step.
fn author_sheets(root: NodeRef<Node>) -> Vec<String> {
    root.descendants()
        .filter(|n| is_html(*n, &LocalName::from("style")))
        .map(text_of)
        .collect()
}

fn element_state(node: NodeRef<Node>, query: &QueryState) -> ElementState {
    let Some(el) = MiniElement::wrap(node, query) else {
        return ElementState::empty();
    };
    let mut state = ElementState::DEFINED;
    for (_, pc) in STATE_PSEUDO_CLASSES {
        if !el.in_state(pc) {
            continue;
        }
        state |= match pc {
            StatePseudo::AnyLink | StatePseudo::Link => ElementState::UNVISITED,
            StatePseudo::Checked => ElementState::CHECKED,
            StatePseudo::Disabled => ElementState::DISABLED,
            StatePseudo::Enabled => ElementState::ENABLED,
            StatePseudo::Focus => ElementState::FOCUS,
            StatePseudo::FocusWithin => ElementState::FOCUS_WITHIN,
            StatePseudo::Optional => ElementState::OPTIONAL_,
            StatePseudo::ReadOnly => ElementState::READONLY,
            StatePseudo::ReadWrite => ElementState::READWRITE,
            StatePseudo::Required => ElementState::REQUIRED,
            StatePseudo::Target => ElementState::URLTARGET,
        };
    }
    state
}

impl DocStyle {
    /// Rebuilt from scratch whenever anything it read may have changed: the arena's mutation
    /// counter covers structure, attributes and text, and focus/target cover element state.
    // shortcut: whole-table invalidation; stylo snapshots are the upgrade if this shows up in
    // profiles.
    fn build(
        dom: &DomState,
        root: NodeRef<Node>,
        key: (u32, i32, i32),
        reuse: Option<DocStyle>,
    ) -> Self {
        enable_prefs();
        let html = matches!(root.value(), Node::Document(d) if !d.xml);
        let query = dom.query_state(html, false, key.1, key.2);
        let quirks = QuirksMode::NoQuirks;
        let sheets = author_sheets(root);
        let stylist = match reuse {
            Some(old) if old.root == root.id() && old.sheets == sheets => old.stylist,
            _ => stylist(quirks, &sheets),
        };
        let extras = root
            .descendants()
            .filter_map(|n| {
                let e = n.value().as_element()?;
                let style_attr = e.attr("style").map(|css| {
                    Arc::new(lock().wrap(parse_style_attribute(
                        css,
                        &url_data(),
                        None,
                        quirks,
                        CssRuleType::Style,
                    )))
                });
                let extra = Extra {
                    data: ElementDataWrapper::default(),
                    flags: Cell::new(ElementSelectorFlags::empty()),
                    id: e.id().map(Atom::from),
                    style_attr,
                    state: element_state(n, &query),
                };
                Some((n.id(), extra))
            })
            .collect();
        Self {
            root: root.id(),
            key,
            quirks,
            html,
            sheets,
            stylist,
            extras,
        }
    }

    fn computed(&self, arena: &ego_tree::Tree<Node>, node: NodeRef<Node>) -> Arc<ComputedValues> {
        let ctx = Ctx { arena, doc: self };
        with_ctx(&ctx, || self.resolve(N::of(node)))
    }

    fn resolve(&self, el: N) -> Arc<ComputedValues> {
        if let Some(style) = el
            .borrow_data()
            .and_then(|d| d.styles.get_primary().cloned())
        {
            return style;
        }
        let guard = lock().read();
        let shared = SharedStyleContext {
            traversal_flags: TraversalFlags::empty(),
            stylist: &self.stylist,
            options: GLOBAL_STYLE_DATA.options.clone(),
            guards: StylesheetGuards::same(&guard),
            visited_styles_enabled: false,
            animations: Default::default(),
            current_time_for_animations: 0.0,
            snapshot_map: &SnapshotMap::new(),
            registered_speculative_painters: &NoPainters,
        };
        thread_state::enter(ThreadState::LAYOUT);
        let styles = {
            let mut thread_local = ThreadLocalStyleContext::new();
            let mut context = StyleContext {
                shared: &shared,
                thread_local: &mut thread_local,
            };
            resolve_style(&mut context, el, RuleInclusion::All, None, None)
        };
        thread_state::exit(ThreadState::LAYOUT);
        let primary = styles.primary().clone();
        // Kept so later reads of this element and its descendants stop the ancestor walk here.
        if let Some(mut data) = el.mutate_data() {
            data.styles.primary = Some(primary.clone());
        }
        primary
    }
}

/// `getComputedStyle(el).getPropertyValue(p)` for each of `props`; `""` for shorthands and
/// unknown names, and for elements outside a document.
pub fn computed_values(
    css: &mut CssState,
    dom: &DomState,
    wire: u32,
    focus: i32,
    target: i32,
    props: &[String],
) -> Vec<String> {
    let blank = || vec![String::new(); props.len()];
    let Some(node) = dom.resolve(wire).and_then(|id| dom.arena.get(id)) else {
        return blank();
    };
    let root = node.ancestors().last().unwrap_or(node);
    if !node.value().is_element() || !root.value().is_document() {
        return blank();
    }
    let key = (dom.mutations, focus, target);
    let doc = match css.doc.take() {
        Some(doc) if doc.root == root.id() && doc.key == key => doc,
        old => DocStyle::build(dom, root, key, old),
    };
    let style = doc.computed(&dom.arena, node);
    let values = props
        .iter()
        .map(
            |name| match PropertyId::parse_enabled_for_all_content(name) {
                Ok(PropertyId::Custom(name)) => {
                    style.computed_value_to_string(PropertyDeclarationId::Custom(&name))
                }
                Ok(PropertyId::NonCustom(id)) => match id.longhand_or_shorthand() {
                    Ok(id) => style.computed_value_to_string(PropertyDeclarationId::Longhand(id)),
                    Err(id) => shorthand_value(&style, id),
                },
                Err(()) => String::new(),
            },
        )
        .collect();
    css.doc = Some(doc);
    values
}

/// As Servo does it: the shorthand serialized from its resolved longhands, "" when they
/// cannot be expressed as it.
fn shorthand_value(style: &ComputedValues, id: ShorthandId) -> String {
    let declarations: Vec<_> = id
        .longhands()
        .map(|longhand| {
            let mut context = resolved::Context {
                style,
                for_property: PropertyId::NonCustom(id.into()),
                current_longhand: Some(longhand),
            };
            style.computed_or_resolved_declaration(longhand, Some(&mut context))
        })
        .collect();
    let mut s = String::new();
    match id.longhands_to_css(&declarations.iter().collect::<Vec<_>>(), &mut s) {
        Ok(()) => s,
        Err(_) => String::new(),
    }
}

/// `focus` and `target` are wires, `-1` for none, as for `op_dom_query`.
#[op2]
#[serde]
pub fn op_css_computed_values(
    state: &mut OpState,
    wire: u32,
    focus: i32,
    target: i32,
    #[serde] props: Vec<String>,
) -> Vec<String> {
    let mut css = state.try_take::<CssState>().unwrap_or_default();
    let values = computed_values(
        &mut css,
        state.borrow::<DomState>(),
        wire,
        focus,
        target,
        &props,
    );
    state.put(css);
    values
}

/// What a handle reads through while stylo runs.
struct Ctx<'a> {
    arena: &'a ego_tree::Tree<Node>,
    doc: &'a DocStyle,
}

thread_local! {
    static CTX: Cell<*const ()> = const { Cell::new(std::ptr::null()) };
}

/// Runs `f` with `ctx` readable by every handle. stylo's sharing cache transmutes element handles
/// to `usize`, so a handle is one `NodeId` and finds the arena here instead of carrying it.
fn with_ctx<R>(ctx: &Ctx, f: impl FnOnce() -> R) -> R {
    struct Reset(*const ());
    impl Drop for Reset {
        fn drop(&mut self) {
            CTX.set(self.0);
        }
    }
    let _reset = Reset(CTX.replace(ctx as *const Ctx as *const ()));
    f()
}

/// A node handle stylo's traits are implemented on; the same type plays node, element and
/// document, as in Blitz.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct N<'a> {
    id: NodeId,
    _ctx: std::marker::PhantomData<&'a ()>,
}

impl fmt::Debug for N<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "N({:?})", self.id)
    }
}

impl<'a> N<'a> {
    fn of(node: NodeRef<'a, Node>) -> Self {
        Self {
            id: node.id(),
            _ctx: std::marker::PhantomData,
        }
    }

    fn ctx(&self) -> &'a Ctx<'a> {
        let ctx = CTX.get();
        assert!(!ctx.is_null(), "style handle used outside with_ctx");
        // SAFETY: handles are only created inside `with_ctx`, which keeps `ctx` alive and set
        // for as long as stylo runs; stylo keeps no handle past `resolve_style`.
        unsafe { &*(ctx as *const Ctx<'a>) }
    }

    fn node(&self) -> NodeRef<'a, Node> {
        self.ctx()
            .arena
            .get(self.id)
            .expect("handle to a live node")
    }

    fn doc(&self) -> &'a DocStyle {
        self.ctx().doc
    }

    fn with(&self, node: NodeRef<'a, Node>) -> Self {
        Self::of(node)
    }

    fn el(&self) -> &'a crate::node::Element {
        self.node().value().as_element().expect("element handle")
    }

    fn extra(&self) -> Option<&'a Extra> {
        self.doc().extras.get(&self.node().id())
    }

    fn addr(&self) -> usize {
        self.node().value() as *const Node as usize
    }

    fn element_sibling(&self, next: bool) -> Option<Self> {
        let mut n = self.node();
        loop {
            n = if next {
                n.next_sibling()
            } else {
                n.prev_sibling()
            }?;
            if n.value().is_element() {
                return Some(self.with(n));
            }
        }
    }

    fn classes(&self) -> impl Iterator<Item = &'a str> {
        self.el()
            .attr("class")
            .unwrap_or_default()
            .split_ascii_whitespace()
    }
}

impl NodeInfo for N<'_> {
    fn is_element(&self) -> bool {
        self.node().value().is_element()
    }

    fn is_text_node(&self) -> bool {
        self.node().value().is_text()
    }
}

impl<'a> TDocument for N<'a> {
    type ConcreteNode = N<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn is_html_document(&self) -> bool {
        self.doc().html
    }

    fn quirks_mode(&self) -> QuirksMode {
        self.doc().quirks
    }

    fn shared_lock(&self) -> &SharedRwLock {
        lock()
    }
}

impl<'a> TShadowRoot for N<'a> {
    type ConcreteNode = N<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn host(&self) -> N<'a> {
        unreachable!("the arena has no shadow roots")
    }

    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        None
    }
}

impl<'a> TNode for N<'a> {
    type ConcreteElement = N<'a>;
    type ConcreteDocument = N<'a>;
    type ConcreteShadowRoot = N<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.node().parent().map(|n| self.with(n))
    }

    fn first_child(&self) -> Option<Self> {
        self.node().first_child().map(|n| self.with(n))
    }

    fn last_child(&self) -> Option<Self> {
        self.node().last_child().map(|n| self.with(n))
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.node().prev_sibling().map(|n| self.with(n))
    }

    fn next_sibling(&self) -> Option<Self> {
        self.node().next_sibling().map(|n| self.with(n))
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        self.with(self.node().ancestors().last().unwrap_or(self.node()))
    }

    fn is_in_document(&self) -> bool {
        true
    }

    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        self.parent_node().filter(|p| p.is_element())
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.addr())
    }

    fn debug_id(self) -> usize {
        self.addr()
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        self.is_element().then_some(*self)
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        self.node().value().is_document().then_some(*self)
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        None
    }
}

impl selectors::Element for N<'_> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self.node().value())
    }

    fn parent_element(&self) -> Option<Self> {
        TNode::traversal_parent(self)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.element_sibling(false)
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.element_sibling(true)
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node()
            .children()
            .find(|c| c.value().is_element())
            .map(|n| self.with(n))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.doc().html && self.el().name.ns == ns!(html)
    }

    fn has_local_name(&self, local_name: &html5ever::LocalName) -> bool {
        self.el().name.local == *local_name
    }

    fn has_namespace(&self, ns: &html5ever::Namespace) -> bool {
        self.el().name.ns == *ns
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.el().name == other.el().name
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&GenericAtomIdent<html5ever::NamespaceStaticSet>>,
        local_name: &GenericAtomIdent<html5ever::LocalNameStaticSet>,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        self.el().attrs.iter().any(|attr| {
            attr.name.local == local_name.0
                && match ns {
                    NamespaceConstraint::Any => true,
                    NamespaceConstraint::Specific(ns) => attr.name.ns == ns.0,
                }
                && operation.eval_str(&attr.value)
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        let flag = pc.state_flag();
        // shortcut: stateless pseudo-classes (`:lang`, `:defined` details, custom states) never
        // match; mini's own matcher covers them for queries.
        !flag.is_empty() && self.state().intersects(flag)
    }

    fn match_pseudo_element(
        &self,
        _pe: &PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        let set = |n: &Self, f: ElementSelectorFlags| {
            if let Some(extra) = n.extra() {
                extra.flags.set(extra.flags.get() | f);
            }
        };
        set(self, flags.for_self());
        if let Some(parent) = TNode::traversal_parent(self) {
            set(&parent, flags.for_parent());
        }
    }

    fn is_link(&self) -> bool {
        self.state().intersects(ElementState::VISITED_OR_UNVISITED)
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        TElement::id(self).is_some_and(|own| case_sensitivity.eq_atom(own, id))
    }

    fn has_class(&self, name: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        self.classes()
            .any(|c| case_sensitivity.eq_atom(&Atom::from(c), name))
    }

    fn imported_part(&self, _name: &AtomIdent) -> Option<AtomIdent> {
        None
    }

    fn is_part(&self, _name: &AtomIdent) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.node().children().all(|c| match c.value() {
            Node::Element(_) => false,
            Node::Text(t) => t.text.is_empty(),
            _ => true,
        })
    }

    fn is_root(&self) -> bool {
        self.node()
            .parent()
            .is_some_and(|p| p.value().is_document())
    }

    fn has_custom_state(&self, _name: &AtomIdent) -> bool {
        false
    }

    fn add_element_unique_hashes(&self, filter: &mut BloomFilter) -> bool {
        each_relevant_element_hash(*self, |hash| filter.insert_hash(hash & BLOOM_HASH_MASK));
        true
    }
}

pub struct Children<'a> {
    parent: N<'a>,
    next: Option<NodeRef<'a, Node>>,
}

impl<'a> Iterator for Children<'a> {
    type Item = N<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.next?;
        self.next = node.next_sibling();
        Some(self.parent.with(node))
    }
}

impl<'a> TElement for N<'a> {
    type ConcreteNode = N<'a>;
    type TraversalChildrenIterator = Children<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn traversal_children(&self) -> LayoutIterator<Self::TraversalChildrenIterator> {
        LayoutIterator(Children {
            parent: *self,
            next: self.node().first_child(),
        })
    }

    fn is_html_element(&self) -> bool {
        self.el().name.ns == ns!(html)
    }

    fn is_mathml_element(&self) -> bool {
        self.el().name.ns == ns!(mathml)
    }

    fn is_svg_element(&self) -> bool {
        self.el().name.ns == ns!(svg)
    }

    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.extra()?.style_attr.as_ref().map(|a| a.borrow_arc())
    }

    fn state(&self) -> ElementState {
        self.extra().map_or(ElementState::empty(), |e| e.state)
    }

    fn has_part_attr(&self) -> bool {
        false
    }

    fn exports_any_part(&self) -> bool {
        false
    }

    fn id(&self) -> Option<&Atom> {
        self.extra()?.id.as_ref()
    }

    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        for c in self.classes() {
            callback(AtomIdent::cast(&Atom::from(c)));
        }
    }

    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&LocalName),
    {
        for attr in &self.el().attrs {
            callback(&GenericAtomIdent(attr.name.local.clone()));
        }
    }

    fn has_dirty_descendants(&self) -> bool {
        false
    }

    fn has_snapshot(&self) -> bool {
        false
    }

    fn handled_snapshot(&self) -> bool {
        true
    }

    unsafe fn set_handled_snapshot(&self) {}

    unsafe fn set_dirty_descendants(&self) {}

    unsafe fn unset_dirty_descendants(&self) {}

    fn store_children_to_process(&self, _n: isize) {
        unreachable!("parallel traversal only")
    }

    fn did_process_child(&self) -> isize {
        unreachable!("parallel traversal only")
    }

    unsafe fn ensure_data(&self) -> ElementDataMut<'_> {
        self.extra()
            .expect("element in a styled document")
            .data
            .borrow_mut()
    }

    unsafe fn clear_data(&self) {}

    fn has_data(&self) -> bool {
        self.extra().is_some()
    }

    fn borrow_data(&self) -> Option<ElementDataRef<'_>> {
        self.extra().map(|e| e.data.borrow())
    }

    fn mutate_data(&self) -> Option<ElementDataMut<'_>> {
        self.extra().map(|e| e.data.borrow_mut())
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        false
    }

    fn has_animations(&self, _context: &SharedStyleContext) -> bool {
        false
    }

    fn has_css_animations(
        &self,
        _context: &SharedStyleContext,
        _pseudo_element: Option<PseudoElement>,
    ) -> bool {
        false
    }

    fn has_css_transitions(
        &self,
        _context: &SharedStyleContext,
        _pseudo_element: Option<PseudoElement>,
    ) -> bool {
        false
    }

    fn animation_rule(
        &self,
        _context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn transition_rule(
        &self,
        _context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        None
    }

    fn get_attr(&self, attr: &LocalName, ns: &Namespace) -> Option<String> {
        self.el()
            .attrs
            .iter()
            .find(|a| a.name.local == **attr && a.name.ns == **ns)
            .map(|a| a.value.to_string())
    }

    fn lang_attr(&self) -> Option<style::selector_parser::AttrValue> {
        None
    }

    fn match_element_lang(
        &self,
        _override_lang: Option<Option<style::selector_parser::AttrValue>>,
        _value: &style::selector_parser::Lang,
    ) -> bool {
        false
    }

    fn is_html_document_body_element(&self) -> bool {
        self.el().name.ns == ns!(html)
            && self.el().name.local == local_name!("body")
            && self
                .node()
                .parent()
                .is_some_and(|p| N::is_root(&self.with(p)))
    }

    // shortcut: no presentational hints (`bgcolor`, `width=`, `hidden` is in the UA sheet);
    // port Blitz's table when the corpus shows them.
    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _visited_handling: VisitedHandlingMode,
        _hints: &mut V,
    ) where
        V: Push<ApplicableDeclarationBlock>,
    {
    }

    fn local_name(&self) -> &html5ever::LocalName {
        &self.el().name.local
    }

    fn namespace(&self) -> &html5ever::Namespace {
        &self.el().name.ns
    }

    fn query_container_size(
        &self,
        _display: &style::values::specified::Display,
    ) -> euclid::default::Size2D<Option<app_units::Au>> {
        Default::default()
    }

    fn each_custom_state<F>(&self, _callback: F)
    where
        F: FnMut(&AtomIdent),
    {
    }

    fn has_selector_flags(&self, flags: ElementSelectorFlags) -> bool {
        self.extra().is_some_and(|e| e.flags.get().contains(flags))
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        let flags = self
            .extra()
            .map_or(ElementSelectorFlags::empty(), |e| e.flags.get());
        [
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING,
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR,
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING,
        ]
        .into_iter()
        .find(|f| flags.contains(*f))
        .unwrap_or_else(ElementSelectorFlags::empty)
    }
}
