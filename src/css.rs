//! stylo's cascade and computed values over the arena, for `getComputedStyle`. `build.rs` never
//! compiles this file: its op sits in the runtime-only `miniclient` extension, so the snapshot
//! build stays free of stylo.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::sync::{LazyLock, Mutex, RwLock};

use cssparser::{Parser, SourceLocation};
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
use style::parser::ParserContext;
use style::properties::style_structs::Font;
use style::properties::{
    ComputedValues, Importance, PropertyDeclarationBlock, PropertyDeclarationId, PropertyId,
    ShorthandId, SourcePropertyDeclaration, SourcePropertyDeclarationUpdate,
    parse_one_declaration_into, parse_style_attribute,
};
use style::queries::values::PrefersColorScheme;
use style::selector_parser::{NonTSPseudoClass, PseudoElement, SelectorImpl, SnapshotMap};
use style::servo::media_features::PointerCapabilities;
use style::servo_arc::{Arc, ArcBorrow};
use style::shared_lock::{Locked, SharedRwLock, StylesheetGuards};
use style::stylesheets::import_rule::{ImportLayer, ImportSheet, ImportSupportsCondition};
use style::stylesheets::supports_rule::parse_condition_or_declaration;
use style::stylesheets::{
    AllowImportRules, CssRule, CssRuleType, DocumentStyleSheet, ImportRule, Origin, Stylesheet,
    StylesheetLoader, UrlExtraData,
};
use style::stylist::{RuleInclusion, Stylist};
use style::thread_state::{self, ThreadState};
use style::traversal::resolve_style;
use style::traversal_flags::TraversalFlags;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{CSSPixelLength, Length};
use style::values::resolved;
use style::values::{AtomIdent, AtomString, CssUrl, GenericAtomIdent};
use style::{Atom, CaseSensitivityExt, LocalName, Namespace};
use style_traits::ParsingMode;
use stylo_dom::ElementState;

use crate::dom::DomState;
pub(crate) use crate::dom_query::enable_prefs;
use crate::dom_query::{FORM_FLAGS, MiniElement, QueryState};
use crate::node::Node;

/// jsdom's window size, so media queries agree with `innerWidth` / `innerHeight`.
const VIEWPORT: (f32, f32) = (1024.0, 768.0);

/// Bounds the parsed-sheet cache, which pages that build `<style>` text from data would grow
/// without limit.
const SHEET_CACHE_SIZE: usize = 256;

// One process-wide lock: stylo asserts a sheet is read through the lock it was created with,
// and parsed sheets are shared by every isolate.
pub(crate) fn lock() -> &'static SharedRwLock {
    &GLOBAL_STYLE_DATA.shared_lock
}

// That lock panics instead of waiting when isolates on other threads collide, so op entry points
// take this one first: shared to compute styles, exclusive to write sheets.
pub(crate) static GATE: RwLock<()> = RwLock::new(());

/// `base` resolves the sheet's relative `url()`s; one that does not parse falls back to
/// about:blank, as for a document without a URL.
pub(crate) fn url_data(base: &str) -> UrlExtraData {
    static BLANK: LazyLock<url::Url> =
        LazyLock::new(|| url::Url::parse("about:blank").expect("static URL"));
    UrlExtraData::from(url::Url::parse(base).unwrap_or_else(|_| BLANK.clone()))
}

/// One author sheet: its text, the URL its relative `url()`s and `@import`s resolve against,
/// and its owner's `media` attribute.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Source {
    pub(crate) css: String,
    pub(crate) base: String,
    pub(crate) media: String,
}

fn author_context(
    base: &UrlExtraData,
    rule_type: CssRuleType,
    quirks: QuirksMode,
) -> ParserContext<'_> {
    ParserContext::new(
        Origin::Author,
        base,
        Some(rule_type),
        ParsingMode::DEFAULT,
        quirks,
        Default::default(),
        None,
        None,
        Default::default(),
    )
}

pub(crate) fn media_list(media: &str, base: &UrlExtraData, quirks: QuirksMode) -> MediaList {
    let mut context = author_context(base, CssRuleType::Media, quirks);
    MediaList::parse(&mut context, &mut Parser::new(media))
}

