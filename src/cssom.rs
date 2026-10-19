//! The CSSOM's sheets and rules: stylo objects the page reaches by id. A JS sheet or rule keeps
//! its id, so a rule removed from its list still reads; JS frees an id once its object is
//! collected, and a released document takes its ids along. Runtime-only, as `css.rs` is.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use cssparser::Parser;
use deno_core::{OpState, op2};
use ego_tree::NodeId;
use selectors::parser::ParseRelative;
use serde_json::{Value, json};
use style::context::QuirksMode;
use style::media_queries::{MediaList, MediaQuery};
use style::parser::ParserContext;
use style::selector_parser::SelectorParser;
use style::servo_arc::Arc;
use style::shared_lock::{Locked, SharedRwLockReadGuard, ToCssWithGuard};
use style::stylesheets::import_rule::ImportLayer;
use style::stylesheets::keyframes_rule::{Keyframe, KeyframeSelectors};
use style::stylesheets::{
    AllowImportRules, CssRule, CssRuleType, CssRuleTypes, CssRules, CustomMediaEvaluator,
    DocumentStyleSheet, Origin, RulesMutateError, StylesheetContents,
};
use style::values::KeyframesName;
use style_traits::{ParsingMode, ToCss};

use crate::css::{
    CssState, ImportLoader, Source, author_sheet, block_text, device, lock, parse_block,
    preprocess, sheet, sweep, url_data,
};
use crate::dom::DomState;

/// What an id names.
enum Item {
    Sheet {
        sheet: DocumentStyleSheet,
        /// From the process-wide cache, so copied before anything writes to it or hands out its
        /// rules.
        shared: bool,
        disabled: bool,
        constructed: bool,
    },
    Rule {
        rule: CssRule,
        /// The sheet the rule sits in, which a nested parse reads its URL and namespaces from.
        contents: Arc<StylesheetContents>,
        /// The rules it sits inside, and the nearest style or scope rule among them.
        types: CssRuleTypes,
        relative: Option<CssRuleType>,
    },
    Keyframe {
        keyframe: Arc<Locked<Keyframe>>,
        contents: Arc<StylesheetContents>,
    },
}

struct Entry {
    doc: NodeId,
    item: Item,
}

/// Every sheet and rule the page holds, and the sheet each `<style>` or `<link>` owns.
#[derive(Default)]
pub struct Handles {
    entries: HashMap<u32, Entry>,
    owners: HashMap<NodeId, u32>,
    adopted: HashMap<NodeId, Vec<u32>>,
    next: u32,
}

impl Handles {
    /// The sheet `owner` contributes to the cascade, unless it is disabled.
    pub(crate) fn owned_sheet(&self, owner: NodeId) -> Option<DocumentStyleSheet> {
        match &self.entries.get(self.owners.get(&owner)?)?.item {
            Item::Sheet {
                sheet,
                disabled: false,
                ..
            } => Some(sheet.clone()),
            _ => None,
        }
    }

    pub(crate) fn adopted_sheets(
        &self,
        doc: NodeId,
    ) -> impl Iterator<Item = DocumentStyleSheet> + '_ {
        self.adopted
            .get(&doc)
            .into_iter()
            .flatten()
            .filter_map(|id| match &self.entries.get(id)?.item {
                Item::Sheet {
                    sheet,
                    disabled: false,
                    ..
                } => Some(sheet.clone()),
                _ => None,
            })
    }

    pub(crate) fn retain_docs(&mut self, live: impl Fn(NodeId) -> bool) {
        self.entries.retain(|_, e| live(e.doc));
        let entries = &self.entries;
        self.owners.retain(|_, id| entries.contains_key(id));
        self.adopted.retain(|doc, _| live(*doc));
        for ids in self.adopted.values_mut() {
            ids.retain(|id| entries.contains_key(id));
        }
    }

    fn add(&mut self, doc: NodeId, item: Item) -> u32 {
        self.next += 1;
        self.entries.insert(self.next, Entry { doc, item });
        self.next
    }
}

/// A rule list, with what parsing a rule into it needs.
struct List {
    rules: Arc<Locked<CssRules>>,
    contents: Arc<StylesheetContents>,
    types: CssRuleTypes,
    relative: Option<CssRuleType>,
}

