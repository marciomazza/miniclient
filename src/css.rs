//! stylo's cascade and computed values over the arena, for `getComputedStyle`. `build.rs` never
//! compiles this file: its op sits in the runtime-only `miniclient` extension, so the snapshot
//! build stays free of stylo.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::sync::{LazyLock, Mutex, Once};

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

/// jsdom's window size, so media queries agree with `innerWidth` / `innerHeight`.
const VIEWPORT: (f32, f32) = (1024.0, 768.0);

/// Bounds the parsed-sheet cache, which pages that build `<style>` text from data would grow
/// without limit.
const SHEET_CACHE_SIZE: usize = 256;

// One process-wide lock: stylo asserts a sheet is read through the lock it was created with,
// and parsed sheets are shared by every isolate.
fn lock() -> &'static SharedRwLock {
    &GLOBAL_STYLE_DATA.shared_lock
}

/// `base` resolves the sheet's relative `url()`s; one that does not parse falls back to
/// about:blank, as for a document without a URL.
fn url_data(base: &str) -> UrlExtraData {
    static BLANK: LazyLock<url::Url> =
        LazyLock::new(|| url::Url::parse("about:blank").expect("static URL"));
    UrlExtraData::from(url::Url::parse(base).unwrap_or_else(|_| BLANK.clone()))
}

fn sheet(css: &str, base: &str, origin: Origin, quirks: QuirksMode) -> DocumentStyleSheet {
    DocumentStyleSheet(Arc::new(Stylesheet::from_str(
        css,
        url_data(base),
        origin,
        Arc::new(lock().wrap(MediaList::empty())),
        lock().clone(),
        None,
        None,
        quirks,
        AllowImportRules::Yes,
    )))
}

/// Every page of a test suite tends to ship the same sheets, so they parse once per process.
fn author_sheet(css: &str, base: &str, quirks: QuirksMode) -> DocumentStyleSheet {
    type Key = (String, String, QuirksMode);
    static SHEETS: LazyLock<Mutex<HashMap<Key, DocumentStyleSheet>>> =
        LazyLock::new(Default::default);
    let mut sheets = SHEETS.lock().unwrap_or_else(|e| e.into_inner());
    let key = (css.to_owned(), base.to_owned(), quirks);
    if let Some(sheet) = sheets.get(&key) {
        return sheet.clone();
    }
    // shortcut: dropping the whole cache when full beats LRU bookkeeping on every hit; move to
    // LRU if a page cycles through more distinct sheets than the cache holds.
    if sheets.len() >= SHEET_CACHE_SIZE {
        sheets.clear();
    }
    let parsed = sheet(css, base, Origin::Author, quirks);
    sheets.insert(key, parsed.clone());
    parsed
}

