//! CSS selector matching over the arena: a `selectors::SelectorImpl` and a `selectors::Element`
//! over `ego_tree::NodeRef<Node>`.

use std::cell::Cell;

use crate::node::{Element as ElementData, Node};
use cssparser::{CowRcStr, ParseError, Parser as CssParser, ParserInput, SourceLocation, ToCss};
use ego_tree::{NodeId, NodeRef};
use html5ever::{LocalName, Namespace};
use precomputed_hash::PrecomputedHash;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::matching::{
    self, ElementSelectorFlags, MatchingContext, MatchingForInvalidation, MatchingMode,
    NeedsSelectorFlags, QuirksMode, SelectorCaches,
};
use selectors::parser::{self, Component, ParseRelative, SelectorList, SelectorParseErrorKind};
use selectors::{Element, OpaqueElement};

pub(crate) const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

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

/// The state pseudo-classes matched natively: focus and target come from `QueryState`, the rest
/// derive from attributes and the tree. Any other state pseudo-class fails to parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatePseudo {
    AnyLink,
    Checked,
    Disabled,
    Enabled,
    Focus,
    FocusWithin,
    Link,
    Optional,
    ReadOnly,
    ReadWrite,
    Required,
    Target,
}

const STATE_PSEUDO_CLASSES: [(&str, StatePseudo); 12] = [
    ("any-link", StatePseudo::AnyLink),
    ("checked", StatePseudo::Checked),
    ("disabled", StatePseudo::Disabled),
    ("enabled", StatePseudo::Enabled),
    ("focus", StatePseudo::Focus),
    ("focus-within", StatePseudo::FocusWithin),
    ("link", StatePseudo::Link),
    ("optional", StatePseudo::Optional),
    ("read-only", StatePseudo::ReadOnly),
    ("read-write", StatePseudo::ReadWrite),
    ("required", StatePseudo::Required),
    ("target", StatePseudo::Target),
];

impl StatePseudo {
    fn name(self) -> &'static str {
        STATE_PSEUDO_CLASSES
            .iter()
            .find(|(_, p)| *p == self)
            .expect("every variant is in the table")
            .0
    }
}

/// What a query knows beyond the arena: the document type, focus and target.
#[derive(Debug, Default)]
pub struct QueryState {
    /// Whether the node document is an HTML document, for the HTML case rules.
    pub html: bool,
    pub quirks: bool,
    /// The focused element and its ancestors, nearest first.
    pub focus_path: Vec<NodeId>,
    pub target: Option<NodeId>,
}

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
        let mini = MiniParser::default();
        let list = SelectorList::parse(&mini, &mut parser, ParseRelative::No).ok()?;
        (!mini.unsupported.get()).then_some(Self { list })
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
/// `Closest`). All nodes of one tree share `state.html`.
pub fn query(
    scope: NodeRef<Node>,
    selector: &MatchSelector,
    mode: QueryMode,
    state: &QueryState,
) -> Vec<NodeId> {
    let candidates: Box<dyn Iterator<Item = NodeRef<Node>>> = match mode {
        QueryMode::First | QueryMode::All => Box::new(scope.descendants().skip(1)),
        QueryMode::Closest => Box::new(std::iter::once(scope).chain(scope.ancestors())),
        QueryMode::Matches => Box::new(std::iter::once(scope)),
    };
    select(scope, candidates, selector, mode, state)
}

/// `query` over caller-chosen `candidates`, which must already be in result order.
pub fn select<'a>(
    scope: NodeRef<'a, Node>,
    candidates: impl Iterator<Item = NodeRef<'a, Node>>,
    selector: &MatchSelector,
    mode: QueryMode,
    state: &QueryState,
) -> Vec<NodeId> {
    let scope_el = MiniElement::wrap(scope, state);
    let mut caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        if state.quirks {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        },
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    context.scope_element = scope_el.as_ref().map(Element::opaque);
    let mut hits = candidates
        .filter_map(|n| MiniElement::wrap(n, state))
        .filter(|el| selector.matches(el, &mut context));
    match mode {
        QueryMode::All => hits.map(|el| el.node.id()).collect(),
        _ => hits.next().map(|el| el.node.id()).into_iter().collect(),
    }
}

