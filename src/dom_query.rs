//! CSS selector matching over the arena: a `selectors::SelectorImpl` and a `selectors::Element`
//! over `ego_tree::NodeRef<Node>`.

use std::collections::HashMap;
use std::sync::Once;

use crate::node::{Element as ElementData, Node};
use cssparser::Parser as CssParser;
use ego_tree::{NodeId, NodeRef};
use html5ever::{LocalName, Namespace};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::matching::{
    self, ElementSelectorFlags, MatchingContext, MatchingForInvalidation, MatchingMode,
    NeedsSelectorFlags, QuirksMode, SelectorCaches,
};
use selectors::parser::{Component, ParseRelative, RelativeSelector, SelectorList};
use selectors::visitor::SelectorVisitor;
use selectors::{Element, OpaqueElement};
use style::selector_parser::{NonTSPseudoClass, PseudoElement, SelectorImpl, SelectorParser};
use style::stylesheets::UrlExtraData;
use style::stylesheets::{Namespaces, Origin};
use style::values::{AtomIdent, AtomString, GenericAtomIdent};
use stylo_dom::ElementState;

pub(crate) const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

// Stylo gates selectors and properties browsers ship behind prefs read at parse time, so they
// are set before the first sheet; no thread pool, as styling runs on the isolate's thread.
pub(crate) fn enable_prefs() {
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

/// The state pseudo-classes matched natively: focus and target come from `QueryState`, the rest
/// derive from attributes and the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatePseudo {
    AnyLink,
    Checked,
    Default,
    Disabled,
    Enabled,
    Focus,
    FocusVisible,
    FocusWithin,
    Optional,
    ReadOnly,
    ReadWrite,
    Required,
    Target,
}

/// Each native state and the stylo flag it sets. Pseudo-classes outside it (`:hover`,
/// `:visited`, `:modal`, ...) parse and never match.
const STATE_FLAGS: [(StatePseudo, ElementState); 13] = [
    (StatePseudo::AnyLink, ElementState::UNVISITED),
    (StatePseudo::Checked, ElementState::CHECKED),
    (StatePseudo::Default, ElementState::DEFAULT),
    (StatePseudo::Disabled, ElementState::DISABLED),
    (StatePseudo::Enabled, ElementState::ENABLED),
    (StatePseudo::Focus, ElementState::FOCUS),
    (StatePseudo::FocusVisible, ElementState::FOCUSRING),
    (StatePseudo::FocusWithin, ElementState::FOCUS_WITHIN),
    (StatePseudo::Optional, ElementState::OPTIONAL_),
    (StatePseudo::ReadOnly, ElementState::READONLY),
    (StatePseudo::ReadWrite, ElementState::READWRITE),
    (StatePseudo::Required, ElementState::REQUIRED),
    (StatePseudo::Target, ElementState::URLTARGET),
];

/// The state whose inputs (values, validity, `indeterminate`) live only in jsdom, which hands it
/// over per query: bit `i` of a snapshot entry stands for `FORM_FLAGS[i]`.
pub const FORM_FLAGS: [ElementState; 6] = [
    ElementState::VALID,
    ElementState::INVALID,
    ElementState::INRANGE,
    ElementState::OUTOFRANGE,
    ElementState::PLACEHOLDER_SHOWN,
    ElementState::INDETERMINATE,
];

/// The `FORM_FLAGS` a snapshot entry's `bits` set.
pub fn form_flags(bits: u32) -> ElementState {
    FORM_FLAGS
        .iter()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .fold(ElementState::empty(), |state, (_, flag)| state | *flag)
}

/// `:state()` names standing in for names stylo 0.22 lacks; see `rewrite`.
const IN_RANGE: &str = "-mini-in-range";
const DIR_LTR: &str = "-mini-dir-ltr";
const DIR_RTL: &str = "-mini-dir-rtl";