fn list(item: &Item, guard: &SharedRwLockReadGuard) -> Option<List> {
    let (rule, contents, types, relative) = match item {
        Item::Sheet { sheet, .. } => {
            let contents = sheet.0.contents.read_with(guard).clone();
            return Some(List {
                rules: contents.rules.clone(),
                contents,
                types: CssRuleTypes::default(),
                relative: None,
            });
        }
        Item::Rule {
            rule,
            contents,
            types,
            relative,
        } => (rule, contents, types, relative),
        Item::Keyframe { .. } => return None,
    };
    let ty = rule.rule_type();
    let rules = match rule {
        CssRule::Style(r) => r.read_with(guard).rules.clone()?,
        CssRule::Media(r) => r.rules.clone(),
        CssRule::Supports(r) => r.rules.clone(),
        CssRule::LayerBlock(r) => r.rules.clone(),
        CssRule::Scope(r) => r.rules.clone(),
        CssRule::StartingStyle(r) => r.rules.clone(),
        CssRule::Import(r) => {
            let sheet = r.read_with(guard).stylesheet.as_sheet()?.clone();
            return list(
                &Item::Sheet {
                    sheet: DocumentStyleSheet(sheet),
                    shared: false,
                    disabled: false,
                    constructed: false,
                },
                guard,
            );
        }
        _ => return None,
    };
    let mut types = *types;
    types.insert(ty);
    let relative = match ty {
        CssRuleType::Style | CssRuleType::Scope => Some(ty),
        _ => *relative,
    };
    Some(List {
        rules,
        contents: contents.clone(),
        types,
        relative,
    })
}

/// The IDL interface a rule's JS object implements.
fn class(rule: &CssRule) -> &'static str {
    match rule {
        CssRule::Style(_) => "CSSStyleRule",
        CssRule::Import(_) => "CSSImportRule",
        CssRule::Media(_) => "CSSMediaRule",
        CssRule::FontFace(_) => "CSSFontFaceRule",
        CssRule::Keyframes(_) => "CSSKeyframesRule",
        CssRule::Namespace(_) => "CSSNamespaceRule",
        CssRule::Supports(_) => "CSSSupportsRule",
        CssRule::LayerBlock(_) => "CSSLayerBlockRule",
        CssRule::LayerStatement(_) => "CSSLayerStatementRule",
        CssRule::Scope(_) => "CSSScopeRule",
        CssRule::NestedDeclarations(_) => "CSSNestedDeclarations",
        // shortcut: `@starting-style` has no interface in jsdom and reads as its base, add one
        // once a page reads its own members.
        CssRule::StartingStyle(_) => "CSSGroupingRule",
        _ => "CSSRule",
    }
}

fn error_name(e: RulesMutateError) -> &'static str {
    match e {
        RulesMutateError::Syntax => "SyntaxError",
        RulesMutateError::IndexSize => "IndexSizeError",
        RulesMutateError::HierarchyRequest => "HierarchyRequestError",
        RulesMutateError::InvalidState => "InvalidStateError",
    }
}

fn css_text(item: &Item, guard: &SharedRwLockReadGuard) -> String {
    let mut s = String::new();
    let _ = match item {
        Item::Rule { rule, .. } => rule.to_css(guard, &mut s),
        Item::Keyframe { keyframe, .. } => keyframe.read_with(guard).to_css(guard, &mut s),
        Item::Sheet { .. } => Ok(()),
    };
    s
}

/// The declarations of a rule serialised as `name: value;` pairs, read back out of its text.
fn body(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(a), Some(b)) if a < b => text[a + 1..b].trim(),
        _ => "",
    }
}

/// The media list `item` owns: a sheet's, an `@media` rule's or an `@import`'s.
fn media_of(item: &Item, guard: &SharedRwLockReadGuard) -> Option<Arc<Locked<MediaList>>> {
    match item {
        Item::Sheet { sheet, .. } => Some(sheet.0.media.clone()),
        Item::Rule {
            rule: CssRule::Media(r),
            ..
        } => Some(r.media_queries.clone()),
        Item::Rule {
            rule: CssRule::Import(r),
            ..
        } => Some(r.read_with(guard).stylesheet.as_sheet()?.media.clone()),
        _ => None,
    }
}

