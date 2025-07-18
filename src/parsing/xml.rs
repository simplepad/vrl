//! XML parsing utilities. Exports functions and configuration structs
//! that are sufficient to process a `roxmltree::Node`.

use crate::compiler::prelude::*;
use regex::{Regex, RegexBuilder};
// Re-export `roxmltree` to match the public API of `process_node`.
use roxmltree::NodeType;
pub use roxmltree::{Document, Node};
use rust_decimal::prelude::Zero;
use std::sync::LazyLock;
use std::{
    borrow::Cow,
    collections::{BTreeMap, btree_map::Entry},
};

/// A lazily initialized regular expression that matches excess whitespace between XML/HTML tags.
///
/// This regex helps in cleaning up formatted or pretty-printed XML/HTML by removing unnecessary
/// spaces, newlines, or indentation between tags. It specifically looks for occurrences where
/// a `>` (closing tag or self-closing tag) is immediately followed by whitespace (spaces, tabs,
/// or newlines) and then a `<` (opening tag).
///
/// ## Notes
/// - This regex is compiled once and reused, improving performance.
/// - The `multi_line(true)` flag (if used with `RegexBuilder`) ensures it applies across multiple lines.
/// - This is particularly useful for XML minification or normalization before processing.
pub static XML_RE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r">\s+?<")
        .multi_line(true)
        .build()
        .expect("trim regex failed")
});

/// Configuration to determine which XML options will be used when
/// parsing a roxmltree `Node`.
#[derive(Debug, Clone)]
pub struct ParseXmlConfig<'a> {
    /// Include XML attributes. Default: true,
    pub include_attr: bool,
    /// XML attribute prefix, e.g. `<a href="test">` -> `{a: { "@href": "test }}`. Default: "@".
    pub attr_prefix: Cow<'a, str>,
    /// Key to use for text nodes when attributes are included. Default: "text".
    pub text_key: Cow<'a, str>,
    /// Always use text default (instead of flattening). Default: false.
    pub always_use_text_key: bool,
    /// Parse "true" or "false" as booleans. Default: true.
    pub parse_bool: bool,
    /// Parse "null" as null. Default: true.
    pub parse_null: bool,
    /// Parse numeric values as integers/floats. Default: true.
    pub parse_number: bool,
}

/// Used to keep Clippy's `too_many_argument` check happy.
#[derive(Debug, Default)]
pub struct ParseOptions {
    pub trim: Option<Value>,
    pub include_attr: Option<Value>,
    pub attr_prefix: Option<Value>,
    pub text_key: Option<Value>,
    pub always_use_text_key: Option<Value>,
    pub parse_bool: Option<Value>,
    pub parse_null: Option<Value>,
    pub parse_number: Option<Value>,
}

/// Parses an XML string into a structured `Resolved` format based on the provided `ParseOptions`.
///
/// This function processes an XML document, applying transformations and extracting elements
/// according to the given parsing options.
///
/// # Parameters
/// - `value`: A [`vrl::value::Value`] containing the XML string to be parsed.
/// - `options`: A `ParseOptions` struct that defines parsing behaviors, including:
///   - `trim`: Whether to remove excess whitespace between XML elements (default: `true`).
///   - `include_attr`: Whether to include XML attributes in the output (default: `true`).
///   - `attr_prefix`: The prefix used for attribute keys (default: `"@"`).
///   - `text_key`: The key used for text content within an element (default: `"text"`).
///   - `always_use_text_key`: Whether text values should always be wrapped in a text key (default: `false`).
///   - `parse_bool`: Whether to attempt parsing boolean values (default: `true`).
///   - `parse_null`: Whether to attempt parsing null values (default: `true`).
///   - `parse_number`: Whether to attempt parsing numeric values (default: `true`).
///
/// # Returns
/// - `Ok(Resolved)`: The structured representation of the parsed XML.
/// - `Err(String)`: If XML parsing fails or an error occurs during processing.
///
/// # Errors
/// - Returns an error if the input is not valid XML or if any step in processing fails.
pub fn parse_xml(value: Value, options: ParseOptions) -> Resolved {
    let string = value.try_bytes_utf8_lossy()?;
    let trim = match options.trim {
        Some(value) => value.try_boolean()?,
        None => true,
    };
    let include_attr = match options.include_attr {
        Some(value) => value.try_boolean()?,
        None => true,
    };
    let attr_prefix = match options.attr_prefix {
        Some(value) => Cow::from(value.try_bytes_utf8_lossy()?.into_owned()),
        None => Cow::from("@"),
    };
    let text_key = match options.text_key {
        Some(value) => Cow::from(value.try_bytes_utf8_lossy()?.into_owned()),
        None => Cow::from("text"),
    };
    let always_use_text_key = match options.always_use_text_key {
        Some(value) => value.try_boolean()?,
        None => false,
    };
    let parse_bool = match options.parse_bool {
        Some(value) => value.try_boolean()?,
        None => true,
    };
    let parse_null = match options.parse_null {
        Some(value) => value.try_boolean()?,
        None => true,
    };
    let parse_number = match options.parse_number {
        Some(value) => value.try_boolean()?,
        None => true,
    };
    let config = ParseXmlConfig {
        include_attr,
        attr_prefix,
        text_key,
        always_use_text_key,
        parse_bool,
        parse_null,
        parse_number,
    };
    // Trim whitespace around XML elements, if applicable.
    let parse = if trim { trim_xml(&string) } else { string };
    let doc = Document::parse(&parse).map_err(|e| format!("unable to parse xml: {e}"))?;
    let value = process_node(doc.root(), &config);
    Ok(value)
}

