//! CSS selector matching over the arena. `scraper`'s own selector layer cannot be extended (its
//! pseudo-class enum is uninhabited and its `Selector` sealed), so this module owns a
//! `selectors::SelectorImpl` and a `selectors::Element` over `ego_tree::NodeRef<scraper::Node>`.

use cssparser::{CowRcStr, ParseError, Parser as CssParser, ParserInput, SourceLocation, ToCss};
use ego_tree::{NodeId, NodeRef};
use html5ever::Namespace;
use scraper::node::{Element as ScraperElement, Node};
use scraper::selector::{CssLocalName, CssString};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::matching::{
    self, ElementSelectorFlags, MatchingContext, MatchingForInvalidation, MatchingMode,
    NeedsSelectorFlags, QuirksMode, SelectorCaches,
};
use selectors::parser::{self, Component, ParseRelative, SelectorList, SelectorParseErrorKind};
use selectors::{Element, OpaqueElement};

const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

/// Legal pseudo-classes no element can be in here: no history, media, fullscreen or autofill.
const NEVER_MATCHING_PSEUDO_CLASSES: [&str; 21] = [
    "autofill",
    "blank",
    "buffering",
    "current",
    "fullscreen",
    "future",
    "has-slotted",
    "heading",
    "modal",
    "muted",
    "past",
    "paused",
    "picture-in-picture",
    "playing",
    "popover-open",
    "seeking",
    "stalled",
    "user-invalid",
    "user-valid",
    "visited",
    "volume-locked",
];

/// Pseudo-elements are never in the tree, so they parse and never match.
const PSEUDO_ELEMENTS: [&str; 12] = [
    "after",
    "backdrop",
    "before",
    "cue",
    "cue-region",
    "file-selector-button",
    "first-letter",
    "first-line",
    "marker",
    "placeholder",
    "selection",
    "target-text",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryMode {
    First,
    All,
    /// The element itself, then its ancestors.
    Closest,
    /// The element itself only.
    Matches,
}

impl QueryMode {
    pub fn from_u8(mode: u8) -> Option<Self> {
        [Self::First, Self::All, Self::Closest, Self::Matches]
            .get(mode as usize)
            .copied()
    }
}

/// A compiled selector group (comma-separated list).
pub struct MatchSelector {
    list: SelectorList<MiniImpl>,
}

impl MatchSelector {
    /// `None` when the selector is invalid or needs what this matcher lacks (namespace
    /// prefixes, `:lang`, state pseudo-classes, ...).
    pub fn parse(selector: &str) -> Option<Self> {
        let mut input = ParserInput::new(selector);
        let mut parser = CssParser::new(&mut input);
        SelectorList::parse(&MiniParser, &mut parser, ParseRelative::No)
            .ok()
            .map(|list| Self { list })
    }

    /// The id every match must bear: the list is one selector whose subject compound has an
    /// id selector.
    pub fn subject_id(&self) -> Option<&str> {
        let [selector] = self.list.slice() else {
            return None;
        };
        selector.iter().find_map(|c| match c {
            Component::ID(id) => Some(id.0.as_ref()),
            _ => None,
        })
    }

    fn matches(&self, element: &MiniElement, context: &mut MatchingContext<MiniImpl>) -> bool {
        self.list
            .slice()
            .iter()
            .any(|s| matching::matches_selector(s, 0, None, element, context))
    }
}

/// The ids of the elements `scope` selects in `mode`, in document order (ancestor order for
/// `Closest`). `html` tells whether the nodes' document is an HTML document; all nodes of one
/// tree share it.
pub fn query(
    scope: NodeRef<Node>,
    selector: &MatchSelector,
    mode: QueryMode,
    html: bool,
    quirks: bool,
) -> Vec<NodeId> {
    let candidates: Box<dyn Iterator<Item = NodeRef<Node>>> = match mode {
        QueryMode::First | QueryMode::All => Box::new(scope.descendants().skip(1)),
        QueryMode::Closest => Box::new(std::iter::once(scope).chain(scope.ancestors())),
        QueryMode::Matches => Box::new(std::iter::once(scope)),
    };
    select(scope, candidates, selector, mode, html, quirks)
}

/// `query` over caller-chosen `candidates`, which must already be in result order.
pub fn select<'a>(
    scope: NodeRef<'a, Node>,
    candidates: impl Iterator<Item = NodeRef<'a, Node>>,
    selector: &MatchSelector,
    mode: QueryMode,
    html: bool,
    quirks: bool,
) -> Vec<NodeId> {
    let scope_el = MiniElement::wrap(scope, html);
    let mut caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        if quirks {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        },
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    context.scope_element = scope_el.as_ref().map(Element::opaque);
    let mut hits = candidates
        .filter_map(|n| MiniElement::wrap(n, html))
        .filter(|el| selector.matches(el, &mut context));
    match mode {
        QueryMode::All => hits.map(|el| el.node.id()).collect(),
        _ => hits.next().map(|el| el.node.id()).into_iter().collect(),
    }
}