fn get(item: &Item, field: &str, guard: &SharedRwLockReadGuard) -> Value {
    if let Some(media) = media_of(item, guard).filter(|_| field.starts_with("media")) {
        let media = media.read_with(guard);
        return match field {
            "mediaItems" => json!(
                media
                    .media_queries
                    .iter()
                    .map(|q| q.to_css_string())
                    .collect::<Vec<_>>()
            ),
            "mediaMatches" => json!(media.evaluate(
                &device(QuirksMode::NoQuirks),
                QuirksMode::NoQuirks,
                &mut CustomMediaEvaluator::none()
            )),
            _ => json!(media.to_css_string()),
        };
    }
    if field == "cssText" {
        return json!(css_text(item, guard));
    }
    let block = |b: &Arc<Locked<_>>| json!(block_text(b.read_with(guard)));
    let rule = match item {
        Item::Keyframe { keyframe, .. } => {
            let keyframe = keyframe.read_with(guard);
            return match field {
                "style" => block(&keyframe.block),
                "keyText" => json!(keyframe.selector.to_css_string()),
                _ => Value::Null,
            };
        }
        Item::Rule { rule, .. } => rule,
        Item::Sheet { .. } => return Value::Null,
    };
    match (rule, field) {
        (CssRule::Style(r), "selectorText") => {
            json!(cssparser::ToCss::to_css_string(
                &r.read_with(guard).selectors
            ))
        }
        (CssRule::Style(r), "style") => block(&r.read_with(guard).block),
        (CssRule::NestedDeclarations(r), "style") => block(&r.read_with(guard).block),
        // shortcut: font-face descriptors read back as declarations, so only those that are also
        // properties survive, give them their own block once a page reads `src`.
        (CssRule::FontFace(_), "style") => json!(body(&css_text(item, guard))),
        (CssRule::Supports(r), "conditionText") => {
            json!(r.condition.to_css_string())
        }
        (CssRule::Supports(r), "matches") => json!(r.enabled),
        (CssRule::Keyframes(r), "name") => json!(r.read_with(guard).name.as_atom().to_string()),
        (CssRule::LayerBlock(r), "name") => {
            json!(r.name.as_ref().map_or(String::new(), |n| n.to_css_string()))
        }
        (CssRule::LayerStatement(r), "nameList") => json!(
            r.names
                .iter()
                .map(|n| n.to_css_string())
                .collect::<Vec<_>>()
        ),
        (CssRule::Namespace(r), "prefix") => {
            json!(r.prefix.as_ref().map_or(String::new(), |p| p.to_string()))
        }
        (CssRule::Namespace(r), "namespaceURI") => json!(r.url.to_string()),
        (CssRule::Import(r), "href") => {
            let text = r.read_with(guard).url.to_css_string();
            let input = &text;
            json!(
                Parser::new(input)
                    .expect_url_or_string()
                    .map_or(String::new(), |u| u.to_string())
            )
        }
        (CssRule::Import(r), "resolvedHref") => {
            json!(r.read_with(guard).url.url().map(|u| u.as_str().to_owned()))
        }
        (CssRule::Import(r), "layerName") => match &r.read_with(guard).layer {
            ImportLayer::None => Value::Null,
            ImportLayer::Anonymous => json!(""),
            ImportLayer::Named(n) => json!(n.to_css_string()),
        },
        (CssRule::Import(r), "supportsText") => json!(
            r.read_with(guard)
                .supports
                .as_ref()
                .map(|s| s.condition.to_css_string())
        ),
        (CssRule::Scope(r), "start" | "end") => {
            let bound = if field == "start" {
                &r.bounds.start
            } else {
                &r.bounds.end
            };
            json!(bound.as_ref().map(cssparser::ToCss::to_css_string))
        }
        _ => Value::Null,
    }
}

fn parser_context<'a>(
    contents: &'a StylesheetContents,
    rule_type: CssRuleType,
) -> ParserContext<'a> {
    ParserContext::new(
        Origin::Author,
        &contents.url_data,
        Some(rule_type),
        ParsingMode::DEFAULT,
        contents.quirks_mode,
        std::borrow::Cow::Borrowed(&contents.namespaces),
        None,
        None,
        Default::default(),
    )
}