/// The `FORM_FLAGS` bits `pc` reads.
fn form_bits(pc: &NonTSPseudoClass) -> u32 {
    let flag = match pc {
        NonTSPseudoClass::CustomState(state) if &*state.0 == IN_RANGE => ElementState::INRANGE,
        _ => pc.state_flag(),
    };
    (0..FORM_FLAGS.len())
        .filter(|&i| flag.intersects(FORM_FLAGS[i]))
        .fold(0, |bits, i| bits | 1 << i)
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
    /// jsdom's `FORM_FLAGS` per element, when the query reads them.
    pub form_state: Option<HashMap<NodeId, ElementState>>,
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

/// Why a selector gets no native answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declined {
    /// Not a valid selector: the caller throws `SyntaxError`.
    Invalid,
    /// Valid, but it reads these `FORM_FLAGS` bits and the query carries no snapshot of them.
    NeedsState(u32),
}

/// A compiled selector group (comma-separated list).
pub struct MatchSelector {
    list: SelectorList<SelectorImpl>,
    /// The `FORM_FLAGS` bits it reads.
    pub form_bits: u32,
}

impl MatchSelector {
    /// Parsed as the cascade parses author selectors, under the same prefs.
    pub fn parse(selector: &str) -> Result<Self, Declined> {
        enable_prefs();
        let url_data = UrlExtraData::from(url::Url::parse("about:blank").expect("static URL"));
        let parser = SelectorParser {
            stylesheet_origin: Origin::Author,
            namespaces: &Namespaces::default(),
            url_data: &url_data,
            for_supports_rule: false,
        };
        let parse = |source: &str| {
            CssParser::new(source)
                .parse_entirely(|i| SelectorList::parse(&parser, i, ParseRelative::No))
                .ok()
        };
        let list = parse(selector)
            .or_else(|| {
                let mut source = String::new();
                rewrite(&mut CssParser::new(selector), &mut source).then(|| parse(&source))?
            })
            .ok_or(Declined::Invalid)?;
        let mut form = FormBits(0);
        for s in list.slice() {
            s.visit(&mut form);
        }
        Ok(Self {
            list,
            form_bits: form.0,
        })
    }

    /// The id every match must bear: the list is one selector whose subject compound has an
    /// id selector.
    pub fn subject_id(&self) -> Option<&str> {
        let [selector] = self.list.slice() else {
            return None;
        };
        selector.iter().find_map(|c| match c {
            Component::ID(id) => Some(&*id.0),
            _ => None,
        })
    }

    fn matches(&self, element: &MiniElement, context: &mut MatchingContext<SelectorImpl>) -> bool {
        self.list
            .slice()
            .iter()
            .any(|s| matching::matches_selector(s, 0, None, element, context))
    }
}

/// Copies `input` to `out` with the names stylo 0.22 lacks but browsers accept swapped for ones
/// it parses that match alike (see `replacement`). False when what follows a name is malformed.
fn rewrite(input: &mut CssParser, out: &mut String) -> bool {
    use cssparser::Token;
    let mut colons = 0;
    let mut ok = true;
    loop {
        let start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments() else {
            return ok;
        };
        let token = token.clone();
        let text = input.slice_from(start).to_owned();
        let Ok(swap) = replacement(colons, &token, input) else {
            return false;
        };
        if let Some(swap) = swap {
            out.push_str(&swap);
        } else {
            out.push_str(&text);
            let close = match token {
                Token::Function(_) | Token::ParenthesisBlock => Some(")"),
                Token::SquareBracketBlock => Some("]"),
                _ => None,
            };
            if let Some(close) = close {
                let verbatim = matches!(token, Token::SquareBracketBlock);
                let _ = input.parse_nested_block(|i| {
                    if verbatim {
                        let start = i.position();
                        while i.next_including_whitespace_and_comments().is_ok() {}
                        out.push_str(i.slice_from(start));
                    } else {
                        ok &= rewrite(i, out);
                    }
                    Ok::<_, cssparser::ParseError<()>>(())
                });
                out.push_str(close);
            }
        }
        colons = if matches!(token, Token::Colon) {
            colons + 1
        } else {
            0
        };
    }
}