/// Answers `@import` from the bodies the page already fetched, so parsing stays synchronous. An
/// import not fetched yet, or one that would import itself again, gets an empty sheet that keeps
/// its media; the fetch fills it in place, so the rule objects the page holds stay valid.
pub(crate) struct ImportLoader<'a> {
    pub(crate) imports: &'a BTreeMap<String, String>,
    pub(crate) quirks: QuirksMode,
    /// URLs of the sheets being parsed, outermost first.
    pub(crate) chain: RefCell<Vec<String>>,
}

impl StylesheetLoader for ImportLoader<'_> {
    fn request_stylesheet(
        &self,
        url: CssUrl,
        location: SourceLocation,
        lock: &SharedRwLock,
        media: Arc<Locked<MediaList>>,
        supports: Option<ImportSupportsCondition>,
        layer: ImportLayer,
    ) -> Arc<Locked<ImportRule>> {
        let href = url.url().map(|u| u.as_str().to_owned());
        let css = href
            .as_ref()
            .filter(|href| !self.chain.borrow().contains(href))
            .and_then(|href| self.imports.get(href));
        let stylesheet = if supports.as_ref().is_some_and(|s| !s.enabled) {
            ImportSheet::new_refused()
        } else {
            let href = href.unwrap_or_else(|| "about:blank".to_owned());
            self.chain.borrow_mut().push(href.clone());
            let sheet = Stylesheet::from_str(
                &preprocess(css.map_or("", String::as_str)),
                url_data(&href),
                Origin::Author,
                media,
                lock.clone(),
                Some(self),
                None,
                self.quirks,
                AllowImportRules::Yes,
            );
            self.chain.borrow_mut().pop();
            ImportSheet::new(Arc::new(sheet))
        };
        Arc::new(lock.wrap(ImportRule {
            url,
            stylesheet,
            supports,
            layer,
            source_location: location,
        }))
    }
}

/// `src` parsed, with `@import` answered from `imports`; a constructed sheet allows none.
pub(crate) fn sheet(
    src: &Source,
    origin: Origin,
    quirks: QuirksMode,
    imports: &BTreeMap<String, String>,
    allow_imports: AllowImportRules,
) -> DocumentStyleSheet {
    // Pref-gated rules such as `@scope` are dropped at parse time, before any cascade.
    enable_prefs();
    let base = url_data(&src.base);
    let media = media_list(&src.media, &base, quirks);
    let loader = ImportLoader {
        imports,
        quirks,
        chain: RefCell::new(vec![src.base.clone()]),
    };
    DocumentStyleSheet(Arc::new(Stylesheet::from_str(
        &preprocess(&src.css),
        base,
        origin,
        Arc::new(lock().wrap(media)),
        lock().clone(),
        Some(&loader),
        None,
        quirks,
        allow_imports,
    )))
}

fn has_imports(sheet: &Stylesheet) -> bool {
    let guard = lock().read();
    let contents = sheet.contents.read_with(&guard);
    contents
        .rules(&guard)
        .iter()
        .any(|r| matches!(r, CssRule::Import(_)))
}

/// Every page of a test suite tends to ship the same sheets, so they parse once per process.
/// The flag tells whether the sheet came from the cache, so is shared and copied before a write.
/// A sheet with `@import` is never shared, as fetches fill its imports in place.
pub(crate) fn author_sheet(
    src: &Source,
    quirks: QuirksMode,
    imports: &BTreeMap<String, String>,
) -> (DocumentStyleSheet, bool) {
    type Key = (Source, QuirksMode);
    static SHEETS: LazyLock<Mutex<HashMap<Key, DocumentStyleSheet>>> =
        LazyLock::new(Default::default);
    let mut sheets = SHEETS.lock().unwrap_or_else(|e| e.into_inner());
    let key = (src.clone(), quirks);
    if let Some(sheet) = sheets.get(&key) {
        return (sheet.clone(), true);
    }
    let parsed = sheet(src, Origin::Author, quirks, imports, AllowImportRules::Yes);
    if has_imports(&parsed.0) {
        return (parsed, false);
    }
    // shortcut: dropping the whole cache when full beats LRU bookkeeping on every hit, move to
    // LRU if a page cycles through more distinct sheets than the cache holds.
    if sheets.len() >= SHEET_CACHE_SIZE {
        sheets.clear();
    }
    sheets.insert(key, parsed.clone());
    (parsed, true)
}