/// Writes `value` to `field`. Returns `false` when it does not parse, or a DOMException name for
/// a write the spec makes throw.
fn set(item: &Item, field: &str, value: &str, guard: SharedRwLockReadGuard) -> Value {
    let value = preprocess(value);
    if let Some(media) = media_of(item, &guard).filter(|_| field.starts_with("media")) {
        let contents = match item {
            Item::Sheet { sheet, .. } => sheet.0.contents.read_with(&guard).clone(),
            Item::Rule { contents, .. } | Item::Keyframe { contents, .. } => contents.clone(),
        };
        drop(guard);
        let context = parser_context(&contents, CssRuleType::Media);
        let mut w = lock().write();
        let list = media.write_with(&mut w);
        return match field {
            "mediaAppend" => {
                // stylo's own append ignores trailing junk; a bad medium must append "not all".
                let mut context = context;
                let parsed = MediaList::parse(&mut context, &mut Parser::new(&value)).media_queries;
                let query = match <[_; 1]>::try_from(parsed) {
                    Ok([query]) => query,
                    Err(_) => MediaQuery::never_matching(),
                };
                list.media_queries.retain(|q| q != &query);
                list.media_queries.push(query);
                json!(true)
            }
            "mediaDelete" => json!(list.delete_medium(&context, &value)),
            _ => {
                let mut context = context;
                let input = &value;
                *list = MediaList::parse(&mut context, &mut Parser::new(input));
                json!(true)
            }
        };
    }
    let (rule, contents, relative) = match item {
        Item::Keyframe { keyframe, contents } => {
            let parsed = match field {
                "style" => KeyframeOrBlock::Block(parse_block(
                    &value,
                    &contents.url_data,
                    CssRuleType::Keyframe,
                )),
                "keyText" => {
                    let input = &value;
                    match Parser::new(input).parse_entirely(KeyframeSelectors::parse) {
                        Ok(s) => KeyframeOrBlock::Selector(s),
                        Err(_) => return json!("SyntaxError"),
                    }
                }
                _ => return json!(false),
            };
            drop(guard);
            let mut w = lock().write();
            match parsed {
                KeyframeOrBlock::Block(b) => {
                    let block = keyframe.write_with(&mut w).block.clone();
                    *block.write_with(&mut w) = b;
                }
                KeyframeOrBlock::Selector(s) => keyframe.write_with(&mut w).selector = s,
            }
            return json!(true);
        }
        Item::Rule {
            rule,
            contents,
            relative,
            ..
        } => (rule, contents, relative),
        Item::Sheet { .. } => return json!(false),
    };
    let block = match (rule, field) {
        (CssRule::Style(r), "style") => Some(r.read_with(&guard).block.clone()),
        (CssRule::NestedDeclarations(r), "style") => Some(r.read_with(&guard).block.clone()),
        _ => None,
    };
    if let Some(block) = block {
        let parsed = parse_block(&value, &contents.url_data, CssRuleType::Style);
        drop(guard);
        *block.write_with(&mut lock().write()) = parsed;
        return json!(true);
    }
    let mut input = Parser::new(&value);
    match (rule, field) {
        (CssRule::Style(r), "selectorText") => {
            let parser = SelectorParser {
                stylesheet_origin: Origin::Author,
                namespaces: &contents.namespaces,
                url_data: &contents.url_data,
                for_supports_rule: false,
            };
            let relative = match relative {
                Some(CssRuleType::Style) => ParseRelative::ForNesting,
                Some(CssRuleType::Scope) => ParseRelative::ForScope,
                _ => ParseRelative::No,
            };
            let Ok(selectors) =
                input.parse_entirely(|i| selectors::SelectorList::parse(&parser, i, relative))
            else {
                return json!(false);
            };
            drop(guard);
            r.write_with(&mut lock().write()).selectors = selectors;
            json!(true)
        }
        (CssRule::Keyframes(r), "name") => {
            drop(guard);
            r.write_with(&mut lock().write()).name = KeyframesName::from_ident(&value);
            json!(true)
        }
        _ => json!(false),
    }
}

enum KeyframeOrBlock {
    Block(style::properties::PropertyDeclarationBlock),
    Selector(KeyframeSelectors),
}