// --- selectors::SelectorImpl wiring -------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiniImpl;

impl parser::SelectorImpl for MiniImpl {
    type AttrValue = CssString;
    type Identifier = CssLocalName;
    type LocalName = CssLocalName;
    type NamespacePrefix = CssLocalName;
    type NamespaceUrl = Namespace;
    type BorrowedNamespaceUrl = Namespace;
    type BorrowedLocalName = CssLocalName;
    type NonTSPseudoClass = MiniPseudo;
    type PseudoElement = MiniPseudoElement;
    type ExtraMatchingData<'a> = ();
}

/// Non-tree-structural pseudo-classes this matcher accepts. Any other name is a parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiniPseudo {
    NeverMatches(String),
}

impl parser::NonTSPseudoClass for MiniPseudo {
    type Impl = MiniImpl;

    fn is_active_or_hover(&self) -> bool {
        false
    }

    fn is_user_action_state(&self) -> bool {
        false
    }
}

impl ToCss for MiniPseudo {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        match self {
            MiniPseudo::NeverMatches(name) => write!(dest, ":{name}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiniPseudoElement(String);

impl parser::PseudoElement for MiniPseudoElement {
    type Impl = MiniImpl;
}

impl ToCss for MiniPseudoElement {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        write!(dest, "::{}", self.0)
    }
}

struct MiniParser;

impl<'i> parser::Parser<'i> for MiniParser {
    type Impl = MiniImpl;
    type Error = SelectorParseErrorKind<'i>;

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_has(&self) -> bool {
        true
    }

    fn parse_nth_child_of(&self) -> bool {
        true
    }

    fn parse_non_ts_pseudo_class(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<MiniPseudo, ParseError<'i, Self::Error>> {
        known(&NEVER_MATCHING_PSEUDO_CLASSES, location, name).map(MiniPseudo::NeverMatches)
    }

    fn parse_pseudo_element(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<MiniPseudoElement, ParseError<'i, Self::Error>> {
        known(&PSEUDO_ELEMENTS, location, name).map(MiniPseudoElement)
    }
}

/// `name` lower-cased when it is in `list` or vendor-prefixed, else a parse error.
fn known<'i>(
    list: &[&str],
    location: SourceLocation,
    name: CowRcStr<'i>,
) -> Result<String, ParseError<'i, SelectorParseErrorKind<'i>>> {
    let lower = name.to_ascii_lowercase();
    if list.contains(&lower.as_str()) || lower.starts_with("-webkit-") {
        return Ok(lower);
    }
    Err(
        location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
            name,
        )),
    )
}

// --- selectors::Element newtype ---------------------------------------------------------

#[derive(Clone, Copy)]
pub struct MiniElement<'a> {
    node: NodeRef<'a, Node>,
    /// Whether the node document is an HTML document, for the HTML case rules.
    html: bool,
}

impl std::fmt::Debug for MiniElement<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MiniElement({:?})", self.node.id())
    }
}