// --- selectors::SelectorImpl wiring -------------------------------------------------------

/// An attribute value in a parsed selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssString(pub String);

impl From<&str> for CssString {
    fn from(val: &str) -> Self {
        Self(val.to_owned())
    }
}

impl AsRef<str> for CssString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ToCss for CssString {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        cssparser::serialize_string(&self.0, dest)
    }
}

/// A name in a parsed selector: identifier, local name or namespace prefix.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CssLocalName(pub LocalName);

impl From<&str> for CssLocalName {
    fn from(val: &str) -> Self {
        Self(val.into())
    }
}

impl ToCss for CssLocalName {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        dest.write_str(&self.0)
    }
}

impl PrecomputedHash for CssLocalName {
    fn precomputed_hash(&self) -> u32 {
        self.0.precomputed_hash()
    }
}

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
    State(StatePseudo),
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
            MiniPseudo::State(pc) => write!(dest, ":{}", pc.name()),
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

/// `:is()` and `:where()` drop what fails to parse, so `unsupported` keeps those selectors
/// away from this matcher too.
#[derive(Default)]
struct MiniParser {
    unsupported: Cell<bool>,
}

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
        let state = STATE_PSEUDO_CLASSES
            .iter()
            .find(|(n, _)| name.eq_ignore_ascii_case(n));
        if let Some(&(_, pc)) = state {
            return Ok(MiniPseudo::State(pc));
        }
        let known = known(&NEVER_MATCHING_PSEUDO_CLASSES, location, name);
        self.unsupported
            .set(self.unsupported.get() || known.is_err());
        known.map(MiniPseudo::NeverMatches)
    }

    fn parse_pseudo_element(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<MiniPseudoElement, ParseError<'i, Self::Error>> {
        let known = known(&PSEUDO_ELEMENTS, location, name);
        self.unsupported
            .set(self.unsupported.get() || known.is_err());
        known.map(MiniPseudoElement)
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
    state: &'a QueryState,
}

impl std::fmt::Debug for MiniElement<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MiniElement({:?})", self.node.id())
    }
}

impl<'a> MiniElement<'a> {
    fn wrap(node: NodeRef<'a, Node>, state: &'a QueryState) -> Option<Self> {
        node.value().is_element().then_some(Self { node, state })
    }