/// Runs `f` on the op state's `CssState` and `DomState`.
fn with_css<R>(state: &mut OpState, f: impl FnOnce(&mut CssState, &DomState) -> R) -> R {
    let _gate = crate::css::GATE.write().unwrap_or_else(|e| e.into_inner());
    let mut css = state.try_take::<CssState>().unwrap_or_default();
    let out = f(&mut css, state.borrow::<DomState>());
    state.put(css);
    out
}

/// Runs `f` on the op state's `CssState` under the shared gate: `f` must only read through
/// `lock()`, as concurrent readers would panic on a write.
fn read_css<R>(state: &OpState, none: R, f: impl FnOnce(&CssState) -> R) -> R {
    let _gate = crate::css::GATE.read().unwrap_or_else(|e| e.into_inner());
    state.try_borrow::<CssState>().map_or(none, f)
}

impl CssState {
    /// The sheet `id` names, copied out of the cache first so a write stays its own.
    fn unshare(&mut self, id: u32) {
        let Some(Entry {
            doc,
            item: Item::Sheet { sheet, shared, .. },
        }) = self.handles.entries.get_mut(&id)
        else {
            return;
        };
        if *shared {
            *sheet = DocumentStyleSheet(Arc::new((*sheet.0).clone()));
            *shared = false;
            self.docs.remove(doc);
        }
    }

    fn entry(&self, id: u32) -> Option<&Entry> {
        self.handles.entries.get(&id)
    }

    /// A new id for each rule in `id`'s list, with its interface name.
    pub(crate) fn rules(&mut self, id: u32) -> Vec<(u32, &'static str)> {
        self.unshare(id);
        let Some(entry) = self.entry(id) else {
            return vec![];
        };
        let doc = entry.doc;
        let guard = lock().read();
        let items: Vec<(Item, &'static str)> = match &entry.item {
            Item::Rule {
                rule: CssRule::Keyframes(r),
                contents,
                ..
            } => r
                .read_with(&guard)
                .keyframes
                .iter()
                .map(|k| {
                    let item = Item::Keyframe {
                        keyframe: k.clone(),
                        contents: contents.clone(),
                    };
                    (item, "CSSKeyframeRule")
                })
                .collect(),
            item => list(item, &guard).map_or(vec![], |l| {
                l.rules
                    .read_with(&guard)
                    .0
                    .iter()
                    .map(|rule| {
                        let item = Item::Rule {
                            rule: rule.clone(),
                            contents: l.contents.clone(),
                            types: l.types,
                            relative: l.relative,
                        };
                        (item, class(rule))
                    })
                    .collect()
            }),
        };
        drop(guard);
        items
            .into_iter()
            .map(|(item, class)| (self.handles.add(doc, item), class))
            .collect()
    }

