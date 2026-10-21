use crate::{
    Error, Result, SafeWorkspace,
    io::{Budget, cut, read_text, scan},
    registry::{flag, limit_output, text},
};
use serde_json::{Map, Value, json};
use std::{ops::ControlFlow, path::Path};
use tree_sitter::{Node, ParseOptions, Parser};
pub(crate) fn source_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| {
            ["js", "ts", "jsx", "tsx", "mjs", "cjs", "mts", "cts"]
                .contains(&s.to_ascii_lowercase().as_str())
        })
}
fn kind(node: Node<'_>) -> Option<&'static str> {
    Some(match node.kind() {
        "function_declaration" | "generator_function_declaration" => "FunctionDeclaration",
        "class_declaration" | "abstract_class_declaration" => "ClassDeclaration",
        "interface_declaration" => "InterfaceDeclaration",
        "type_alias_declaration" => "TypeAliasDeclaration",
        "enum_declaration" => "EnumDeclaration",
        "variable_declarator" => "VariableDeclaration",
        "method_definition" => {
            let mut cursor = node.walk();
            if node
                .children(&mut cursor)
                .any(|child| matches!(child.kind(), "get" | "set"))
            {
                return None;
            }
            "MethodDeclaration"
        }
        "public_field_definition" | "field_definition" => "PropertyDeclaration",
        "internal_module" | "module" => "ModuleDeclaration",
        "method_signature" | "abstract_method_signature" => "MethodSignature",
        "property_signature" => "PropertySignature",
        _ => return None,
    })
}
fn exported(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "export_statement" => return true,
            "lexical_declaration" | "variable_declaration" => node = parent,
            _ => return false,
        }
    }
    false
}
fn analyze(
    path: &str,
    source: &str,
    query: Option<&str>,
    case: bool,
    b: &Budget,
) -> Result<(Vec<Value>, bool)> {
    b.check()?;
    let language = match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE.into(),
        _ => return Err(Error::UnsupportedLanguage),
    };
    let mut parser = Parser::new();
    parser.set_language(&language).map_err(|_| Error::Parse)?;
    let mut progress = |_: &tree_sitter::ParseState| {
        if b.check().is_ok() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    };
    let bytes = source.as_bytes();
    let mut input = |offset: usize, _| bytes.get(offset..).unwrap_or(&[]);
    let tree = parser
        .parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or_else(|| b.check().err().unwrap_or(Error::Parse))?;
    let mut nodes = vec![tree.root_node()];
    let mut symbols = vec![];
    let mut truncated = false;
    while let Some(node) = nodes.pop() {
        b.check()?;
        if symbols.len() >= 200 {
            truncated = true;
            break;
        }
        if let Some(kind) = kind(node)
            && let Some(name_node) = node.child_by_field_name("name")
            && [
                "identifier",
                "property_identifier",
                "type_identifier",
                "string",
            ]
            .contains(&name_node.kind())
        {
            let raw = name_node.utf8_text(bytes).map_err(|_| Error::Parse)?;
            let name = if name_node.kind() == "string" {
                raw.get(1..raw.len().saturating_sub(1)).unwrap_or("")
            } else {
                raw
            };
            let constructor = kind == "MethodDeclaration"
                && name == "constructor"
                && node
                    .parent()
                    .is_some_and(|parent| parent.kind() == "class_body");
            if !constructor
                && query.is_none_or(|q| {
                    if case {
                        name.contains(q)
                    } else {
                        name.to_lowercase().contains(&q.to_lowercase())
                    }
                })
            {
                symbols.push(json!({"name":cut(name,200),"kind":kind,"line":node.start_position().row+1,"endLine":node.end_position().row+1,"exported":exported(node)}));
            }
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        nodes.extend(children.into_iter().rev());
    }
    Ok((symbols, truncated))
}
pub(crate) fn execute(
    ws: &SafeWorkspace,
    name: &str,
    a: &Map<String, Value>,
    b: &Budget,
) -> Result<Value> {
    if name == "file_outline" {
        let path = text(a, "path", "");
        if !source_file(path) {
            return Err(Error::UnsupportedLanguage);
        }
        let (source, _) = read_text(ws, path, 262_144, b)?;
        let (symbols, truncated) = analyze(path, &source, None, false, b)?;
        return limit_output(
            json!({"path":ws.relative(&ws.resolve_safe_path(path,true)?)?,"symbols":symbols,"truncated":truncated,"analysis":"syntax-only"}),
        );
    }
    let root = ws.resolve_safe_path(text(a, "path", "."), true)?;
    let scan = scan(ws, &root, b, true, |_, _| false)?;
    let sources: Vec<_> = scan.files.iter().filter(|p| source_file(p)).collect();
    let mut truncated = scan.truncated || sources.len() > 100;
    let mut matches = vec![];
    let mut errors = vec![];
    let mut bytes = 0;
    let mut inspected = 0;
    for file in sources.into_iter().take(100) {
        b.check()?;
        if bytes >= 2_097_152 || matches.len() >= 100 {
            truncated = true;
            break;
        }
        let result = (|| {
            let (source, len) = read_text(ws, file, 262_144.min(2_097_152 - bytes), b)?;
            bytes += len;
            inspected += 1;
            analyze(
                file,
                &source,
                Some(text(a, "query", "").trim()),
                flag(a, "caseSensitive", false),
                b,
            )
        })();
        match result {
            Ok((symbols, limited)) => {
                truncated |= limited || matches.len() + symbols.len() > 100;
                let remaining = 100 - matches.len();
                for mut symbol in symbols.into_iter().take(remaining) {
                    symbol["path"] = (*file).clone().into();
                    matches.push(symbol);
                }
            }
            Err(e) => {
                b.check()?;
                bytes += 262_144;
                errors.push(json!({"path":file,"error":e.to_string()}));
                truncated = true;
            }
        }
    }
    limit_output(
        json!({"matches":matches,"inspected":inspected,"errors":errors,"truncated":truncated,"analysis":"syntax-only"}),
    )
}