// Parsed once per process.
// shortcut: no-quirks UA sheet for every document, parse a quirks copy once quirks pages matter.
static UA_SHEET: LazyLock<DocumentStyleSheet> = LazyLock::new(|| {
    let src = Source {
        css: include_str!("ua.css").to_owned(),
        base: "about:blank".to_owned(),
        media: String::new(),
    };
    sheet(
        &src,
        Origin::UserAgent,
        QuirksMode::NoQuirks,
        &BTreeMap::new(),
        AllowImportRules::Yes,
    )
});

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
    sheets: Vec<DocumentStyleSheet>,
    stylist: Stylist,
    query: QueryState,
    /// The jsdom form-state snapshot `query` was built from; see `DomState::query_state`.
    form_state: Option<Vec<u32>>,
    /// Filled as stylo reaches each element. Boxed so a handed-out `&Extra` stays put while
    /// the map grows.
    extras: RefCell<HashMap<NodeId, Box<Extra>>>,
}

/// The op's state: one style table per document read from, the sheet bodies the page fetched
/// for `@import`, and the sheets and rules the CSSOM reaches by id.
#[derive(Default)]
pub struct CssState {
    pub(crate) docs: HashMap<NodeId, DocStyle>,
    /// `@import` bodies by URL, per document.
    pub(crate) imports: HashMap<NodeId, BTreeMap<String, String>>,
    pub(crate) handles: crate::cssom::Handles,
}

pub(crate) fn device(quirks: QuirksMode) -> Device {
    Device::new(
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
    )
}

fn stylist(quirks: QuirksMode, sheets: &[DocumentStyleSheet]) -> Stylist {
    let mut stylist = Stylist::new(device(quirks), quirks);
    let guard = lock().read();
    stylist.append_stylesheet(UA_SHEET.clone(), &guard);
    let mut seen = HashSet::new();
    for sheet in sheets {
        // stylo takes each sheet once, and a repeat still counts at its own place in the order.
        let sheet = if seen.insert(&*sheet.0 as *const _) {
            sheet.clone()
        } else {
            DocumentStyleSheet(Arc::new((*sheet.0).clone()))
        };
        stylist.append_stylesheet(sheet, &guard);
    }
    let guards = StylesheetGuards::same(&guard);
    let _ = stylist.flush(&guards);
    stylist
}

fn has_token(list: Option<&str>, token: &str) -> bool {
    list.unwrap_or("")
        .split_ascii_whitespace()
        .any(|t| t.eq_ignore_ascii_case(token))
}

/// The document's `<style>` and loaded `<link>` sheets in tree order, minus the ones a browser
/// leaves off: a foreign `type`, alternates, and titled sheets outside the first title's set. A
/// `disabled` link has no sheet at all.
// shortcut: tree order stands in for the order sheets were created, which differs only when a
// `<link>` loads after a later `<style>` gave the first title, track creation order once a page
// relies on that.
fn author_sheets(root: NodeRef<Node>, handles: &crate::cssom::Handles) -> Vec<DocumentStyleSheet> {
    let mut preferred = None;
    let mut sheets: Vec<_> = root
        .descendants()
        .filter_map(|n| {
            let e = n.value().as_element().filter(|e| e.name.ns == ns!(html))?;
            let applies = match e.name.local {
                local_name!("style") => e
                    .attr("type")
                    .is_none_or(|t| t.is_empty() || t.eq_ignore_ascii_case("text/css")),
                local_name!("link") => {
                    has_token(e.attr("rel"), "stylesheet") && !has_token(e.attr("rel"), "alternate")
                }
                _ => false,
            };
            let sheet = handles.owned_sheet(n.id()).filter(|_| applies)?;
            let title = e.attr("title").unwrap_or("");
            if !title.is_empty() && *preferred.get_or_insert_with(|| title.to_owned()) != title {
                return None;
            }
            Some(sheet)
        })
        .collect();
    sheets.extend(handles.adopted_sheets(root.id()));
    sheets
}

fn element_state(node: NodeRef<Node>, query: &QueryState) -> ElementState {
    let Some(el) = MiniElement::wrap(node, query) else {
        return ElementState::empty();
    };
    el.element_state()
}