    /// Parses `text` into `id`'s list at `index`; the new rule's id and interface, or the
    /// DOMException name the insertion throws.
    pub(crate) fn insert(
        &mut self,
        id: u32,
        text: &str,
        index: usize,
    ) -> Result<(u32, &'static str), &'static str> {
        self.unshare(id);
        let text = preprocess(text);
        let entry = self.entry(id).ok_or("InvalidStateError")?;
        let doc = entry.doc;
        if let Item::Rule {
            rule: CssRule::Keyframes(r),
            contents,
            ..
        } = &entry.item
        {
            let keyframe = Keyframe::parse(&text, contents, lock()).map_err(|_| "SyntaxError")?;
            let item = Item::Keyframe {
                keyframe: keyframe.clone(),
                contents: contents.clone(),
            };
            let mut w = lock().write();
            let keyframes = &mut r.write_with(&mut w).keyframes;
            keyframes.insert(index.min(keyframes.len()), keyframe);
            drop(w);
            self.docs.remove(&doc);
            return Ok((self.handles.add(doc, item), "CSSKeyframeRule"));
        }
        // A style rule grows its nested list on the first insertion.
        if let Item::Rule {
            rule: CssRule::Style(r),
            ..
        } = &entry.item
        {
            let mut w = lock().write();
            let r = r.write_with(&mut w);
            if r.rules.is_none() {
                r.rules = Some(CssRules::new(vec![], lock()));
            }
        }
        let constructed = matches!(
            entry.item,
            Item::Sheet {
                constructed: true,
                ..
            }
        );
        let empty = BTreeMap::new();
        let guard = lock().read();
        let l = list(&entry.item, &guard).ok_or("HierarchyRequestError")?;
        let loader = ImportLoader {
            imports: self.imports.get(&doc).unwrap_or(&empty),
            quirks: l.contents.quirks_mode,
            chain: RefCell::new(vec![l.contents.url_data.as_str().to_owned()]),
        };
        let allow = if constructed {
            AllowImportRules::No
        } else {
            AllowImportRules::Yes
        };
        let rule = l
            .rules
            .read_with(&guard)
            .parse_rule_for_insert(
                lock(),
                &text,
                &l.contents,
                index,
                l.types,
                l.relative,
                Some(&loader),
                allow,
            )
            .map_err(error_name)?;
        drop(guard);
        l.rules
            .write_with(&mut lock().write())
            .0
            .insert(index, rule.clone());
        self.docs.remove(&doc);
        let class = class(&rule);
        let item = Item::Rule {
            rule,
            contents: l.contents,
            types: l.types,
            relative: l.relative,
        };
        Ok((self.handles.add(doc, item), class))
    }

    /// Removes the rule at `index` from `id`'s list; the DOMException name on failure.
    pub(crate) fn delete(&mut self, id: u32, index: usize) -> Result<(), &'static str> {
        self.unshare(id);
        let entry = self.entry(id).ok_or("InvalidStateError")?;
        let doc = entry.doc;
        if let Item::Rule {
            rule: CssRule::Keyframes(r),
            ..
        } = &entry.item
        {
            let mut w = lock().write();
            let keyframes = &mut r.write_with(&mut w).keyframes;
            if index < keyframes.len() {
                keyframes.remove(index);
            }
        } else {
            let guard = lock().read();
            let l = list(&entry.item, &guard).ok_or("IndexSizeError")?;
            drop(guard);
            let mut w = lock().write();
            let rules = &mut l.rules.write_with(&mut w).0;
            if index >= rules.len() {
                return Err("IndexSizeError");
            }
            // stylo also refuses when only `@layer` statements remain, which insertion accepts.
            let harmless = |r: &CssRule| {
                matches!(
                    r,
                    CssRule::Namespace(_) | CssRule::Import(_) | CssRule::LayerStatement(_)
                )
            };
            if matches!(rules[index], CssRule::Namespace(_)) && !rules.iter().all(harmless) {
                return Err("InvalidStateError");
            }
            rules.remove(index);
        }
        self.docs.remove(&doc);
        Ok(())
    }

    pub(crate) fn get(&self, id: u32, field: &str) -> Value {
        let guard = lock().read();
        self.entry(id)
            .map_or(Value::Null, |e| get(&e.item, field, &guard))
    }

    pub(crate) fn set(&mut self, id: u32, field: &str, value: &str) -> Value {
        self.unshare(id);
        let Some(entry) = self.entry(id) else {
            return json!(false);
        };
        let doc = entry.doc;
        let out = set(&entry.item, field, value, lock().read());
        self.docs.remove(&doc);
        out
    }

    /// Fills the `@import`s of `root`'s sheets that wait on `url` with `body`.
    fn import_loaded(&mut self, root: NodeId, url: String, body: String) {
        self.imports
            .entry(root)
            .or_default()
            .insert(url.clone(), body);
        let guard = lock().read();
        let mut waiting = vec![];
        for entry in self.handles.entries.values() {
            if let (Item::Sheet { sheet, .. }, true) = (&entry.item, entry.doc == root) {
                let contents = sheet.0.contents.read_with(&guard);
                let base = contents.url_data.as_str().to_owned();
                imports_of(
                    contents.rules(&guard),
                    &url,
                    &mut vec![base],
                    &guard,
                    &mut waiting,
                );
            }
        }
        drop(guard);
        waiting.sort_by_key(|(sheet, _)| &**sheet as *const _);
        waiting.dedup_by_key(|(sheet, _)| &**sheet as *const _);
        let imports = &self.imports[&root];
        for (sheet, chain) in waiting {
            let loader = ImportLoader {
                imports,
                quirks: QuirksMode::NoQuirks,
                chain: RefCell::new(chain),
            };
            let contents = StylesheetContents::from_str(
                &preprocess(&imports[&url]),
                url_data(&url),
                Origin::Author,
                lock(),
                Some(&loader),
                None,
                QuirksMode::NoQuirks,
                AllowImportRules::Yes,
                None,
            );
            *sheet.contents.write_with(&mut lock().write()) = contents;
        }
        self.docs.remove(&root);
    }
}