/// Process an XML `Node` and return a VRL `Value`.
pub fn process_node(node: Node, config: &ParseXmlConfig) -> Value {
    // Helper to recurse over a `Node`s children, and build an object.
    let recurse = |node: Node| -> ObjectMap {
        let mut map = BTreeMap::new();

        // Expand attributes, if required.
        if config.include_attr {
            for attr in node.attributes() {
                map.insert(
                    format!("{}{}", config.attr_prefix, attr.name()).into(),
                    attr.value().into(),
                );
            }
        }

        for n in node.children().filter(|n| n.is_element() || n.is_text()) {
            let name = match n.node_type() {
                NodeType::Element => n.tag_name().name().to_string().into(),
                NodeType::Text => config.text_key.to_string().into(),
                _ => unreachable!("shouldn't be other XML nodes"),
            };

            // Transform the node into a VRL `Value`.
            let value = process_node(n, config);

            // If the key already exists, add it. Otherwise, insert.
            match map.entry(name) {
                Entry::Occupied(mut entry) => {
                    let v = entry.get_mut();

                    // Push a value onto the existing array, or wrap in a `Value::Array`.
                    match v {
                        Value::Array(v) => v.push(value),
                        v => {
                            let prev = std::mem::replace(v, Value::Array(Vec::with_capacity(2)));
                            if let Value::Array(v) = v {
                                v.extend_from_slice(&[prev, value]);
                            }
                        }
                    };
                }
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
            }
        }

        map
    };

    match node.node_type() {
        NodeType::Root => Value::Object(recurse(node)),

        NodeType::Element => {
            match (
                config.always_use_text_key,
                node.attributes().len().is_zero(),
            ) {
                // If the node has attributes, *always* recurse to expand default keys.
                (_, false) if config.include_attr => Value::Object(recurse(node)),
                // If a text key should be used, always recurse.
                (true, true) => Value::Object(recurse(node)),
                // Otherwise, check the node count to determine what to do.
                _ => match node.children().count() {
                    // For a single node, 'flatten' the object if necessary.
                    1 => {
                        // Expect a single element.
                        let node = node.children().next().expect("expected 1 XML node");

                        // If the node is an element, treat it as an object.
                        if node.is_element() {
                            let mut map = BTreeMap::new();

                            map.insert(
                                node.tag_name().name().to_string().into(),
                                process_node(node, config),
                            );

                            Value::Object(map)
                        } else {
                            // Otherwise, 'flatten' the object by continuing processing.
                            process_node(node, config)
                        }
                    }
                    // For 2+ nodes, expand.
                    _ => Value::Object(recurse(node)),
                },
            }
        }
        NodeType::Text => process_text(node.text().expect("expected XML text node"), config),
        _ => unreachable!("shouldn't be other XML nodes"),
    }
}

/// Process a text node, and return the correct `Value` type based on config.
fn process_text<'a>(text: &'a str, config: &ParseXmlConfig<'a>) -> Value {
    match text {
        // Parse nulls.
        "" | "null" if config.parse_null => Value::Null,
        // Parse bools.
        "true" if config.parse_bool => true.into(),
        "false" if config.parse_bool => false.into(),
        // String numbers.
        _ if !config.parse_number => text.into(),
        // Parse numbers, falling back to string.
        _ => {
            // Attempt an integer first (effectively a subset of float).
            if let Ok(v) = text.parse::<i64>() {
                return v.into();
            }

            // Then a float.
            if let Ok(v) = text.parse::<f64>() {
                return Value::from_f64_or_zero(v);
            }

            // Fall back to string.
            text.into()
        }
    }
}

#[inline]
fn trim_xml(xml: &str) -> Cow<str> {
    XML_RE.replace_all(xml, "><")
}