impl DocStyle {
    /// Rebuilt whenever anything it read may have changed: the document's change counter
    /// covers structure, attributes and text, and focus/target cover element state. The
    /// stylist survives while the same sheets apply; a CSSOM write drops the table instead.
    // shortcut: whole-table invalidation; stylo snapshots are the upgrade if this shows up in
    // profiles.
    fn build(
        dom: &DomState,
        root: NodeRef<Node>,
        key: (u32, i32, i32),
        base: &str,
        reuse: Option<DocStyle>,
        handles: &crate::cssom::Handles,
        form_state: Option<&[u32]>,
    ) -> Self {
        enable_prefs();
        let html = matches!(root.value(), Node::Document(d) if !d.xml);
        // shortcut: standards mode for every document, as jsdom's cascade did; pass the mode
        // through the op once quirks pages matter.
        let quirks = QuirksMode::NoQuirks;
        let sheets = author_sheets(root, handles);
        let same = |old: &DocStyle| {
            old.sheets.len() == sheets.len()
                && old
                    .sheets
                    .iter()
                    .zip(&sheets)
                    .all(|(a, b)| Arc::ptr_eq(&a.0, &b.0))
        };
        let stylist = match reuse {
            Some(old) if same(&old) => old.stylist,
            _ => stylist(quirks, &sheets),
        };
        Self {
            key,
            quirks,
            html,
            base: base.to_owned(),
            sheets,
            stylist,
            query: dom.query_state(html, false, key.1, key.2, form_state),
            form_state: form_state.map(<[u32]>::to_vec),
            extras: RefCell::default(),
        }
    }