/// The sheets of the `@import`s in `rules` (and in the sheets they import) that load `url`, with
/// the chain of sheet URLs above each, which stops a sheet from importing itself.
fn imports_of(
    rules: &[CssRule],
    url: &str,
    chain: &mut Vec<String>,
    guard: &SharedRwLockReadGuard,
    out: &mut Vec<(Arc<style::stylesheets::Stylesheet>, Vec<String>)>,
) {
    for rule in rules {
        let CssRule::Import(r) = rule else {
            continue;
        };
        let r = r.read_with(guard);
        let (Some(sheet), Some(href)) = (r.stylesheet.as_sheet(), r.url.url()) else {
            continue;
        };
        let href = href.as_str();
        if chain.iter().any(|c| c == href) {
            continue;
        }
        chain.push(href.to_owned());
        if href == url {
            out.push((sheet.clone(), chain.clone()));
        } else {
            let contents = sheet.contents.read_with(guard);
            imports_of(contents.rules(guard), url, chain, guard, out);
        }
        chain.pop();
    }
}

/// A new sheet in the document `doc` names: `owner`'s (a `<style>` or `<link>` wire), or a
/// constructed one for `-1`. `base` resolves its URLs; `media` is its media list's text.
#[op2(fast)]
pub fn op_css_sheet_new(
    state: &mut OpState,
    doc: u32,
    owner: i32,
    #[string] text: &str,
    #[string] base: &str,
    #[string] media: &str,
) -> u32 {
    with_css(state, |css, dom| {
        let Some(root) = dom.resolve(doc) else {
            return 0;
        };
        sweep(css, dom);
        let owner = u32::try_from(owner).ok().and_then(|o| dom.resolve(o));
        css.sheet_new(root, owner, text, base, media)
    })
}

impl CssState {
    pub(crate) fn sheet_new(
        &mut self,
        root: NodeId,
        owner: Option<NodeId>,
        text: &str,
        base: &str,
        media: &str,
    ) -> u32 {
        let src = Source {
            css: text.to_owned(),
            base: base.to_owned(),
            media: media.to_owned(),
        };
        let empty = BTreeMap::new();
        let imports = self.imports.get(&root).unwrap_or(&empty);
        let (parsed, shared) = match owner {
            Some(_) => author_sheet(&src, QuirksMode::NoQuirks, imports),
            None => {
                let parsed = sheet(
                    &src,
                    Origin::Author,
                    QuirksMode::NoQuirks,
                    imports,
                    AllowImportRules::No,
                );
                (parsed, false)
            }
        };
        let item = Item::Sheet {
            sheet: parsed,
            shared,
            disabled: false,
            constructed: owner.is_none(),
        };
        let id = self.handles.add(root, item);
        if let Some(owner) = owner {
            self.handles.owners.insert(owner, id);
            self.docs.remove(&root);
        }
        id
    }
}

/// Replaces `doc`'s adopted constructed sheets, in cascade order.
#[op2]
pub fn op_css_sheet_adopt(state: &mut OpState, doc: u32, #[serde] ids: Vec<u32>) -> bool {
    with_css(state, |css, dom| {
        let Some(root) = dom.resolve(doc) else {
            return false;
        };
        if !ids.iter().all(|id| {
            matches!(
                css.handles.entries.get(id),
                Some(Entry {
                    doc,
                    item: Item::Sheet {
                        constructed: true,
                        ..
                    }
                }) if *doc == root
            )
        }) {
            return false;
        }
        css.handles.adopted.insert(root, ids);
        css.docs.remove(&root);
        true
    })
}

/// Sheet `id` leaves the cascade, as its owner dropped it.
#[op2(fast)]
pub fn op_css_sheet_detach(state: &mut OpState, id: u32) {
    with_css(state, |css, _| {
        css.handles.owners.retain(|_, v| *v != id);
        if let Some(e) = css.handles.entries.get(&id) {
            css.docs.remove(&e.doc);
        }
    })
}

#[op2(fast)]
pub fn op_css_sheet_set_disabled(state: &mut OpState, id: u32, disabled: bool) {
    with_css(state, |css, _| {
        if let Some(Entry {
            doc,
            item: Item::Sheet { disabled: d, .. },
        }) = css.handles.entries.get_mut(&id)
        {
            *d = disabled;
            css.docs.remove(doc);
        }
    })
}