// Parsed once per process.
// shortcut: no-quirks UA sheet for every document, parse a quirks copy once quirks pages matter.
static UA_SHEET: LazyLock<DocumentStyleSheet> = LazyLock::new(|| {
    sheet(
        include_str!("ua.css"),
        "about:blank",
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
        stylo_static_prefs::set_pref!("layout.css.grid-template-subgrid-value.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.anchor-positioning.enabled", true);
        stylo_static_prefs::set_pref!("layout.variable_fonts.enabled", true);
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
    key: (u32, i32, i32),
    quirks: QuirksMode,
    html: bool,
    base: String,
    sheets: Vec<String>,
    stylist: Stylist,
    query: QueryState,
    /// Filled as stylo reaches each element. Boxed so a handed-out `&Extra` stays put while
    /// the map grows.
    extras: RefCell<HashMap<NodeId, Box<Extra>>>,
}

/// The op's state: one style table per document read from.
#[derive(Default)]
pub struct CssState {
    docs: HashMap<NodeId, DocStyle>,
}

fn stylist(quirks: QuirksMode, base: &str, sheets: &[String]) -> Stylist {
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
    let mut seen = std::collections::HashSet::new();
    for css in sheets {
        // stylo takes each sheet once, and a repeat still counts at its own place in the order.
        let sheet = if seen.insert(css) {
            author_sheet(css, base, quirks)
        } else {
            sheet(css, base, Origin::Author, quirks)
        };
        stylist.append_stylesheet(sheet, &guard);
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
    /// Rebuilt whenever anything it read may have changed: the document's change counter
    /// covers structure, attributes and text, and focus/target cover element state. The
    /// stylist survives while the sheets and base URL stay the same.
    // shortcut: whole-table invalidation; stylo snapshots are the upgrade if this shows up in
    // profiles.
    fn build(
        dom: &DomState,
        root: NodeRef<Node>,
        key: (u32, i32, i32),
        base: &str,
        reuse: Option<DocStyle>,
    ) -> Self {
        enable_prefs();
        let html = matches!(root.value(), Node::Document(d) if !d.xml);
        // shortcut: standards mode for every document, as jsdom's cascade did; pass the mode
        // through the op once quirks pages matter.
        let quirks = QuirksMode::NoQuirks;
        let sheets = author_sheets(root);
        let stylist = match reuse {
            Some(old) if old.sheets == sheets && old.base == base => old.stylist,
            _ => stylist(quirks, base, &sheets),
        };
        Self {
            key,
            quirks,
            html,
            base: base.to_owned(),
            sheets,
            stylist,
            query: dom.query_state(html, false, key.1, key.2),
            extras: RefCell::default(),
        }
    }

    fn extra_for(&self, node: NodeRef<Node>, e: &crate::node::Element) -> Extra {
        let style_attr = e.attr("style").map(|css| {
            Arc::new(lock().wrap(parse_style_attribute(
                css,
                &url_data(&self.base),
                None,
                self.quirks,
                CssRuleType::Style,
            )))
        });
        Extra {
            data: ElementDataWrapper::default(),
            flags: Cell::new(ElementSelectorFlags::empty()),
            id: e.id().map(Atom::from),
            style_attr,
            state: element_state(node, &self.query),
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

/// The element's computed style, `None` outside a document. `base` is the document's base
/// URL, which resolves relative `url()`s.
fn style_of(
    css: &mut CssState,
    dom: &DomState,
    wire: u32,
    focus: i32,
    target: i32,
    base: &str,
) -> Option<Arc<ComputedValues>> {
    // shortcut: a released document's table lingers until the next read; hook `release` if
    // pages that never read styles again hold many.
    css.docs.retain(|root, _| dom.doc_version(*root).is_some());
    let node = dom.resolve(wire).and_then(|id| dom.arena.get(id))?;
    let root = node.ancestors().last().unwrap_or(node);
    if !node.value().is_element() {
        return None;
    }
    let key = (dom.doc_version(root.id())?, focus, target);
    let doc = match css.docs.remove(&root.id()) {
        Some(doc) if doc.key == key && doc.base == base => doc,
        old => DocStyle::build(dom, root, key, base, old),
    };
    let style = doc.computed(&dom.arena, node);
    css.docs.insert(root.id(), doc);
    Some(style)
}

/// `getComputedStyle(el).getPropertyValue(p)` for each of `props`; `""` for unknown names and
/// for elements outside a document.
pub fn computed_values(
    css: &mut CssState,
    dom: &DomState,
    wire: u32,
    focus: i32,
    target: i32,
    base: &str,
    props: &[String],
) -> Vec<String> {
    let Some(style) = style_of(css, dom, wire, focus, target, base) else {
        return vec![String::new(); props.len()];
    };
    props
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
        .collect()
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
    #[string] base: &str,
) -> Vec<String> {
    let mut css = state.try_take::<CssState>().unwrap_or_default();
    let values = computed_values(
        &mut css,
        state.borrow::<DomState>(),
        wire,
        focus,
        target,
        base,
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
        let node = self.node();
        let e = node.value().as_element()?;
        let doc = self.doc();
        let mut extras = doc.extras.borrow_mut();
        let extra = extras
            .entry(node.id())
            .or_insert_with(|| Box::new(doc.extra_for(node, e)));
        // SAFETY: entries are boxed and never removed while `doc` lives, so the `Extra` stays
        // put after the borrow ends.
        Some(unsafe { &*(&**extra as *const Extra) })
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
        if let NonTSPseudoClass::Lang(lang) = pc {
            return self.match_element_lang(None, lang);
        }
        let flag = pc.state_flag();
        // shortcut: other stateless pseudo-classes (`:defined` details, custom states) never
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
        self.get_attr(
            &GenericAtomIdent(local_name!("lang")),
            &GenericAtomIdent(ns!(xml)),
        )
        .or_else(|| {
            self.get_attr(
                &GenericAtomIdent(local_name!("lang")),
                &GenericAtomIdent(ns!()),
            )
        })
        .map(|v| AtomString::from(v.as_str()))
    }

    // shortcut: equality or `-` prefix match only, no RFC 4647 `*` wildcards; upgrade when a
    // page uses `:lang("*-CH")`.
    fn match_element_lang(
        &self,
        override_lang: Option<Option<style::selector_parser::AttrValue>>,
        value: &style::selector_parser::Lang,
    ) -> bool {
        let lang = override_lang.unwrap_or_else(|| {
            std::iter::successors(Some(*self), selectors::Element::parent_element)
                .find_map(|e| e.lang_attr())
        });
        lang.is_some_and(|lang| {
            let (lang, value) = (lang.as_ref(), value.as_ref());
            lang.get(..value.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(value))
                && matches!(lang.as_bytes().get(value.len()), None | Some(b'-'))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn element(dom: &mut DomState, root: NodeId, name: &str) -> u32 {
        let hit = dom.arena.get(root).unwrap().descendants().find(|n| {
            n.value()
                .as_element()
                .is_some_and(|e| &*e.name.local == name)
        });
        let id = hit.unwrap().id();
        dom.wire(id)
    }

    fn read(css: &mut CssState, dom: &DomState, wire: u32) -> Arc<ComputedValues> {
        style_of(css, dom, wire, -1, -1, "about:blank").unwrap()
    }

    #[test]
    fn a_change_in_one_document_keeps_anothers_table() {
        let mut dom = DomState::new();
        let a = dom.parse_document("<style>p { color: rgb(1, 2, 3) }</style><p>a</p>");
        let b = dom.parse_document("<p>b</p>");
        let (pa, pb) = (element(&mut dom, a, "p"), element(&mut dom, b, "p"));
        let mut css = CssState::default();
        let color = |css: &mut CssState, dom: &DomState, wire| {
            computed_values(css, dom, wire, -1, -1, "about:blank", &["color".into()]).remove(0)
        };
        assert_eq!(color(&mut css, &dom, pa), "rgb(1, 2, 3)");
        assert_eq!(color(&mut css, &dom, pb), "rgb(0, 0, 0)");

        let before = read(&mut css, &dom, pa);
        dom.set_attr(pb, None, None, "class", "x");
        read(&mut css, &dom, pb);
        assert!(Arc::ptr_eq(&before, &read(&mut css, &dom, pa)));

        dom.set_attr(pa, None, None, "class", "x");
        assert!(!Arc::ptr_eq(&before, &read(&mut css, &dom, pa)));

        let a_wire = dom.wire(a);
        dom.release(&[a_wire]);
        read(&mut css, &dom, pb);
        assert!(!css.docs.contains_key(&a));
    }
}
