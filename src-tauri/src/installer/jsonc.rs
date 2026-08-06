//! JSONC parsing helpers built on top of `jsonc-parser`.
//!
//! This module is the only place that touches the Agent hook config syntax tree.
//! It exposes structural lookups over byte-indexed AST ranges; callers splice
//! source ranges and never search for braces or keys with regex or substring
//! matching (design 9.2/9.4).

use jsonc_parser::ast::Object;
use jsonc_parser::common::Range;
use jsonc_parser::{CollectOptions, CommentCollectionStrategy, ParseOptions};

use crate::error::{AppError, ErrorDomain};

/// Parse a JSONC/JSON source string into the `jsonc-parser` AST, collecting
/// comments so callers may preserve them. Returns a configuration-domain error
/// when the source is not valid JSONC.
pub fn parse(source: &str) -> Result<jsonc_parser::ast::Value<'_>, AppError> {
    let result = jsonc_parser::parse_to_ast(
        source,
        &CollectOptions {
            comments: CommentCollectionStrategy::Separate,
            tokens: false,
        },
        &ParseOptions {
            allow_comments: true,
            allow_trailing_commas: true,
            allow_loose_object_property_names: true,
        },
    )
    .map_err(|error| invalid_config(format!("jsonc parse failed: {error}")))?;
    result
        .value
        .ok_or_else(|| invalid_config("missing root value".into()))
}

/// Re-validate a fully-assembled document before returning it to the caller
/// (design 9.4 "validate before returning"). Comments are not needed here.
pub fn validate(source: &str) -> Result<(), AppError> {
    jsonc_parser::parse_to_ast(
        source,
        &CollectOptions {
            comments: CommentCollectionStrategy::Off,
            tokens: false,
        },
        &ParseOptions {
            allow_comments: true,
            allow_trailing_commas: true,
            allow_loose_object_property_names: true,
        },
    )
    .map_err(|error| invalid_config(format!("jsonc validate failed: {error}")))?;
    Ok(())
}

/// Byte-indexed range of a `jsonc-parser` AST value.
pub fn value_range(value: &jsonc_parser::ast::Value) -> Range {
    use jsonc_parser::ast::Value;
    match value {
        Value::Object(node) => node.range,
        Value::Array(node) => node.range,
        Value::StringLit(node) => node.range,
        Value::NumberLit(node) => node.range,
        Value::BooleanLit(node) => node.range,
        Value::NullKeyword(node) => node.range,
    }
}

/// Locate the top-level `hooks` object property on the root object, if any.
pub fn hooks_object<'a>(root: &'a jsonc_parser::ast::Value) -> Option<&'a Object<'a>> {
    root.as_object()
        .and_then(|object| object.get("hooks"))
        .and_then(|prop| prop.value.as_object())
}

fn invalid_config(message: String) -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: "configuration.invalid_jsonc".to_owned(),
        message,
        suggested_action: None,
    }
}
