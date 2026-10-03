//! The arena's node type. Per-node state the DOM ops need lives on the node itself, so an
//! attribute write edits the element in place and nothing is looked up in a side table.

use ego_tree::NodeId;
use html5ever::{Attribute, QualName};
use selectors::attr::CaseSensitivity;

/// `wire` of a node no JS handle has been issued for yet.
pub const NO_WIRE: u32 = u32::MAX;

pub enum Node {
    Document(Document),
    Fragment(Fragment),
    Doctype(Doctype),
    Comment(Comment),
    Text(Text),
    ProcessingInstruction(ProcessingInstruction),
    Element(Element),
}

pub struct Document {
    pub wire: u32,
    /// An XML document; every other document is HTML. Selector matching needs this for its case
    /// rules.
    pub xml: bool,
}

pub struct Fragment {
    pub wire: u32,
}

pub struct Doctype {
    pub wire: u32,
    pub name: String,
    pub public_id: String,
    pub system_id: String,
}

pub struct Comment {
    pub wire: u32,
    pub comment: String,
}

pub struct Text {
    pub wire: u32,
    pub text: String,
    /// A CDATA section: text to selectors, dropped from HTML serialization.
    pub cdata: bool,
}

pub struct ProcessingInstruction {
    pub wire: u32,
    pub target: String,
    pub data: String,
}

pub struct Element {
    pub wire: u32,
    pub name: QualName,
    pub attrs: Vec<Attribute>,
    /// A `<template>`'s content fragment, a separate root as in jsdom.
    pub template_contents: Option<NodeId>,
    /// The `is` value, which serialization writes even without an `is` attribute.
    pub is_value: Option<String>,
}

impl Node {
    pub fn document(xml: bool) -> Self {
        Node::Document(Document { wire: NO_WIRE, xml })
    }

    pub fn fragment() -> Self {
        Node::Fragment(Fragment { wire: NO_WIRE })
    }

    pub fn doctype(name: &str, public_id: &str, system_id: &str) -> Self {
        Node::Doctype(Doctype {
            wire: NO_WIRE,
            name: name.into(),
            public_id: public_id.into(),
            system_id: system_id.into(),
        })
    }

    pub fn comment(comment: &str) -> Self {
        Node::Comment(Comment {
            wire: NO_WIRE,
            comment: comment.into(),
        })
    }

    pub fn text(text: &str, cdata: bool) -> Self {
        Node::Text(Text {
            wire: NO_WIRE,
            text: text.into(),
            cdata,
        })
    }

    pub fn processing_instruction(target: &str, data: &str) -> Self {
        Node::ProcessingInstruction(ProcessingInstruction {
            wire: NO_WIRE,
            target: target.into(),
            data: data.into(),
        })
    }

    pub fn element(name: QualName, attrs: Vec<Attribute>) -> Self {
        Node::Element(Element {
            wire: NO_WIRE,
            name,
            attrs,
            template_contents: None,
            is_value: None,
        })
    }

    pub fn wire_mut(&mut self) -> &mut u32 {
        match self {
            Node::Document(n) => &mut n.wire,
            Node::Fragment(n) => &mut n.wire,
            Node::Doctype(n) => &mut n.wire,
            Node::Comment(n) => &mut n.wire,
            Node::Text(n) => &mut n.wire,
            Node::ProcessingInstruction(n) => &mut n.wire,
            Node::Element(n) => &mut n.wire,
        }
    }

    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn is_element(&self) -> bool {
        matches!(self, Node::Element(_))
    }

    pub fn is_document(&self) -> bool {
        matches!(self, Node::Document(_))
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Node::Text(_))
    }
}

impl Element {
    /// A no-namespace attribute's value.
    pub fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name.ns.is_empty() && &*a.name.local == local)
            .map(|a| &*a.value)
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    pub fn has_class(&self, class: &str, case_sensitivity: CaseSensitivity) -> bool {
        self.attr("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|c| case_sensitivity.eq(c.as_bytes(), class.as_bytes()))
        })
    }
}
