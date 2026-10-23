use html5ever::{local_name, tendril::TendrilSink as _};
use markup5ever_rcdom::{NodeData, RcDom};

use super::*;

pub(super) fn markdown(source: &str) -> Option<HashSet<Resource>> {
    // Match TextView's default GFM options and use the same locked parser.
    let root = ::markdown::to_mdast(source, &::markdown::ParseOptions::gfm()).ok()?;
    let mut references = HashSet::new();
    let mut definitions = Vec::new();
    let mut identifiers = HashSet::new();
    let mut nodes = vec![&root];
    while let Some(node) = nodes.pop() {
        match node {
            ::markdown::mdast::Node::Image(image) => {
                references.insert(Resource::Uri(image.url.clone().into()));
            }
            ::markdown::mdast::Node::ImageReference(image) => {
                identifiers.insert(&image.identifier);
            }
            ::markdown::mdast::Node::Definition(definition) => {
                definitions.push(definition);
            }
            ::markdown::mdast::Node::Html(fragment) => {
                // TextView parses each fragment independently, not the whole document.
                for source in html_sources(&fragment.value) {
                    references.insert(Resource::Uri(source.clone().into()));
                    retain_minified_source(&source, &mut references);
                }
            }
            _ => {}
        }
        if let Some(children) = node.children() {
            nodes.extend(children);
        }
    }
    // Keep used image definitions even where TextView does not yet render them.
    // Link-only and unused definitions cannot own an image.
    for definition in definitions {
        if identifiers.contains(&definition.identifier) {
            references.insert(Resource::Uri(definition.url.clone().into()));
        }
    }
    Some(references)
}

fn html_sources(source: &str) -> Vec<String> {
    let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(source);
    let mut sources = Vec::new();
    // RcDom's node destructor drains descendants, so keep the root alive.
    let mut nodes = vec![dom.document.clone()];
    while let Some(node) = nodes.pop() {
        if let NodeData::Element { name, attrs, .. } = &node.data
            && name.local == local_name!("img")
            && let Some(source) = attrs
                .borrow()
                .iter()
                .find(|attribute| attribute.name.local == local_name!("src"))
        {
            sources.push(source.value.to_string());
        }
        nodes.extend(node.children.borrow().iter().cloned());
    }
    sources
}

fn retain_minified_source(source: &str, references: &mut HashSet<Resource>) {
    // TODO: Consume TextView's canonical image references once exposed; these
    // private minifier rules must otherwise be checked when gpui-base changes.
    // gpui-base's private HTML minifier trims attributes, collapses ASCII
    // whitespace runs, and may emit unquoted values. Retain both the original
    // DOM value and that spelling; parse unquoted values with HTML5, including
    // its entity handling, rather than decoding entities ourselves.
    let mut whitespace = false;
    let source: String = source
        .trim_matches(|character: char| character.is_ascii_whitespace())
        .chars()
        .filter(|character| {
            let previous = whitespace;
            whitespace = character.is_ascii_whitespace();
            !whitespace || !previous
        })
        .collect();
    if source
        .chars()
        .all(|character| !character.is_ascii_whitespace() && !matches!(character, '"' | '\'' | '='))
    {
        for source in html_sources(&format!("<img src={source}>")) {
            references.insert(Resource::Uri(source.into()));
        }
    } else {
        references.insert(Resource::Uri(source.into()));
    }
}