/// Replaces constructed sheet `id`'s rules with `text`'s, which may hold no `@import`.
#[op2(fast)]
pub fn op_css_sheet_replace(state: &mut OpState, id: u32, #[string] text: &str) {
    with_css(state, |css, _| {
        css.unshare(id);
        let Some(Entry {
            doc,
            item: Item::Sheet { sheet, .. },
        }) = css.handles.entries.get(&id)
        else {
            return;
        };
        let url_data = sheet.0.contents.read_with(&lock().read()).url_data.clone();
        let contents = StylesheetContents::from_str(
            &preprocess(text),
            url_data,
            Origin::Author,
            lock(),
            None,
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::No,
            None,
        );
        *sheet.0.contents.write_with(&mut lock().write()) = contents;
        let doc = *doc;
        css.docs.remove(&doc);
    })
}

/// The rules in `id`'s list (a sheet's, a grouping rule's, an `@import`'s sheet's or the
/// keyframes of `@keyframes`), each as a new id and its interface name.
#[op2]
#[serde]
pub fn op_css_rules(state: &mut OpState, id: u32) -> Vec<(u32, &'static str)> {
    with_css(state, |css, _| css.rules(id))
}

/// `[id, interface]` of the rule `text` inserts at `index`, or `[0, DOMException name]`.
#[op2]
#[serde]
pub fn op_css_rule_insert(
    state: &mut OpState,
    id: u32,
    #[string] text: &str,
    index: u32,
) -> (u32, &'static str) {
    with_css(state, |css, _| {
        css.insert(id, text, index as usize)
            .unwrap_or_else(|name| (0, name))
    })
}

/// `""`, or the DOMException name removing the rule at `index` throws.
#[op2]
#[string]
pub fn op_css_rule_delete(state: &mut OpState, id: u32, index: u32) -> &'static str {
    with_css(state, |css, _| {
        css.delete(id, index as usize).err().unwrap_or("")
    })
}

/// The index of the last keyframe of `@keyframes` `id` that `select` names, or -1.
#[op2(fast)]
pub fn op_css_find_keyframe(state: &mut OpState, id: u32, #[string] select: &str) -> i32 {
    read_css(state, -1, |css| {
        let guard = lock().read();
        match css.entry(id).map(|e| &e.item) {
            Some(Item::Rule {
                rule: CssRule::Keyframes(r),
                ..
            }) => r
                .read_with(&guard)
                .find_rule(&guard, select)
                .map_or(-1, |i| i as i32),
            _ => -1,
        }
    })
}

/// A field of sheet or rule `id`, by its IDL attribute name; `media*` fields read its media list.
#[op2]
#[serde]
pub fn op_css_get(state: &mut OpState, id: u32, #[string] field: &str) -> serde_json::Value {
    read_css(state, Value::Null, |css| css.get(id, field))
}

#[op2]
#[serde]
pub fn op_css_set(
    state: &mut OpState,
    id: u32,
    #[string] field: &str,
    #[string] value: &str,
) -> serde_json::Value {
    with_css(state, |css, _| css.set(id, field, value))
}

/// The JS object for `id` was collected.
#[op2(fast)]
pub fn op_css_release(state: &mut OpState, id: u32) {
    with_css(state, |css, _| {
        css.handles.entries.remove(&id);
        css.handles.owners.retain(|_, v| *v != id);
        for (doc, ids) in &mut css.handles.adopted {
            if ids.contains(&id) {
                ids.retain(|adopted| *adopted != id);
                css.docs.remove(doc);
            }
        }
    })
}

/// An `@import` the page fetched for a sheet in `owner`'s document; `url` is where it came from.
#[op2(fast)]
pub fn op_css_import_loaded(
    state: &mut OpState,
    owner: u32,
    #[string] url: &str,
    #[string] body: &str,
) {
    with_css(state, |css, dom| {
        let Some(node) = dom.resolve(owner).and_then(|id| dom.arena.get(id)) else {
            return;
        };
        let root = node.ancestors().last().unwrap_or(node).id();
        css.import_loaded(root, url.to_owned(), body.to_owned());
    })
}