    fn el(&self) -> &'a ElementData {
        self.node
            .value()
            .as_element()
            .expect("wrap guards elements")
    }

    fn local(&self) -> &str {
        self.el().name.local.as_ref()
    }

    /// The local name of an HTML element, `None` for other namespaces.
    fn html_local(&self) -> Option<&str> {
        (self.el().name.ns.as_ref() == HTML_NS).then(|| self.local())
    }

    fn has_attr(&self, name: &str) -> bool {
        self.el().attr(name).is_some()
    }

    /// The `type` attribute's state; missing and unknown values are the Text state.
    fn input_type(&self) -> String {
        let ty = self
            .el()
            .attr("type")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if INPUT_TYPES.contains(&ty.as_str()) {
            ty
        } else {
            "text".into()
        }
    }

    fn in_state(&self, pc: StatePseudo) -> bool {
        use StatePseudo as P;
        let id = self.node.id();
        match pc {
            P::AnyLink | P::Link => self.is_link(),
            P::Checked => match self.html_local() {
                Some("input") => {
                    matches!(self.input_type().as_str(), "checkbox" | "radio") && self.el().checked
                }
                Some("option") => self.el().checked,
                _ => false,
            },
            P::Disabled => self.is_disabled(),
            P::Enabled => {
                matches!(
                    self.html_local(),
                    Some(
                        "button"
                            | "input"
                            | "select"
                            | "textarea"
                            | "optgroup"
                            | "option"
                            | "fieldset"
                    )
                ) && !self.is_disabled()
            }
            P::Required => self.is_required(),
            P::Optional => {
                matches!(self.html_local(), Some("input" | "select" | "textarea"))
                    && !self.is_required()
            }
            P::ReadWrite => self.is_read_write(),
            P::ReadOnly => !self.is_read_write(),
            P::Focus => self.state.focus_path.first() == Some(&id),
            P::FocusWithin => self.state.focus_path.contains(&id),
            P::Target => self.state.target == Some(id),
        }
    }

    /// HTML's "actually disabled".
    fn is_disabled(&self) -> bool {
        match self.html_local() {
            Some("button" | "input" | "select" | "textarea" | "fieldset") => {
                self.has_attr("disabled") || self.in_disabled_fieldset()
            }
            Some("optgroup") => self.has_attr("disabled"),
            Some("option") => {
                self.has_attr("disabled")
                    || self.parent_element().is_some_and(|p| {
                        p.html_local() == Some("optgroup") && p.has_attr("disabled")
                    })
            }
            _ => false,
        }
    }

    /// A descendant of a disabled `fieldset`, outside that fieldset's first `legend`.
    fn in_disabled_fieldset(&self) -> bool {
        let mut child = self.node;
        for ancestor in self.node.ancestors() {
            let disabled_fieldset = Self::wrap(ancestor, self.state)
                .is_some_and(|f| f.html_local() == Some("fieldset") && f.has_attr("disabled"));
            if disabled_fieldset {
                let legend = ancestor.children().find(|c| {
                    Self::wrap(*c, self.state).is_some_and(|e| e.html_local() == Some("legend"))
                });
                if legend.map(|l| l.id()) != Some(child.id()) {
                    return true;
                }
            }
            child = ancestor;
        }
        false
    }

    fn is_required(&self) -> bool {
        let applies = match self.html_local() {
            Some("select" | "textarea") => true,
            Some("input") => REQUIRED_INPUT_TYPES.contains(&self.input_type().as_str()),
            _ => false,
        };
        applies && self.has_attr("required")
    }

    /// Mutable text controls, editing hosts and editable elements.
    fn is_read_write(&self) -> bool {
        let mutable = || !self.has_attr("readonly") && !self.is_disabled();
        match self.html_local() {
            Some("input") => {
                READONLY_INPUT_TYPES.contains(&self.input_type().as_str()) && mutable()
            }
            Some("textarea") => mutable(),
            _ => self.is_editable(),
        }
    }

    /// `contenteditable`, inherited from the nearest ancestor that sets a known state.
    fn is_editable(&self) -> bool {
        std::iter::once(self.node)
            .chain(self.node.ancestors())
            .filter_map(|n| MiniElement::wrap(n, self.state))
            .filter(|e| e.html_local().is_some())
            .filter_map(|e| e.el().attr("contenteditable"))
            .find_map(|v| match v.to_ascii_lowercase().as_str() {
                "" | "true" | "plaintext-only" => Some(true),
                "false" => Some(false),
                _ => None,
            })
            .unwrap_or(false)
    }
}

const INPUT_TYPES: [&str; 22] = [
    "button",
    "checkbox",
    "color",
    "date",
    "datetime-local",
    "email",
    "file",
    "hidden",
    "image",
    "month",
    "number",
    "password",
    "radio",
    "range",
    "reset",
    "search",
    "submit",
    "tel",
    "text",
    "time",
    "url",
    "week",
];

/// Input types the `readonly` attribute applies to.
const READONLY_INPUT_TYPES: [&str; 12] = [
    "date",
    "datetime-local",
    "email",
    "month",
    "number",
    "password",
    "search",
    "tel",
    "text",
    "time",
    "url",
    "week",
];