    /// The `FORM_FLAGS` bits the sheets' selectors read.
    fn form_bits(&self) -> u32 {
        (0..FORM_FLAGS.len())
            .filter(|&i| {
                self.stylist
                    .iter_origins()
                    .any(|(data, _)| data.has_state_dependency(FORM_FLAGS[i]))
            })
            .fold(0, |bits, i| bits | 1 << i)
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

    fn computed(
        &self,
        arena: &ego_tree::Tree<Node>,
        node: NodeRef<Node>,
        pseudo: Option<PseudoElement>,
    ) -> Option<Arc<ComputedValues>> {
        let ctx = Ctx { arena, doc: self };
        with_ctx(&ctx, || self.resolve(N::of(node), pseudo))
    }

    fn resolve(&self, el: N, pseudo: Option<PseudoElement>) -> Option<Arc<ComputedValues>> {
        if let Some(pseudo) = pseudo.filter(|p| p.is_lazy()) {
            let primary = self.resolve(el, None)?;
            let guard = lock().read();
            thread_state::enter(ThreadState::LAYOUT);
            let style = self.stylist.lazily_compute_pseudo_element_style(
                &StylesheetGuards::same(&guard),
                el,
                &pseudo,
                RuleInclusion::All,
                &primary,
                false,
                None,
            );
            thread_state::exit(ThreadState::LAYOUT);
            return style;
        }
        if let Some(data) = el.borrow_data()
            && data.styles.get_primary().is_some()
        {
            return match pseudo {
                Some(pseudo) => data.styles.pseudos.get(&pseudo).cloned(),
                None => data.styles.get_primary().cloned(),
            };
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
            resolve_style(&mut context, el, RuleInclusion::All, pseudo.as_ref(), None)
        };
        thread_state::exit(ThreadState::LAYOUT);
        let style = match pseudo {
            Some(pseudo) => styles.pseudos.get(&pseudo).cloned(),
            None => Some(styles.primary().clone()),
        };
        // Keep the eager pseudos too: resolving the primary computes them in the same pass.
        if let Some(mut data) = el.mutate_data() {
            data.styles = styles;
        }
        style
    }
}

/// Drops what released documents left behind.
pub(crate) fn sweep(css: &mut CssState, dom: &DomState) {
    css.docs.retain(|root, _| dom.doc_version(*root).is_some());
    css.imports
        .retain(|root, _| dom.doc_version(*root).is_some());
    css.handles
        .retain_docs(|root| dom.doc_version(root).is_some());
}

/// The element's computed style, `None` outside a document. `base` is the document's base
/// URL, which resolves relative `url()`s. `Err` carries the `FORM_FLAGS` bits the sheets read
/// when `form_state`, jsdom's snapshot (see `DomState::query_state`), is missing.
#[allow(clippy::too_many_arguments)]
fn style_of(
    css: &mut CssState,
    dom: &DomState,
    wire: u32,
    focus: i32,
    target: i32,
    base: &str,
    pseudo: Option<PseudoElement>,
    form_state: Option<&[u32]>,
) -> Result<Option<Arc<ComputedValues>>, u32> {
    let _gate = GATE.read().unwrap_or_else(|e| e.into_inner());
    // shortcut: a released document's table lingers until the next read, hook `release` if
    // pages that never read styles again hold many.
    sweep(css, dom);
    let Some(node) = dom.resolve(wire).and_then(|id| dom.arena.get(id)) else {
        return Ok(None);
    };
    let root = node.ancestors().last().unwrap_or(node);
    if !node.value().is_element() {
        return Ok(None);
    }
    let Some(version) = dom.doc_version(root.id()) else {
        return Ok(None);
    };
    let key = (version, focus, target);
    let mut doc = match css.docs.remove(&root.id()) {
        Some(doc) if doc.key == key && doc.base == base => doc,
        old => DocStyle::build(dom, root, key, base, old, &css.handles, None),
    };
    let wanted = doc.form_bits();
    if wanted != 0 {
        // shortcut: jsdom walks the whole tree for a snapshot on every read while the sheets use
        // form state; have it flag changes instead if `innerText` on such pages shows up.
        match form_state {
            None => {
                css.docs.insert(root.id(), doc);
                return Err(wanted);
            }
            Some(state) if doc.form_state.as_deref() != Some(state) => {
                doc = DocStyle::build(dom, root, key, base, Some(doc), &css.handles, Some(state));
            }
            Some(_) => {}
        }
    }
    let style = doc.computed(&dom.arena, node, pseudo);
    css.docs.insert(root.id(), doc);
    Ok(style)
}

fn pseudo_element(name: &str) -> Option<PseudoElement> {
    use PseudoElement::*;
    Some(match name.to_ascii_lowercase().as_str() {
        "::after" => After,
        "::before" => Before,
        "::selection" => Selection,
        "::first-letter" => FirstLetter,
        "::backdrop" => Backdrop,
        "::details-content" => DetailsContent,
        "::marker" => Marker,
        "::color-swatch" => ColorSwatch,
        "::file-selector-button" => FileSelectorButton,
        "::placeholder" => Placeholder,
        "::slider-fill" => SliderFill,
        "::slider-thumb" => SliderThumb,
        "::slider-track" => SliderTrack,
        "::-moz-progress-bar" => MozProgressBar,
        _ => return None,
    })
}

/// `getComputedStyle(el).getPropertyValue(p)` for each of `props`; `""` for unknown names and
/// for elements outside a document. `Err` as for `style_of`.
#[allow(clippy::too_many_arguments)]
pub fn computed_values(
    css: &mut CssState,
    dom: &DomState,
    wire: u32,
    focus: i32,
    target: i32,
    base: &str,
    pseudo: &str,
    props: &[String],
    form_state: Option<&[u32]>,
) -> Result<Vec<String>, u32> {
    let pseudo = if pseudo.is_empty() {
        None
    } else if let Some(pseudo) = pseudo_element(pseudo) {
        Some(pseudo)
    } else {
        return Ok(vec![String::new(); props.len()]);
    };
    let Some(style) = style_of(css, dom, wire, focus, target, base, pseudo, form_state)? else {
        return Ok(vec![String::new(); props.len()]);
    };
    Ok(props
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
        .collect())
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

/// The values, or the `FORM_FLAGS` bits to send a snapshot of before asking again.
#[derive(serde::Serialize)]
#[serde(untagged)]
pub enum Computed {
    Values(Vec<String>),
    NeedsState(u32),
}

/// `focus` and `target` are wires, `-1` for none, and `form_state` jsdom's snapshot or null, as
/// for `op_dom_query`.
#[op2]
#[serde]
pub fn op_css_computed_values(
    state: &mut OpState,
    wire: u32,
    focus: i32,
    target: i32,
    #[serde] props: Vec<String>,
    #[string] base: &str,
    #[string] pseudo: &str,
    #[serde] form_state: Option<Vec<u32>>,
) -> Computed {
    let mut css = state.try_take::<CssState>().unwrap_or_default();
    let values = computed_values(
        &mut css,
        state.borrow::<DomState>(),
        wire,
        focus,
        target,
        base,
        pseudo,
        &props,
        form_state.as_deref(),
    );
    state.put(css);
    values.map_or_else(Computed::NeedsState, Computed::Values)
}

/// Runs `f` on the declaration block `text` parses to, its relative `url()`s resolved against
/// `base`. The CSSOM keeps no block of its own, so a `style` attribute is parsed here, once per
/// distinct text and base.
fn with_block<R>(text: &str, base: &str, f: impl FnOnce(&PropertyDeclarationBlock) -> R) -> R {
    static BLOCKS: LazyLock<Mutex<HashMap<(String, String), PropertyDeclarationBlock>>> =
        LazyLock::new(Default::default);
    let mut blocks = BLOCKS.lock().unwrap_or_else(|e| e.into_inner());
    let key = (text.to_owned(), base.to_owned());
    if !blocks.contains_key(&key) {
        // shortcut: drops the whole cache when full, as the sheet cache does.
        if blocks.len() >= SHEET_CACHE_SIZE {
            blocks.clear();
        }
        let block = parse_block(text, &url_data(base), CssRuleType::Style);
        blocks.insert(key.clone(), block);
    }
    f(&blocks[&key])
}

/// `text` as the declarations of a rule of `rule_type`.
pub(crate) fn parse_block(
    text: &str,
    base: &UrlExtraData,
    rule_type: CssRuleType,
) -> PropertyDeclarationBlock {
    enable_prefs();
    parse_style_attribute(
        &preprocess(text),
        base,
        None,
        QuirksMode::NoQuirks,
        rule_type,
    )
}

/// CSS input preprocessing, which stylo leaves to its embedder: custom property values keep
/// their source text, so a raw CR or form feed would otherwise survive serialisation.
pub(crate) fn preprocess(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace(['\r', '\u{c}'], "\n")
        .replace('\0', "\u{fffd}")
}

/// `name` as a property, when it is one; a custom property's name must be a whole ident, which
/// stylo does not check, as its parser only ever hands it one.
fn property_id(name: &str) -> Option<PropertyId> {
    if name.starts_with("--") {
        let mut parser = Parser::new(name);
        let ident = matches!(
            parser.next_including_whitespace(),
            Ok(cssparser::Token::Ident(_))
        );
        if !ident || parser.next_including_whitespace().is_ok() {
            return None;
        }
    }
    PropertyId::parse_enabled_for_all_content(name).ok()
}

pub(crate) fn block_text(block: &PropertyDeclarationBlock) -> String {
    let mut s = String::new();
    let _ = block.to_css(&mut s);
    s
}

/// The serialization of the declarations in `text`, for `style.cssText`.
pub fn declarations_text(text: &str, base: &str) -> String {
    with_block(text, base, block_text)
}

/// The longhands `text` declares, in order, for `style.length` and `style.item()`.
pub fn declaration_names(text: &str, base: &str) -> Vec<String> {
    with_block(text, base, |block| {
        block
            .declarations()
            .iter()
            .map(|d| d.id().name().into_owned())
            .collect()
    })
}

/// `property`'s value and whether it is `!important`; `""` for an unknown or undeclared one.
pub fn declaration_value(text: &str, base: &str, property: &str) -> (String, bool) {
    let Some(id) = property_id(property) else {
        return (String::new(), false);
    };
    with_block(text, base, |block| {
        let mut value = String::new();
        let _ = block.property_value_to_css(&id, &mut value);
        (value, block.property_priority(&id).important())
    })
}

/// `value` parsed as `id`'s declarations, as a style rule would hold them.
fn parse_value(
    id: PropertyId,
    value: &str,
    base: &UrlExtraData,
) -> Option<SourcePropertyDeclaration> {
    enable_prefs();
    let mut source = SourcePropertyDeclaration::default();
    parse_one_declaration_into(
        &mut source,
        id,
        &preprocess(value),
        Origin::Author,
        base,
        None,
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        CssRuleType::Style,
    )
    .ok()?;
    Some(source)
}

/// The serialization of `text` with `property` set to `value`, or `None` when the name is
/// unknown, the value does not parse, or nothing would change.
pub fn set_declaration(
    text: &str,
    base: &str,
    property: &str,
    value: &str,
    important: bool,
) -> Option<String> {
    let id = property_id(property)?;
    let mut source = parse_value(id, value, &url_data(base))?;
    let importance = if important {
        Importance::Important
    } else {
        Importance::Normal
    };
    with_block(text, base, |block| {
        let mut block = block.clone();
        let mut updates = SourcePropertyDeclarationUpdate::default();
        if !block.prepare_for_update(&source, importance, &mut updates) {
            return None;
        }
        block.update(source.drain(), importance, &mut updates);
        Some(block_text(&block))
    })
}

/// The serialization of `text` without `property` (and, for a shorthand, its longhands), or
/// `None` when it declares none of them.
pub fn remove_declaration(text: &str, base: &str, property: &str) -> Option<String> {
    let id = property_id(property)?;
    with_block(text, base, |block| {
        let first = block.first_declaration_to_remove(&id)?;
        let mut block = block.clone();
        block.remove_property(&id, first);
        Some(block_text(&block))
    })
}

/// `CSS.supports(property, value)`, or `CSS.supports(conditionText)` when `value` is `None`.
pub fn supports(property: &str, value: Option<&str>) -> bool {
    enable_prefs();
    let base = url_data("about:blank");
    let Some(value) = value else {
        let context = author_context(&base, CssRuleType::Style, QuirksMode::NoQuirks);
        return Parser::new(&preprocess(property))
            .parse_entirely(parse_condition_or_declaration)
            .is_ok_and(|condition| condition.eval(&context));
    };
    // Unlike `property_id`, any name after `--` counts: CSS.supports only asks for the prefix.
    let Ok(id) = PropertyId::parse_enabled_for_all_content(property) else {
        return false;
    };
    parse_value(id, value, &base).is_some()
}

/// The longhands `getComputedStyle` enumerates: every enabled one, by name, prefixed ones last.
pub fn computed_longhands() -> Vec<String> {
    enable_prefs();
    let mut names: Vec<_> = ShorthandId::All
        .longhands()
        .map(|l| l.name())
        .chain(["direction", "unicode-bidi"])
        .filter(|name| PropertyId::parse_enabled_for_all_content(name).is_ok())
        .map(str::to_owned)
        .collect();
    names.sort_by(|a, b| (a.starts_with('-'), a).cmp(&(b.starts_with('-'), b)));
    names
}

#[op2]
#[string]
pub fn op_css_declarations_text(#[string] text: &str, #[string] base: &str) -> String {
    declarations_text(text, base)
}

#[op2]
#[serde]
pub fn op_css_declaration_names(#[string] text: &str, #[string] base: &str) -> Vec<String> {
    declaration_names(text, base)
}

#[op2]
#[serde]
pub fn op_css_declaration_value(
    #[string] text: &str,
    #[string] base: &str,
    #[string] property: &str,
) -> (String, bool) {
    declaration_value(text, base, property)
}

#[op2]
#[string]
pub fn op_css_set_declaration(
    #[string] text: &str,
    #[string] base: &str,
    #[string] property: &str,
    #[string] value: &str,
    important: bool,
) -> Option<String> {
    set_declaration(text, base, property, value, important)
}

#[op2]
#[string]
pub fn op_css_remove_declaration(
    #[string] text: &str,
    #[string] base: &str,
    #[string] property: &str,
) -> Option<String> {
    remove_declaration(text, base, property)
}

#[op2]
pub fn op_css_supports(#[string] property: &str, #[string] value: Option<String>) -> bool {
    supports(property, value.as_deref())
}

#[op2]
#[serde]
pub fn op_css_computed_longhands() -> Vec<String> {
    computed_longhands()
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

    // shortcut: equality or `-` prefix match only, no RFC 4647 `*` wildcards, upgrade when a
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
        lang.is_some_and(|lang| crate::dom_query::lang_matches(lang.as_ref(), value.as_ref()))
    }

    fn is_html_document_body_element(&self) -> bool {
        self.el().name.ns == ns!(html)
            && self.el().name.local == local_name!("body")
            && self
                .node()
                .parent()
                .is_some_and(|p| N::is_root(&self.with(p)))
    }

    // shortcut: no presentational hints (`bgcolor`, `width=`, `hidden` is in the UA sheet),
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
        style_of(css, dom, wire, -1, -1, "about:blank", None, None)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn a_change_in_one_document_keeps_anothers_table() {
        let mut dom = DomState::new();
        let a = dom.parse_document("<style>p { color: rgb(1, 2, 3) }</style><p>a</p>");
        let b = dom.parse_document("<p>b</p>");
        let (pa, pb) = (element(&mut dom, a, "p"), element(&mut dom, b, "p"));
        let mut css = CssState::default();
        let style = element(&mut dom, a, "style");
        let style = dom.resolve(style);
        css.sheet_new(a, style, "p { color: rgb(1, 2, 3) }", "about:blank", "");
        let color = |css: &mut CssState, dom: &DomState, wire| {
            computed_values(
                css,
                dom,
                wire,
                -1,
                -1,
                "about:blank",
                "",
                &["color".into()],
                None,
            )
            .unwrap()
            .remove(0)
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

    #[test]
    fn declarations_read_and_write_through_text() {
        let text = "COLOR: red; margin: 1px 2px; --Gap: 3px; bogus: 1";
        assert_eq!(
            declarations_text(text, "about:blank"),
            "color: red; margin: 1px 2px; --Gap: 3px;"
        );
        assert_eq!(
            declaration_names(text, "about:blank"),
            [
                "color",
                "margin-top",
                "margin-right",
                "margin-bottom",
                "margin-left",
                "--Gap"
            ]
        );
        assert_eq!(
            declaration_value(text, "about:blank", "margin-left"),
            ("2px".into(), false)
        );
        assert_eq!(
            declaration_value(text, "about:blank", "bogus"),
            (String::new(), false)
        );

        let set = set_declaration(text, "about:blank", "Color", "blue", true).unwrap();
        assert_eq!(set, "color: blue !important; margin: 1px 2px; --Gap: 3px;");
        assert_eq!(
            declaration_value(&set, "about:blank", "color"),
            ("blue".into(), true)
        );
        assert_eq!(
            set_declaration(&set, "about:blank", "color", "blue", true),
            None
        );
        assert_eq!(
            set_declaration(text, "about:blank", "color", "1px", false),
            None
        );
        assert_eq!(
            set_declaration(text, "about:blank", "bogus", "1", false),
            None
        );

        assert_eq!(
            remove_declaration(text, "about:blank", "margin").unwrap(),
            "color: red; --Gap: 3px;"
        );
        assert_eq!(remove_declaration(text, "about:blank", "padding"), None);

        assert_eq!(
            declarations_text("--x: a\r\nb\u{c}c", "about:blank"),
            "--x: a\nb\nc;"
        );
        assert_eq!(set_declaration("", "about:blank", "--x ", "1", false), None);
        assert_eq!(
            set_declaration("", "about:blank", "--x y", "1", false),
            None
        );
    }

    #[test]
    fn every_style_attribute_names_a_parsed_property() {
        enable_prefs();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/node_modules/jsdom/lib/jsdom/living/css/style-property-names.json"
        );
        let names: HashMap<String, String> =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let stale: Vec<_> = names
            .values()
            .filter(|p| PropertyId::parse_enabled_for_all_content(p).is_err())
            .collect();
        assert!(
            stale.is_empty(),
            "rerun scripts/css-style-properties.py: {stale:?}"
        );
    }

    #[test]
    fn computed_longhands_are_enabled_and_sorted() {
        let names = computed_longhands();
        assert!(names.contains(&"grid-template-columns".to_owned()));
        assert!(!names.contains(&"flex-line-count".to_owned()));
        assert!(!names.contains(&"margin".to_owned()));
        let first_prefixed = names.iter().position(|n| n.starts_with('-'));
        assert!(first_prefixed.is_none_or(|i| names[i..].iter().all(|n| n.starts_with('-'))));
        assert!(
            names
                .windows(2)
                .all(|w| w[0] < w[1] || w[1].starts_with('-'))
        );
    }
}