impl<'a> MiniElement<'a> {
    fn wrap(node: NodeRef<'a, Node>, html: bool) -> Option<Self> {
        node.value().is_element().then_some(Self { node, html })
    }

    fn el(&self) -> &'a ScraperElement {
        self.node
            .value()
            .as_element()
            .expect("wrap guards elements")
    }

    fn local(&self) -> &str {
        self.el().name.local.as_ref()
    }
}

impl Element for MiniElement<'_> {
    type Impl = MiniImpl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self.node.value())
    }

    fn parent_element(&self) -> Option<Self> {
        self.node.parent().and_then(|n| Self::wrap(n, self.html))
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
        self.node
            .prev_siblings()
            .find_map(|n| Self::wrap(n, self.html))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.node
            .next_siblings()
            .find_map(|n| Self::wrap(n, self.html))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node.children().find_map(|n| Self::wrap(n, self.html))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.html && self.el().name.ns.as_ref() == HTML_NS
    }

    fn has_local_name(&self, name: &CssLocalName) -> bool {
        self.el().name.local == name.0
    }

    fn has_namespace(&self, namespace: &Namespace) -> bool {
        &self.el().name.ns == namespace
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.el().name == other.el().name
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&Namespace>,
        local_name: &CssLocalName,
        operation: &AttrSelectorOperation<&CssString>,
    ) -> bool {
        self.el().attrs.iter().any(|(key, value)| {
            !matches!(*ns, NamespaceConstraint::Specific(url) if *url != key.ns)
                && local_name.0 == key.local
                && operation.eval_str(value)
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &MiniPseudo,
        _context: &mut MatchingContext<'_, MiniImpl>,
    ) -> bool {
        match pc {
            MiniPseudo::NeverMatches(_) => false,
        }
    }

    fn match_pseudo_element(
        &self,
        _pe: &MiniPseudoElement,
        _context: &mut MatchingContext<'_, MiniImpl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        matches!(self.local(), "a" | "area") && self.el().attr("href").is_some()
    }

    fn is_html_slot_element(&self) -> bool {
        self.local() == "slot"
    }

    fn has_id(&self, id: &CssLocalName, case_sensitivity: CaseSensitivity) -> bool {
        match self.el().id() {
            Some(val) => case_sensitivity.eq(id.0.as_bytes(), val.as_bytes()),
            None => false,
        }
    }

    fn has_class(&self, name: &CssLocalName, case_sensitivity: CaseSensitivity) -> bool {
        self.el().has_class(&name.0, case_sensitivity)
    }

    fn has_custom_state(&self, _name: &CssLocalName) -> bool {
        false
    }

    fn imported_part(&self, _name: &CssLocalName) -> Option<CssLocalName> {
        None
    }

    fn is_part(&self, _name: &CssLocalName) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        !self
            .node
            .children()
            .any(|c| c.value().is_element() || c.value().is_text())
    }

    fn is_root(&self) -> bool {
        self.node.parent().is_some_and(|p| p.value().is_document())
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{MatchSelector, QueryMode};
    use crate::dom::DomState;

    /// Tag names (upper-cased, as `NodeRecord` reports them) of `scope.querySelectorAll`, or of
    /// the first hit when `!all`.
    fn q(dom: &mut DomState, scope: u32, selector: &str, all: bool) -> Vec<String> {
        let mode = if all {
            QueryMode::All
        } else {
            QueryMode::First
        };
        let ids = dom
            .query(scope, selector, mode, true, false)
            .expect("parses natively");
        dom.nodes(&ids)
            .into_iter()
            .map(|r| r.name.unwrap_or_default())
            .collect()
    }

    fn doc(html: &str) -> (DomState, u32) {
        let mut dom = DomState::new();
        let root = dom.parse(html, None)[0].id;
        (dom, root)
    }

    #[test]
    fn tag_id_class_and_attribute_selectors() {
        let (mut dom, root) = doc("<html><body>
               <div id='a' class='box'></div>
               <div class='box wide' data-role='panel'></div>
               <span>x</span>
             </body></html>");
        assert_eq!(q(&mut dom, root, "div", true), ["DIV", "DIV"]);
        assert_eq!(q(&mut dom, root, "#a", true), ["DIV"]);
        assert_eq!(q(&mut dom, root, ".box", true), ["DIV", "DIV"]);
        assert_eq!(q(&mut dom, root, ".box.wide", true), ["DIV"]);
        assert_eq!(q(&mut dom, root, "[data-role='panel']", true), ["DIV"]);
        assert_eq!(q(&mut dom, root, "span, #a", true), ["DIV", "SPAN"]); // document order
    }

    #[test]
    fn combinators_and_structural_pseudos() {
        let (mut dom, root) = doc(
            "<html><body><ul><li>1</li><li class='sel'>2</li><li>3</li></ul>
             <ol><li>a</li></ol></body></html>",
        );
        assert_eq!(q(&mut dom, root, "ul li", true), ["LI", "LI", "LI"]);
        assert_eq!(q(&mut dom, root, "ul > li", true), ["LI", "LI", "LI"]);
        assert_eq!(q(&mut dom, root, "li:nth-child(2)", true), ["LI"]);
        assert_eq!(q(&mut dom, root, "li:first-child", true), ["LI", "LI"]);
        assert_eq!(q(&mut dom, root, "li:not(.sel)", false), ["LI"]);
        assert_eq!(q(&mut dom, root, "li + li", true), ["LI", "LI"]); // 2nd/3rd only
    }

    #[test]
    fn closest_includes_the_element_and_matches_tests_only_it() {
        let (mut dom, root) = doc("<html><body><div><p><b></b></p></div></body></html>");
        let b = dom.query(root, "b", QueryMode::First, true, false).unwrap()[0];
        let closest = |dom: &mut DomState, s| dom.query(b, s, QueryMode::Closest, true, false);
        assert_eq!(closest(&mut dom, "b"), Some(vec![b]));
        let hit = closest(&mut dom, "div, p").unwrap();
        assert_eq!(dom.nodes(&hit)[0].name.as_deref(), Some("P"));
        assert_eq!(
            dom.query(b, "p b", QueryMode::Matches, true, false),
            Some(vec![b])
        );
        assert_eq!(
            dom.query(b, "p", QueryMode::Matches, true, false),
            Some(vec![])
        );
    }

    #[test]
    fn scope_is_excluded_and_first_only_stops_early() {
        let (mut dom, root) = doc("<html><body><div><p>1</p><p>2</p></div></body></html>");
        let div = dom
            .query(root, "div", QueryMode::First, true, false)
            .unwrap()[0];
        assert!(q(&mut dom, div, "div", true).is_empty(), "self excluded");
        assert_eq!(q(&mut dom, div, "p", false), ["P"]);
        assert_eq!(q(&mut dom, div, "p", true), ["P", "P"]);
    }

    #[test]
    fn invalid_or_unsupported_selector_is_a_parse_error() {
        for selector in [
            ":::",
            "div::??",
            ":totallyfake",
            ":checked",
            ":lang(en)",
            "svg|rect",
        ] {
            assert!(MatchSelector::parse(selector).is_none(), "{selector}");
        }
        let (mut dom, root) = doc("<html><body><div></div></body></html>");
        assert_eq!(dom.query(root, ":::", QueryMode::All, true, false), None);
        assert_eq!(
            dom.query(9999, "div", QueryMode::All, true, false),
            Some(vec![])
        );
    }

    #[test]
    fn legal_but_unsupported_pseudos_parse_and_never_match() {
        let (mut dom, root) = doc("<html><body><a href='/'>x</a><p>y</p></body></html>");
        for selector in [
            ":visited",
            ":modal",
            "a:-webkit-autofill",
            "p::before",
            "p:after",
        ] {
            assert!(q(&mut dom, root, selector, true).is_empty(), "{selector}");
        }
        assert_eq!(q(&mut dom, root, "a:not(:visited)", true), ["A"]);
    }
}