/// Input types the `required` attribute applies to.
const REQUIRED_INPUT_TYPES: [&str; 15] = [
    "checkbox",
    "date",
    "datetime-local",
    "email",
    "file",
    "month",
    "number",
    "password",
    "radio",
    "search",
    "tel",
    "text",
    "time",
    "url",
    "week",
];

impl Element for MiniElement<'_> {
    type Impl = MiniImpl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self.node.value())
    }

    fn parent_element(&self) -> Option<Self> {
        self.node.parent().and_then(|n| Self::wrap(n, self.state))
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
            .find_map(|n| Self::wrap(n, self.state))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.node
            .next_siblings()
            .find_map(|n| Self::wrap(n, self.state))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node.children().find_map(|n| Self::wrap(n, self.state))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.state.html && self.el().name.ns.as_ref() == HTML_NS
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
        self.el().attrs.iter().any(|a| {
            !matches!(*ns, NamespaceConstraint::Specific(url) if *url != a.name.ns)
                && local_name.0 == a.name.local
                && operation.eval_str(&a.value)
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &MiniPseudo,
        _context: &mut MatchingContext<'_, MiniImpl>,
    ) -> bool {
        match pc {
            MiniPseudo::NeverMatches(_) => false,
            MiniPseudo::State(pc) => self.in_state(*pc),
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
    use super::{MatchSelector, QueryMode, QueryState};
    use crate::dom::DomState;

    fn html() -> QueryState {
        QueryState {
            html: true,
            ..Default::default()
        }
    }

    /// Tag names (upper-cased, as `NodeRecord` reports them) of `scope.querySelectorAll`, or of
    /// the first hit when `!all`.
    fn q(dom: &mut DomState, scope: u32, selector: &str, all: bool) -> Vec<String> {
        let mode = if all {
            QueryMode::All
        } else {
            QueryMode::First
        };
        let ids = dom
            .query(scope, selector, mode, &html())
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
        let b = dom.query(root, "b", QueryMode::First, &html()).unwrap()[0];
        let closest = |dom: &mut DomState, s| dom.query(b, s, QueryMode::Closest, &html());
        assert_eq!(closest(&mut dom, "b"), Some(vec![b]));
        let hit = closest(&mut dom, "div, p").unwrap();
        assert_eq!(dom.nodes(&hit)[0].name.as_deref(), Some("P"));
        assert_eq!(
            dom.query(b, "p b", QueryMode::Matches, &html()),
            Some(vec![b])
        );
        assert_eq!(dom.query(b, "p", QueryMode::Matches, &html()), Some(vec![]));
    }

    #[test]
    fn scope_is_excluded_and_first_only_stops_early() {
        let (mut dom, root) = doc("<html><body><div><p>1</p><p>2</p></div></body></html>");
        let div = dom.query(root, "div", QueryMode::First, &html()).unwrap()[0];
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
            ":hover",
            ":lang(en)",
            "svg|rect",
        ] {
            assert!(MatchSelector::parse(selector).is_none(), "{selector}");
        }
        let (mut dom, root) = doc("<html><body><div></div></body></html>");
        assert_eq!(dom.query(root, ":::", QueryMode::All, &html()), None);
        assert_eq!(
            dom.query(9999, "div", QueryMode::All, &html()),
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

    #[test]
    fn checked_reads_the_arena_flag_of_checkable_inputs_and_options() {
        let (mut dom, root) = doc("<html><body><input type='RADIO'><input><select>
             <option>a</option><option selected>b</option></select></body></html>");
        let all = |dom: &mut DomState, s| dom.query(root, s, QueryMode::All, &html()).unwrap();
        for wire in all(&mut dom, "input, option:first-child") {
            dom.set_checked(wire, true);
        }
        assert_eq!(q(&mut dom, root, ":checked", true), ["INPUT", "OPTION"]);
    }
}