/// What `rewrite` writes for `token` when it follows `colons` colons, consuming the arguments of
/// a function it replaces: `::before` for pseudo-elements, as no query matches those, `:not(*)`
/// for `:-webkit-*`, and `:state()` names `match_non_ts_pseudo_class` answers for `:in-range` and
/// `:dir()`. Err when `:dir()`'s argument is malformed.
fn replacement(
    colons: u32,
    token: &cssparser::Token,
    input: &mut CssParser,
) -> Result<Option<String>, ()> {
    use cssparser::Token;
    let (Token::Ident(name) | Token::Function(name)) = token else {
        return Ok(None);
    };
    let name = name.to_ascii_lowercase();
    let function = matches!(token, Token::Function(_));
    let element = name.starts_with("-webkit-")
        || ["first-line", "cue", "cue-region", "target-text"].contains(&name.as_str());
    // Only `::cue()` and `::-webkit-*()` take an argument.
    let takes_argument = name == "cue" || name.starts_with("-webkit-");
    Ok(match (colons, name.as_str()) {
        (2, _) if element && (!function || takes_argument) => {
            if function {
                // The argument goes with the pseudo-element.
                let _ = input.parse_nested_block(|i| {
                    while i.next_including_whitespace_and_comments().is_ok() {}
                    Ok::<_, cssparser::ParseError<()>>(())
                });
            }
            Some("before".to_owned())
        }
        (1, "in-range") if !function => Some(format!("state({IN_RANGE})")),
        (1, n) if !function && n.starts_with("-webkit-") => Some("not(*)".to_owned()),
        (1, "dir") if function => Some(
            input
                .parse_nested_block(|i| {
                    let dir = i.expect_ident()?.to_ascii_lowercase();
                    Ok::<_, cssparser::ParseError<()>>(match dir.as_str() {
                        "ltr" => format!("state({DIR_LTR})"),
                        "rtl" => format!("state({DIR_RTL})"),
                        _ => "not(*)".to_owned(),
                    })
                })
                .map_err(|_| ())?,
        ),
        _ => None,
    })
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

/// Collects the `FORM_FLAGS` bits pseudo-classes read, inside `:is()`, `:not()` and `:has()` too.
struct FormBits(u32);

impl SelectorVisitor for FormBits {
    type Impl = SelectorImpl;

    fn visit_simple_selector(&mut self, c: &Component<SelectorImpl>) -> bool {
        if let Component::NonTSPseudoClass(pc) = c {
            self.0 |= form_bits(pc);
        }
        true
    }

    // The default skips `:has()` arguments.
    fn visit_relative_selector_list(&mut self, list: &[RelativeSelector<SelectorImpl>]) -> bool {
        list.iter().all(|r| r.selector.visit(self))
    }
}

/// `lang` is `value` or starts with `value` and a hyphen, ASCII case-insensitively.
pub(crate) fn lang_matches(lang: &str, value: &str) -> bool {
    lang.get(..value.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(value))
        && matches!(lang.as_bytes().get(value.len()), None | Some(b'-'))
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
    pub fn wrap(node: NodeRef<'a, Node>, state: &'a QueryState) -> Option<Self> {
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
        (&*self.el().name.ns == HTML_NS).then(|| self.local())
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

    pub fn in_state(&self, pc: StatePseudo) -> bool {
        use StatePseudo as P;
        let id = self.node.id();
        match pc {
            P::AnyLink => self.is_link(),
            P::Checked => match self.html_local() {
                Some("input") => {
                    matches!(self.input_type().as_str(), "checkbox" | "radio") && self.el().checked
                }
                Some("option") => self.el().checked,
                _ => false,
            },
            P::Default => self.is_default(),
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
            // shortcut: no pointer, so every focus shows a ring; track the last input modality
            // once a test tells mouse focus apart.
            P::Focus | P::FocusVisible => self.state.focus_path.first() == Some(&id),
            P::FocusWithin => self.state.focus_path.contains(&id),
            P::Target => self.state.target == Some(id),
        }
    }

    /// The stylo flags of every native state this element is in.
    pub fn element_state(&self) -> ElementState {
        let form = self.state.form_state.as_ref();
        STATE_FLAGS
            .iter()
            .filter(|(pc, _)| self.in_state(*pc))
            .fold(ElementState::DEFINED, |state, (_, flag)| state | *flag)
            | form
                .and_then(|f| f.get(&self.node.id()))
                .copied()
                .unwrap_or(ElementState::empty())
    }

    /// The `dir` of the nearest HTML element setting a known one is `rtl`.
    // shortcut: `auto` and `bdi` count as ltr; resolve them from the text once a page needs it.
    fn is_rtl(&self) -> bool {
        std::iter::successors(Some(*self), Element::parent_element)
            .filter(|e| e.html_local().is_some())
            .filter_map(|e| e.el().attr("dir"))
            .find_map(|dir| match dir.to_ascii_lowercase().as_str() {
                "rtl" => Some(true),
                "ltr" | "auto" => Some(false),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Checked-by-default controls, selected-by-default options and a form's default button.
    fn is_default(&self) -> bool {
        match self.html_local() {
            Some("input") if matches!(self.input_type().as_str(), "checkbox" | "radio") => {
                self.has_attr("checked")
            }
            Some("option") => self.has_attr("selected"),
            Some("input" | "button") if self.is_submit_button() => {
                // shortcut: the form owner is the nearest ancestor form; honour `form=` once a
                // page relies on it.
                let Some(form) = self
                    .node
                    .ancestors()
                    .filter_map(|n| Self::wrap(n, self.state))
                    .find(|e| e.html_local() == Some("form"))
                else {
                    return false;
                };
                let first = form
                    .node
                    .descendants()
                    .filter_map(|n| Self::wrap(n, self.state))
                    .find(Self::is_submit_button);
                first.is_some_and(|b| b.node.id() == self.node.id())
            }
            _ => false,
        }
    }

    fn is_submit_button(&self) -> bool {
        match self.html_local() {
            Some("input") => matches!(self.input_type().as_str(), "submit" | "image"),
            Some("button") => !matches!(
                self.el()
                    .attr("type")
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("reset" | "button")
            ),
            _ => false,
        }
    }

    /// The nearest `lang` attribute, `xml:lang` first, on this element or an ancestor.
    fn lang(&self) -> Option<&'a str> {
        std::iter::successors(Some(*self), Element::parent_element).find_map(|e| {
            let attr = |ns: &str| {
                e.el()
                    .attrs
                    .iter()
                    .find(|a| &*a.name.ns == ns && &*a.name.local == "lang")
            };
            attr("http://www.w3.org/XML/1998/namespace")
                .or_else(|| attr(""))
                .map(|a| &*a.value)
        })
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
    type Impl = SelectorImpl;

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
        self.state.html && &*self.el().name.ns == HTML_NS
    }

    fn has_local_name(&self, name: &LocalName) -> bool {
        self.el().name.local == *name
    }

    fn has_namespace(&self, namespace: &Namespace) -> bool {
        self.el().name.ns == *namespace
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
        self.el().attrs.iter().any(|a| {
            !matches!(*ns, NamespaceConstraint::Specific(url) if url.0 != a.name.ns)
                && local_name.0 == a.name.local
                && operation.eval_str(&a.value)
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &NonTSPseudoClass,
        _context: &mut MatchingContext<'_, SelectorImpl>,
    ) -> bool {
        match pc {
            NonTSPseudoClass::Lang(value) => {
                return self.lang().is_some_and(|lang| lang_matches(lang, value));
            }
            NonTSPseudoClass::CustomState(state) => {
                return match &*state.0.0 {
                    IN_RANGE => self.element_state().intersects(ElementState::INRANGE),
                    DIR_LTR => !self.is_rtl(),
                    DIR_RTL => self.is_rtl(),
                    _ => false,
                };
            }
            _ => {}
        }
        let flag = pc.state_flag();
        !flag.is_empty() && self.element_state().intersects(flag)
    }

    fn match_pseudo_element(
        &self,
        _pe: &PseudoElement,
        _context: &mut MatchingContext<'_, SelectorImpl>,
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

    fn has_id(&self, id: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        self.el()
            .id()
            .is_some_and(|own| case_sensitivity.eq(id.as_bytes(), own.as_bytes()))
    }

    fn has_class(&self, name: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        self.el().has_class(name, case_sensitivity)
    }

    fn has_custom_state(&self, _name: &AtomIdent) -> bool {
        false
    }

    fn imported_part(&self, _name: &AtomIdent) -> Option<AtomIdent> {
        None
    }

    fn is_part(&self, _name: &AtomIdent) -> bool {
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
    use super::{Declined, MatchSelector, QueryMode, QueryState};
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
        assert_eq!(closest(&mut dom, "b"), Ok(vec![b]));
        let hit = closest(&mut dom, "div, p").unwrap();
        assert_eq!(dom.nodes(&hit)[0].name.as_deref(), Some("P"));
        assert_eq!(
            dom.query(b, "p b", QueryMode::Matches, &html()),
            Ok(vec![b])
        );
        assert_eq!(dom.query(b, "p", QueryMode::Matches, &html()), Ok(vec![]));
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
    fn invalid_selectors_and_form_state_without_a_snapshot_are_declined() {
        let parse = |s| MatchSelector::parse(s).err();
        let bits = |s| MatchSelector::parse(s).map(|m| m.form_bits);
        for selector in [
            ":::",
            "div::??",
            ":totallyfake",
            "svg|rect",
            "[x=':in-range']:::",
            "p::first-line(x)",
            "::target-text(x)",
        ] {
            assert_eq!(parse(selector), Some(Declined::Invalid), "{selector}");
        }
        for (selector, want) in [
            (":invalid", 0b10),
            ("p:is(a, :not(:placeholder-shown))", 0b1_0000),
            (":has(:invalid), :valid", 0b11),
            (":nth-child(1 of :valid)", 0b1),
            ("input:IN-RANGE", 0b100),
            ("p:not(:in-range):out-of-range:indeterminate", 0b10_1100),
        ] {
            assert_eq!(bits(selector), Ok(want), "{selector}");
        }
        for selector in [
            ":hover, :lang(en)",
            "p:not(:dir(ltr))",
            ":-webkit-autofill",
            "p::first-line",
            "::CUE(b) , ::-webkit-scrollbar",
        ] {
            assert_eq!(bits(selector), Ok(0), "{selector}");
        }
        assert_eq!(parse(":dir(1)"), Some(Declined::Invalid));
        let (mut dom, root) = doc("<html><body><div></div></body></html>");
        assert_eq!(
            dom.query(root, ":::", QueryMode::All, &html()),
            Err(Declined::Invalid)
        );
        assert_eq!(
            dom.query(root, "div:valid", QueryMode::All, &html()),
            Err(Declined::NeedsState(0b1))
        );
        assert_eq!(dom.query(9999, "div", QueryMode::All, &html()), Ok(vec![]));
    }

    #[test]
    fn legal_but_unsupported_pseudos_parse_and_never_match() {
        let (mut dom, root) = doc("<html><body><a href='/'>x</a><p>y</p></body></html>");
        for selector in [":visited", ":modal", ":hover", "p::before", "p:after"] {
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
