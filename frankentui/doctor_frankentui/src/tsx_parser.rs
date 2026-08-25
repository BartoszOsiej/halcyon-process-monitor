// SPDX-License-Identifier: Apache-2.0
//! Lightweight TSX/JSX parser pipeline for migration analysis.
//!
//! Extracts a canonical typed AST representation from React/OpenTUI source
//! files using regex-based scanning. Builds a symbol table linking identifiers
//! to declarations across module boundaries.
//!
//! This is not a full parser — it extracts the semantic structures needed for
//! migration planning: components, hooks, props contracts, event handlers,
//! JSX element trees, and type annotations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex_lite::Regex;
use serde::{Deserialize, Serialize};

// ── AST Types ────────────────────────────────────────────────────────────

/// Unique identifier for a symbol within a file.
pub type SymbolId = String;

/// The kind of a component declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentKind {
    /// `function Foo() {}` or `const Foo = () => {}`
    FunctionComponent,
    /// `class Foo extends React.Component`
    ClassComponent,
    /// `React.forwardRef(...)` wrapper
    ForwardRef,
    /// `React.memo(...)` wrapper
    Memo,
}

/// A detected React/OpenTUI component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentDecl {
    pub name: String,
    pub kind: ComponentKind,
    pub is_default_export: bool,
    pub is_named_export: bool,
    pub props_type: Option<String>,
    pub hooks: Vec<HookCall>,
    pub event_handlers: Vec<EventHandler>,
    pub line: usize,
}

/// A React hook invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookCall {
    pub name: String,
    pub binding: Option<String>,
    pub args_snippet: String,
    pub line: usize,
}

/// An event handler detected in JSX or component body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventHandler {
    pub event_name: String,
    pub handler_name: Option<String>,
    pub is_inline: bool,
    pub line: usize,
}

/// A JSX element reference extracted from render output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsxElement {
    pub tag: String,
    pub is_component: bool,
    pub is_fragment: bool,
    pub is_self_closing: bool,
    pub props: Vec<JsxProp>,
    pub line: usize,
}

/// A prop on a JSX element.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsxProp {
    pub name: String,
    pub is_spread: bool,
    pub value_snippet: Option<String>,
}

/// A type/interface declaration relevant to props or state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDecl {
    pub name: String,
    pub kind: TypeDeclKind,
    pub fields: Vec<TypeField>,
    pub is_exported: bool,
    pub line: usize,
}

/// Kind of type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeDeclKind {
    Interface,
    TypeAlias,
    Enum,
}

/// A field within a type/interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeField {
    pub name: String,
    pub type_annotation: String,
    pub optional: bool,
}

/// A symbol table entry linking an identifier to its declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolEntry {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub file: String,
    pub line: usize,
    pub is_exported: bool,
    pub imported_from: Option<ImportSource>,
}

/// What kind of symbol this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Component,
    Hook,
    Type,
    Constant,
    Function,
    Variable,
    Enum,
    Namespace,
}

/// Where an imported symbol comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSource {
    pub specifier: String,
    pub original_name: Option<String>,
}

/// A parse diagnostic for recoverable errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub file: String,
    pub line: Option<usize>,
    pub code: String,
}

/// Severity level for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
    Info,
}

// ── File-Level Parse Result ──────────────────────────────────────────────

/// Complete parse result for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileParse {
    pub file: String,
    pub components: Vec<ComponentDecl>,
    pub hooks: Vec<HookCall>,
    pub jsx_elements: Vec<JsxElement>,
    pub types: Vec<TypeDecl>,
    pub symbols: Vec<SymbolEntry>,
    pub diagnostics: Vec<ParseDiagnostic>,
}

// ── Project-Level Result ─────────────────────────────────────────────────

/// Complete parse result for an entire project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectParse {
    pub files: BTreeMap<String, FileParse>,
    #[serde(default)]
    pub file_contents: BTreeMap<String, String>,
    pub symbol_table: BTreeMap<SymbolId, SymbolEntry>,
    pub component_count: usize,
    pub hook_usage_count: usize,
    pub type_count: usize,
    pub diagnostics: Vec<ParseDiagnostic>,
    pub external_imports: BTreeSet<String>,
}

// ── Parsing Implementation ───────────────────────────────────────────────

/// Parse a single TSX/JSX/TS/JS file and extract its AST representation.
pub fn parse_file(content: &str, file_path: &str) -> FileParse {
    let mut result = FileParse {
        file: file_path.to_string(),
        components: Vec::new(),
        hooks: Vec::new(),
        jsx_elements: Vec::new(),
        types: Vec::new(),
        symbols: Vec::new(),
        diagnostics: Vec::new(),
    };

    extract_components(content, file_path, &mut result);
    extract_hooks(content, file_path, &mut result);
    extract_jsx_elements(content, file_path, &mut result);
    extract_types(content, file_path, &mut result);
    extract_symbols(content, file_path, &mut result);

    result
}

/// Parse all files in a project snapshot directory.
pub fn parse_project(snapshot_root: &Path, files: &[String]) -> ProjectParse {
    let mut project = ProjectParse {
        files: BTreeMap::new(),
        file_contents: BTreeMap::new(),
        symbol_table: BTreeMap::new(),
        component_count: 0,
        hook_usage_count: 0,
        type_count: 0,
        diagnostics: Vec::new(),
        external_imports: BTreeSet::new(),
    };

    for file_rel in files {
        let full_path = snapshot_root.join(file_rel);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                project.diagnostics.push(ParseDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!("Failed to read file: {e}"),
                    file: file_rel.clone(),
                    line: None,
                    code: "E001".to_string(),
                });
                continue;
            }
        };

        let file_parse = parse_file(&content, file_rel);
        project.file_contents.insert(file_rel.clone(), content);

        project.component_count += file_parse.components.len();
        project.hook_usage_count += file_parse.hooks.len();
        project.type_count += file_parse.types.len();
        project.diagnostics.extend(file_parse.diagnostics.clone());

        // Merge symbols into project-level table.
        for sym in &file_parse.symbols {
            project.symbol_table.insert(sym.id.clone(), sym.clone());
        }

        project.files.insert(file_rel.clone(), file_parse);
    }

    // Cross-file symbol resolution.
    resolve_cross_file_symbols(&mut project);

    project
}

// ── Component Extraction ─────────────────────────────────────────────────

fn extract_components(content: &str, file_path: &str, result: &mut FileParse) {
    // Function components: export (default)? function ComponentName(props: Type)
    let re_func_comp = Regex::new(
        r#"(?m)^[^\S\n]*(export\s+)?(default\s+)?function\s+([A-Z]\w*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)"#,
    )
    .expect("func comp regex");

    // Arrow function components: export (default)? const ComponentName = ... => {
    let re_arrow_comp = Regex::new(
        r#"(?m)^[^\S\n]*(export\s+)?(default\s+)?(?:const|let)\s+([A-Z]\w*)\s*(?::\s*React\.FC[^=]*)?\s*=\s*(?:\([^)]*\)|[^=])*=>"#,
    )
    .expect("arrow comp regex");

    // Class components: class ComponentName extends (React.)?Component
    let re_class_comp = Regex::new(
        r#"(?m)^[^\S\n]*(export\s+)?(default\s+)?class\s+([A-Z]\w*)\s+extends\s+(?:React\.)?(?:Component|PureComponent)"#,
    )
    .expect("class comp regex");

    // ForwardRef: React.forwardRef<...>(...) or forwardRef(...)
    let re_forward_ref = Regex::new(
        r#"(?m)^[^\S\n]*(export\s+)?(default\s+)?(?:const|let)\s+([A-Z]\w*)\s*=\s*(?:React\.)?forwardRef"#,
    )
    .expect("forward ref regex");

    // React.memo: React.memo(...)
    let re_memo = Regex::new(
        r#"(?m)^[^\S\n]*(export\s+)?(default\s+)?(?:const|let)\s+([A-Z]\w*)\s*=\s*(?:React\.)?memo"#,
    )
    .expect("memo regex");

    for (line_idx, line) in content.lines().enumerate() {
        let lineno = line_idx + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }

        // Function component.
        if let Some(caps) = re_func_comp.captures(line) {
            let is_export = caps.get(1).is_some();
            let is_default = caps.get(2).is_some();
            let name = caps[3].to_string();
            let params = caps.get(4).map(|m| m.as_str()).unwrap_or("");
            let props_type = extract_props_type(params);

            result.components.push(ComponentDecl {
                name: name.clone(),
                kind: ComponentKind::FunctionComponent,
                is_default_export: is_default,
                is_named_export: is_export && !is_default,
                props_type,
                hooks: Vec::new(),
                event_handlers: Vec::new(),
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name: name.clone(),
                kind: SymbolKind::Component,
                file: file_path.to_string(),
                line: lineno,
                is_exported: is_export,
                imported_from: None,
            });
            continue;
        }

        // ForwardRef (must check before arrow component to avoid false match).
        if let Some(caps) = re_forward_ref.captures(line) {
            let is_export = caps.get(1).is_some();
            let is_default = caps.get(2).is_some();
            let name = caps[3].to_string();

            result.components.push(ComponentDecl {
                name: name.clone(),
                kind: ComponentKind::ForwardRef,
                is_default_export: is_default,
                is_named_export: is_export && !is_default,
                props_type: None,
                hooks: Vec::new(),
                event_handlers: Vec::new(),
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name: name.clone(),
                kind: SymbolKind::Component,
                file: file_path.to_string(),
                line: lineno,
                is_exported: is_export,
                imported_from: None,
            });
            continue;
        }

        // Memo.
        if let Some(caps) = re_memo.captures(line) {
            let is_export = caps.get(1).is_some();
            let is_default = caps.get(2).is_some();
            let name = caps[3].to_string();

            result.components.push(ComponentDecl {
                name: name.clone(),
                kind: ComponentKind::Memo,
                is_default_export: is_default,
                is_named_export: is_export && !is_default,
                props_type: None,
                hooks: Vec::new(),
                event_handlers: Vec::new(),
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name: name.clone(),
                kind: SymbolKind::Component,
                file: file_path.to_string(),
                line: lineno,
                is_exported: is_export,
                imported_from: None,
            });
            continue;
        }

        // Class component.
        if let Some(caps) = re_class_comp.captures(line) {
            let is_export = caps.get(1).is_some();
            let is_default = caps.get(2).is_some();
            let name = caps[3].to_string();

            result.components.push(ComponentDecl {
                name: name.clone(),
                kind: ComponentKind::ClassComponent,
                is_default_export: is_default,
                is_named_export: is_export && !is_default,
                props_type: None,
                hooks: Vec::new(),
                event_handlers: Vec::new(),
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name: name.clone(),
                kind: SymbolKind::Component,
                file: file_path.to_string(),
                line: lineno,
                is_exported: is_export,
                imported_from: None,
            });
            continue;
        }

        // Arrow function component (checked after forwardRef/memo to avoid false matches).
        if let Some(caps) = re_arrow_comp.captures(line) {
            let is_export = caps.get(1).is_some();
            let is_default = caps.get(2).is_some();
            let name = caps[3].to_string();

            result.components.push(ComponentDecl {
                name: name.clone(),
                kind: ComponentKind::FunctionComponent,
                is_default_export: is_default,
                is_named_export: is_export && !is_default,
                props_type: None,
                hooks: Vec::new(),
                event_handlers: Vec::new(),
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name: name.clone(),
                kind: SymbolKind::Component,
                file: file_path.to_string(),
                line: lineno,
                is_exported: is_export,
                imported_from: None,
            });
            continue;
        }
    }

    // If no components found but file has JSX, emit a diagnostic.
    if result.components.is_empty() && content.contains("</") && content.contains("return") {
        let has_jsx = Regex::new(r"<[A-Z]\w*[\s/>]")
            .expect("jsx check")
            .is_match(content);
        if has_jsx {
            result.diagnostics.push(ParseDiagnostic {
                severity: DiagnosticSeverity::Info,
                message: "File contains JSX but no component declarations were detected"
                    .to_string(),
                file: file_path.to_string(),
                line: None,
                code: "I001".to_string(),
            });
        }
    }
}

/// Extract props type annotation from function parameters.
fn extract_props_type(params: &str) -> Option<String> {
    // Pattern: `props: TypeName` or `{ destructured }: TypeName`
    let re = Regex::new(r":\s*([A-Z]\w+(?:<[^>]+>)?)").expect("props type regex");
    re.captures(params).map(|c| c[1].to_string())
}

// ── Hook Extraction ──────────────────────────────────────────────────────

fn extract_hooks(content: &str, file_path: &str, result: &mut FileParse) {
    // Hooks: use* pattern - const [x, setX] = useState(...) or const x = useRef(...)
    let re_hook = Regex::new(
        r#"(?m)(?:const|let)\s+(?:(\[?[^=\]]+\]?)\s*=\s*)?(use[A-Z]\w*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)"#,
    )
    .expect("hook regex");

    // Standalone hook calls without binding.
    let re_standalone_hook =
        Regex::new(r#"(?m)^\s*(use[A-Z]\w*)\s*\(([^)]*)\)"#).expect("standalone hook regex");

    for (line_idx, line) in content.lines().enumerate() {
        let lineno = line_idx + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }

        if let Some(caps) = re_hook.captures(line) {
            let binding = caps.get(1).map(|m| m.as_str().trim().to_string());
            let hook_name = caps[2].to_string();
            let args = caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string();

            result.hooks.push(HookCall {
                name: hook_name,
                binding,
                args_snippet: truncate_snippet(&args, 100),
                line: lineno,
            });
        } else {
            // Also scan statement fragments on the same line so constructs like
            // `const App = () => { useEffect(...); ... }` are captured.
            for segment in line.split('{').flat_map(|s| s.split(';')) {
                if let Some(caps) = re_standalone_hook.captures(segment) {
                    let hook_name = caps[1].to_string();
                    let args = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string();

                    result.hooks.push(HookCall {
                        name: hook_name,
                        binding: None,
                        args_snippet: truncate_snippet(&args, 100),
                        line: lineno,
                    });
                }
            }
        }
    }

    // Expand effect-hook snippets across multiple lines so downstream semantic
    // analyzers can classify side effects and cleanups accurately.
    let lines: Vec<&str> = content.lines().collect();
    for hook in &mut result.hooks {
        if is_effect_hook(&hook.name)
            && let Some(expanded) =
                extract_multiline_hook_args(&lines, hook.line.saturating_sub(1), &hook.name)
        {
            hook.args_snippet = truncate_snippet(&expanded, 100);
        }
    }

    // Associate hooks with their nearest preceding component.
    // In a well-structured file, hooks appear inside component bodies.
    for hook in &result.hooks {
        if let Some(comp) = result
            .components
            .iter_mut()
            .filter(|c| c.line <= hook.line)
            .last()
        {
            comp.hooks.push(hook.clone());
        }
    }

    // Add hook symbols for custom hooks defined in this file.
    let re_custom_hook =
        Regex::new(r#"(?m)^[^\S\n]*(export\s+)?(?:function|const)\s+(use[A-Z]\w*)"#)
            .expect("custom hook def regex");

    for (line_idx, line) in content.lines().enumerate() {
        if let Some(caps) = re_custom_hook.captures(line) {
            let is_export = caps.get(1).is_some();
            let name = caps[2].to_string();
            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name,
                kind: SymbolKind::Hook,
                file: file_path.to_string(),
                line: line_idx + 1,
                is_exported: is_export,
                imported_from: None,
            });
        }
    }
}

fn is_effect_hook(name: &str) -> bool {
    matches!(name, "useEffect" | "useLayoutEffect" | "useInsertionEffect")
}

fn extract_multiline_hook_args(
    lines: &[&str],
    start_line_idx: usize,
    hook_name: &str,
) -> Option<String> {
    let mut window = String::new();
    for line in lines.iter().skip(start_line_idx).take(80) {
        window.push_str(line);
        window.push('\n');
        if let Some(args) = extract_hook_args_from_window(&window, hook_name) {
            return Some(args);
        }
    }
    None
}

fn extract_hook_args_from_window(window: &str, hook_name: &str) -> Option<String> {
    let hook_pos = find_hook_call_in_window(window, hook_name)?;
    let tail = &window[hook_pos + hook_name.len()..];

    // Locate the opening call parenthesis at generic depth 0.
    let mut open_paren_idx = None;
    let mut angle_depth = 0usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in tail.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => in_string = Some(ch),
            '<' => angle_depth += 1,
            '>' if angle_depth > 0 => angle_depth -= 1,
            '(' if angle_depth == 0 => {
                open_paren_idx = Some(idx);
                break;
            }
            _ => {}
        }
    }
    let open_idx = open_paren_idx?;

    // Parse balanced call args, honoring nested parens and quoted strings.
    let args_tail = &tail[open_idx + 1..];
    let mut depth = 1usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in args_tail.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => in_string = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(args_tail[..idx].trim().to_string());
                }
            }
            _ => {}
        }
    }

    None
}

fn find_hook_call_in_window(window: &str, hook_name: &str) -> Option<usize> {
    let mut search_start = 0usize;
    while search_start < window.len() {
        let relative = window[search_start..].find(hook_name)?;
        let pos = search_start + relative;

        let prev_ok = pos == 0
            || window[..pos]
                .chars()
                .next_back()
                .is_none_or(|ch| !is_identifier_char(ch));
        let after = &window[pos + hook_name.len()..];
        let next_ok = after
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace() || ch == '(' || ch == '<');

        if prev_ok && next_ok {
            return Some(pos);
        }
        search_start = pos + hook_name.len();
    }
    None
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

// ── JSX Element Extraction ───────────────────────────────────────────────

fn extract_jsx_elements(content: &str, _file_path: &str, result: &mut FileParse) {
    // Opening JSX tags: <TagName prop1="val" prop2={expr} />
    let re_jsx_open =
        Regex::new(r#"<([A-Za-z][A-Za-z0-9_.]*)\s*([^>]*?)(/?)>"#).expect("jsx open regex");

    // Fragment shorthand: <>...</>
    let re_fragment = Regex::new(r"<>|</>").expect("fragment regex");

    for (line_idx, line) in content.lines().enumerate() {
        let lineno = line_idx + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }

        // Check for fragments.
        if re_fragment.is_match(line) {
            result.jsx_elements.push(JsxElement {
                tag: "Fragment".to_string(),
                is_component: false,
                is_fragment: true,
                is_self_closing: false,
                props: Vec::new(),
                line: lineno,
            });
        }

        // Opening/self-closing tags.
        for caps in re_jsx_open.captures_iter(line) {
            let tag = caps[1].to_string();
            let props_str = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let is_self_closing = caps.get(3).map(|m| m.as_str()) == Some("/");

            // Skip closing tags that look like opening due to regex.
            if tag.starts_with('/') {
                continue;
            }

            let is_component = tag.chars().next().is_some_and(|c| c.is_uppercase());
            let is_fragment = tag == "React.Fragment" || tag == "Fragment";

            let props = extract_jsx_props(props_str);

            result.jsx_elements.push(JsxElement {
                tag,
                is_component,
                is_fragment,
                is_self_closing,
                props,
                line: lineno,
            });
        }
    }
}

/// Extract props from a JSX tag's attribute string.
fn extract_jsx_props(props_str: &str) -> Vec<JsxProp> {
    let mut props = Vec::new();

    // Spread props: {...expr}
    let re_spread = Regex::new(r"\{\.\.\.([^}]+)\}").expect("spread regex");
    for caps in re_spread.captures_iter(props_str) {
        let spread_expr = caps[1].trim().to_string();
        if spread_expr.is_empty() {
            continue;
        }
        props.push(JsxProp {
            name: spread_expr,
            is_spread: true,
            value_snippet: None,
        });
    }

    // Named props: name="value" or name={expr} or name (boolean)
    let re_prop =
        Regex::new(r#"([A-Za-z_][A-Za-z0-9_:\.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|\{([^}]*)\})"#)
            .expect("prop regex");
    for caps in re_prop.captures_iter(props_str) {
        let name = caps[1].to_string();
        let value = caps
            .get(2)
            .or(caps.get(3))
            .or(caps.get(4))
            .map(|m| m.as_str().to_string());
        props.push(JsxProp {
            name,
            is_spread: false,
            value_snippet: value,
        });
    }

    // Boolean props: attribute names without assignment (`disabled`, `required`).
    // We sanitize quoted/braced value payloads first to avoid false matches
    // from words inside string literals or expressions.
    let sanitized = sanitize_jsx_attribute_values(props_str);
    let re_bool = Regex::new(r#"(?:^|\s)([A-Za-z_][A-Za-z0-9_:\.-]*)"#).expect("bool prop regex");
    for caps in re_bool.captures_iter(&sanitized) {
        let Some(name_match) = caps.get(1) else {
            continue;
        };
        let after = &sanitized[name_match.end()..];
        let has_assignment = after.chars().find(|c| !c.is_whitespace()) == Some('=');
        if has_assignment {
            continue;
        }

        let name = caps[1].to_string();
        // Skip if already found as a named/spread prop.
        if !props.iter().any(|p| p.name == name) {
            props.push(JsxProp {
                name,
                is_spread: false,
                value_snippet: None,
            });
        }
    }

    props
}

fn sanitize_jsx_attribute_values(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' | '\'' => {
                let quote = ch;
                out.push(' ');
                let mut escaped = false;
                for c in chars.by_ref() {
                    out.push(' ');
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if c == '\\' {
                        escaped = true;
                        continue;
                    }
                    if c == quote {
                        break;
                    }
                }
            }
            '{' => {
                out.push(' ');
                let mut depth = 1usize;
                let mut in_string: Option<char> = None;
                let mut escaped = false;

                for c in chars.by_ref() {
                    out.push(' ');
                    if let Some(quote) = in_string {
                        if escaped {
                            escaped = false;
                            continue;
                        }
                        if c == '\\' {
                            escaped = true;
                            continue;
                        }
                        if c == quote {
                            in_string = None;
                        }
                        continue;
                    }

                    match c {
                        '"' | '\'' | '`' => {
                            in_string = Some(c);
                        }
                        '{' => {
                            depth += 1;
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

// ── Type Extraction ──────────────────────────────────────────────────────

fn extract_types(content: &str, file_path: &str, result: &mut FileParse) {
    // Interface declarations.
    let re_interface =
        Regex::new(r#"(?m)^[^\S\n]*(export\s+)?interface\s+(\w+)(?:\s+extends\s+[^{]+)?\s*\{"#)
            .expect("interface regex");

    // Type alias declarations.
    let re_type_alias = Regex::new(r#"(?m)^[^\S\n]*(export\s+)?type\s+(\w+)(?:<[^>]+>)?\s*="#)
        .expect("type alias regex");

    // Enum declarations.
    let re_enum = Regex::new(r#"(?m)^[^\S\n]*(export\s+)?(?:const\s+)?enum\s+(\w+)\s*\{"#)
        .expect("enum regex");

    let lines: Vec<&str> = content.lines().collect();

    for (line_idx, line) in lines.iter().enumerate() {
        let lineno = line_idx + 1;

        // Interface.
        if let Some(caps) = re_interface.captures(line) {
            let is_exported = caps.get(1).is_some();
            let name = caps[2].to_string();
            let fields = extract_interface_fields(&lines, line_idx);

            result.types.push(TypeDecl {
                name: name.clone(),
                kind: TypeDeclKind::Interface,
                fields,
                is_exported,
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name,
                kind: SymbolKind::Type,
                file: file_path.to_string(),
                line: lineno,
                is_exported,
                imported_from: None,
            });
            continue;
        }

        // Type alias.
        if let Some(caps) = re_type_alias.captures(line) {
            let is_exported = caps.get(1).is_some();
            let name = caps[2].to_string();

            result.types.push(TypeDecl {
                name: name.clone(),
                kind: TypeDeclKind::TypeAlias,
                fields: Vec::new(),
                is_exported,
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name,
                kind: SymbolKind::Type,
                file: file_path.to_string(),
                line: lineno,
                is_exported,
                imported_from: None,
            });
            continue;
        }

        // Enum.
        if let Some(caps) = re_enum.captures(line) {
            let is_exported = caps.get(1).is_some();
            let name = caps[2].to_string();

            result.types.push(TypeDecl {
                name: name.clone(),
                kind: TypeDeclKind::Enum,
                fields: Vec::new(),
                is_exported,
                line: lineno,
            });

            result.symbols.push(SymbolEntry {
                id: format!("{file_path}::{name}"),
                name,
                kind: SymbolKind::Enum,
                file: file_path.to_string(),
                line: lineno,
                is_exported,
                imported_from: None,
            });
        }
    }
}

/// Extract fields from an interface body (simple single-line field parsing).
fn extract_interface_fields(lines: &[&str], start_line: usize) -> Vec<TypeField> {
    let mut fields = Vec::new();
    let re_field =
        Regex::new(r#"^\s+(\w+)(\??):\s*(.+?)\s*[;,]?\s*$"#).expect("interface field regex");

    let mut brace_depth = 0i32;
    for line in lines.iter().skip(start_line) {
        for ch in line.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        if let Some(caps) = re_field.captures(line) {
            fields.push(TypeField {
                name: caps[1].to_string(),
                type_annotation: caps[3].to_string(),
                optional: &caps[2] == "?",
            });
        }

        if brace_depth < 0 {
            break;
        }
        if brace_depth == 0 && !lines[start_line..].is_empty() {
            // We've reached the closing brace.
            break;
        }
    }

    fields
}

// ── Symbol Extraction ────────────────────────────────────────────────────

fn extract_symbols(content: &str, file_path: &str, result: &mut FileParse) {
    // Import symbols.
    let re_named_import =
        Regex::new(r#"(?m)^[^\S\n]*import\s+\{([^}]+)\}\s+from\s+['"]([^'"]+)['"]"#)
            .expect("named import regex");

    let re_default_import = Regex::new(r#"(?m)^[^\S\n]*import\s+(\w+)\s+from\s+['"]([^'"]+)['"]"#)
        .expect("default import regex");

    let re_namespace_import =
        Regex::new(r#"(?m)^[^\S\n]*import\s+\*\s+as\s+(\w+)\s+from\s+['"]([^'"]+)['"]"#)
            .expect("namespace import regex");

    // Exported constants/variables.
    let re_exported_const = Regex::new(r#"(?m)^[^\S\n]*export\s+(?:const|let|var)\s+(\w+)"#)
        .expect("exported const regex");

    // Exported functions (non-component, lowercase).
    let re_exported_func =
        Regex::new(r#"(?m)^[^\S\n]*export\s+(?:default\s+)?function\s+([a-z]\w*)"#)
            .expect("exported func regex");

    for (line_idx, line) in content.lines().enumerate() {
        let lineno = line_idx + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }

        // Named imports.
        if let Some(caps) = re_named_import.captures(line) {
            let names_str = &caps[1];
            let specifier = caps[2].to_string();

            for name_part in names_str.split(',') {
                let name_part = name_part.trim();
                if name_part.is_empty() || name_part.starts_with("type ") {
                    continue;
                }
                let (original, local) = if let Some((orig, alias)) = name_part.split_once(" as ") {
                    (Some(orig.trim().to_string()), alias.trim().to_string())
                } else {
                    (None, name_part.to_string())
                };

                // Skip if already registered as a component/hook/type.
                let sym_id = format!("{file_path}::{local}");
                if result.symbols.iter().any(|s| s.id == sym_id) {
                    continue;
                }

                result.symbols.push(SymbolEntry {
                    id: sym_id,
                    name: local,
                    kind: SymbolKind::Variable,
                    file: file_path.to_string(),
                    line: lineno,
                    is_exported: false,
                    imported_from: Some(ImportSource {
                        specifier: specifier.clone(),
                        original_name: original,
                    }),
                });
            }
        }

        // Default imports.
        if let Some(caps) = re_default_import.captures(line) {
            let name = caps[1].to_string();
            let specifier = caps[2].to_string();

            let sym_id = format!("{file_path}::{name}");
            if !result.symbols.iter().any(|s| s.id == sym_id) {
                result.symbols.push(SymbolEntry {
                    id: sym_id,
                    name,
                    kind: SymbolKind::Variable,
                    file: file_path.to_string(),
                    line: lineno,
                    is_exported: false,
                    imported_from: Some(ImportSource {
                        specifier,
                        original_name: None,
                    }),
                });
            }
        }

        // Namespace imports.
        if let Some(caps) = re_namespace_import.captures(line) {
            let name = caps[1].to_string();
            let specifier = caps[2].to_string();

            let sym_id = format!("{file_path}::{name}");
            if !result.symbols.iter().any(|s| s.id == sym_id) {
                result.symbols.push(SymbolEntry {
                    id: sym_id,
                    name,
                    kind: SymbolKind::Namespace,
                    file: file_path.to_string(),
                    line: lineno,
                    is_exported: false,
                    imported_from: Some(ImportSource {
                        specifier,
                        original_name: None,
                    }),
                });
            }
        }

        // Exported constants (skip if already a component).
        if let Some(caps) = re_exported_const.captures(line) {
            let name = caps[1].to_string();
            let sym_id = format!("{file_path}::{name}");
            if !result.symbols.iter().any(|s| s.id == sym_id) {
                result.symbols.push(SymbolEntry {
                    id: sym_id,
                    name,
                    kind: SymbolKind::Constant,
                    file: file_path.to_string(),
                    line: lineno,
                    is_exported: true,
                    imported_from: None,
                });
            }
        }

        // Exported functions (lowercase = not a component).
        if let Some(caps) = re_exported_func.captures(line) {
            let name = caps[1].to_string();
            let sym_id = format!("{file_path}::{name}");
            if !result.symbols.iter().any(|s| s.id == sym_id) {
                result.symbols.push(SymbolEntry {
                    id: sym_id,
                    name,
                    kind: SymbolKind::Function,
                    file: file_path.to_string(),
                    line: lineno,
                    is_exported: true,
                    imported_from: None,
                });
            }
        }
    }

    // Event handlers from JSX (on* props).
    let re_event_handler =
        Regex::new(r#"(on[A-Z]\w*)\s*=\s*(?:\{([^}]+)\}|"([^"]*)")"#).expect("event handler regex");

    for (line_idx, line) in content.lines().enumerate() {
        for caps in re_event_handler.captures_iter(line) {
            let event_name = caps[1].to_string();
            let handler = caps
                .get(2)
                .or(caps.get(3))
                .map(|m| m.as_str().trim().to_string());
            let is_inline = handler
                .as_ref()
                .is_some_and(|h| h.contains("=>") || h.contains('('));

            let eh = EventHandler {
                event_name,
                handler_name: if is_inline { None } else { handler },
                is_inline,
                line: line_idx + 1,
            };

            // Associate with nearest preceding component.
            if let Some(comp) = result
                .components
                .iter_mut()
                .filter(|c| c.line <= line_idx + 1)
                .last()
            {
                comp.event_handlers.push(eh);
            }
        }
    }
}

// ── Cross-File Symbol Resolution ─────────────────────────────────────────

fn resolve_cross_file_symbols(project: &mut ProjectParse) {
    // Track external imports (packages not in the project).
    let file_keys: BTreeSet<String> = project.files.keys().cloned().collect();

    for file_parse in project.files.values() {
        for sym in &file_parse.symbols {
            if let Some(ref source) = sym.imported_from {
                let spec = &source.specifier;
                // If the specifier doesn't start with . or /, it's external.
                if !spec.starts_with('.') && !spec.starts_with('/') {
                    let pkg = if spec.starts_with('@') {
                        spec.splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
                    } else {
                        spec.split('/').next().unwrap_or(spec).to_string()
                    };
                    project.external_imports.insert(pkg);
                }
            }
        }
    }

    // Simple cross-file linking: match imported symbols to exported symbols
    // in the referenced files. We don't do full resolution here (that's for
    // the module graph) — just flag whether imported symbols have known sources.
    let _known_exports: BTreeMap<String, Vec<String>> = {
        let mut map = BTreeMap::new();
        for (file, parse) in &project.files {
            for sym in &parse.symbols {
                if sym.is_exported {
                    map.entry(file.clone())
                        .or_insert_with(Vec::new)
                        .push(sym.name.clone());
                }
            }
        }
        map
    };

    // Ensure file_keys is used for diagnostics.
    let _ = file_keys;
}

// ── Utilities ────────────────────────────────────────────────────────────

fn truncate_snippet(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let prefix = s.chars().take(max_len).collect::<String>();
        format!("{prefix}...")
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_function_component() {
        let src = r#"
export default function App(props: AppProps) {
    return <div>Hello</div>;
}
"#;
        let result = parse_file(src, "App.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].name, "App");
        assert_eq!(result.components[0].kind, ComponentKind::FunctionComponent);
        assert!(result.components[0].is_default_export);
        assert_eq!(result.components[0].props_type.as_deref(), Some("AppProps"));
    }

    #[test]
    fn parse_arrow_component() {
        let src = r#"
export const Button = () => {
    return <button>Click</button>;
};
"#;
        let result = parse_file(src, "Button.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].name, "Button");
        assert!(result.components[0].is_named_export);
    }

    #[test]
    fn parse_class_component() {
        let src = r#"
export class Dashboard extends React.Component {
    render() {
        return <div>Dashboard</div>;
    }
}
"#;
        let result = parse_file(src, "Dashboard.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].kind, ComponentKind::ClassComponent);
    }

    #[test]
    fn parse_forward_ref() {
        let src = r#"
export const Input = React.forwardRef((props, ref) => {
    return <input ref={ref} />;
});
"#;
        let result = parse_file(src, "Input.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].kind, ComponentKind::ForwardRef);
    }

    #[test]
    fn parse_memo_component() {
        let src = r#"
export const ExpensiveList = React.memo(({ items }) => {
    return <ul>{items.map(i => <li key={i}>{i}</li>)}</ul>;
});
"#;
        let result = parse_file(src, "ExpensiveList.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].kind, ComponentKind::Memo);
    }

    #[test]
    fn parse_forward_ref_and_memo_emit_component_symbols() {
        let src = r#"
export const Input = React.forwardRef((props, ref) => {
    return <input ref={ref} />;
});
export const ExpensiveList = React.memo(({ items }) => {
    return <ul>{items.map(i => <li key={i}>{i}</li>)}</ul>;
});
"#;
        let result = parse_file(src, "Components.tsx");
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "Input" && s.kind == SymbolKind::Component)
        );
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "ExpensiveList" && s.kind == SymbolKind::Component)
        );
    }

    #[test]
    fn parse_hooks() {
        let src = r#"
function Counter() {
    const [count, setCount] = useState(0);
    const ref = useRef(null);
    useEffect(() => {}, []);
    return <div>{count}</div>;
}
"#;
        let result = parse_file(src, "Counter.tsx");
        assert_eq!(result.hooks.len(), 3);
        assert_eq!(result.hooks[0].name, "useState");
        assert_eq!(
            result.hooks[0].binding.as_deref(),
            Some("[count, setCount]")
        );
        assert_eq!(result.hooks[1].name, "useRef");
        assert_eq!(result.hooks[2].name, "useEffect");
    }

    #[test]
    fn parse_multiline_effect_hook_arguments() {
        let src = r#"
function DataLoader() {
    useEffect(() => {
        fetch('/api/data').then(r => r.json());
        return () => console.log('cleanup');
    }, []);
    return <div />;
}
"#;
        let result = parse_file(src, "DataLoader.tsx");
        let effect = result
            .hooks
            .iter()
            .find(|h| h.name == "useEffect")
            .expect("useEffect hook not found");
        assert!(effect.args_snippet.contains("fetch('/api/data')"));
        assert!(effect.args_snippet.contains("return () =>"));
    }

    #[test]
    fn parse_multiline_effect_ignores_identifier_substring_matches() {
        let src = r#"
function DataLoader() {
    myuseEffect();
    useEffect(() => {
        fetch('/api/data').then(r => r.json());
        return () => console.log('cleanup');
    }, []);
    return <div />;
}
"#;
        let result = parse_file(src, "DataLoader.tsx");
        let effect = result
            .hooks
            .iter()
            .find(|h| h.name == "useEffect")
            .expect("useEffect hook not found");
        assert!(effect.args_snippet.contains("fetch('/api/data')"));
    }

    #[test]
    fn parse_unindented_standalone_hook_call() {
        let src = "function App() {\nuseEffect(() => {}, []);\nreturn <div />;\n}\n";
        let result = parse_file(src, "Counter.tsx");
        assert!(result.hooks.iter().any(|h| h.name == "useEffect"));
    }

    #[test]
    fn hooks_associated_with_component() {
        let src = r#"
function App() {
    const [x, setX] = useState(0);
    return <div>{x}</div>;
}
"#;
        let result = parse_file(src, "App.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].hooks.len(), 1);
        assert_eq!(result.components[0].hooks[0].name, "useState");
    }

    #[test]
    fn hooks_associated_with_same_line_component_declaration() {
        let src = r#"
const App = () => { useMemo(() => 42, []); return <div />; };
"#;
        let result = parse_file(src, "App.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.hooks.len(), 1);
        assert_eq!(result.components[0].hooks.len(), 1);
        assert_eq!(result.components[0].hooks[0].name, "useMemo");
    }

    #[test]
    fn parse_jsx_elements() {
        let src = r#"
function App() {
    return (
        <div className="container">
            <Header title="Hello" />
            <Button onClick={handleClick} disabled>Submit</Button>
            <>Fragment content</>
        </div>
    );
}
"#;
        let result = parse_file(src, "App.tsx");
        assert!(result.jsx_elements.len() >= 3);

        let header = result.jsx_elements.iter().find(|e| e.tag == "Header");
        assert!(header.is_some());
        assert!(header.unwrap().is_component);
        assert!(header.unwrap().is_self_closing);

        let fragment = result.jsx_elements.iter().find(|e| e.tag == "Fragment");
        assert!(fragment.is_some());
        assert!(fragment.unwrap().is_fragment);
    }

    #[test]
    fn parse_jsx_props() {
        let props = extract_jsx_props(r#"name="John" age={25} disabled {...rest}"#);
        assert!(props.iter().any(|p| p.name == "name" && !p.is_spread));
        assert!(props.iter().any(|p| p.name == "age" && !p.is_spread));
        assert!(props.iter().any(|p| p.name == "rest" && p.is_spread));
    }

    #[test]
    fn parse_jsx_props_ignores_words_inside_values_and_supports_hyphenated_names() {
        let props =
            extract_jsx_props(r#"title="hello world" data-test-id="abc" disabled aria-hidden"#);

        assert!(props.iter().any(|p| p.name == "title" && !p.is_spread));
        assert!(
            props
                .iter()
                .any(|p| p.name == "data-test-id" && !p.is_spread)
        );
        assert!(props.iter().any(|p| p.name == "disabled" && !p.is_spread));
        assert!(
            props
                .iter()
                .any(|p| p.name == "aria-hidden" && !p.is_spread)
        );
        assert!(!props.iter().any(|p| p.name == "hello"));
        assert!(!props.iter().any(|p| p.name == "world"));
    }

    #[test]
    fn parse_interface_type() {
        let src = r#"
export interface ButtonProps {
    label: string;
    onClick?: () => void;
    disabled: boolean;
}
"#;
        let result = parse_file(src, "types.ts");
        assert_eq!(result.types.len(), 1);
        assert_eq!(result.types[0].name, "ButtonProps");
        assert_eq!(result.types[0].kind, TypeDeclKind::Interface);
        assert!(result.types[0].is_exported);
        assert!(result.types[0].fields.len() >= 2);
    }

    #[test]
    fn parse_type_alias() {
        let src = r#"
export type Theme = 'light' | 'dark';
type Size = 'sm' | 'md' | 'lg';
"#;
        let result = parse_file(src, "types.ts");
        assert_eq!(result.types.len(), 2);
        assert_eq!(result.types[0].kind, TypeDeclKind::TypeAlias);
        assert!(result.types[0].is_exported);
        assert!(!result.types[1].is_exported);
    }

    #[test]
    fn parse_enum() {
        let src = r#"
export enum Status {
    Active,
    Inactive,
    Pending,
}
"#;
        let result = parse_file(src, "status.ts");
        assert_eq!(result.types.len(), 1);
        assert_eq!(result.types[0].kind, TypeDeclKind::Enum);
    }

    #[test]
    fn parse_named_imports_as_symbols() {
        let src = r#"
import { useState, useEffect } from 'react';
import { Button } from './Button';
import React from 'react';
"#;
        let result = parse_file(src, "App.tsx");
        assert!(result.symbols.iter().any(|s| s.name == "useState"));
        assert!(result.symbols.iter().any(|s| s.name == "useEffect"));
        assert!(result.symbols.iter().any(|s| s.name == "Button"));
        assert!(result.symbols.iter().any(|s| s.name == "React"));
    }

    #[test]
    fn parse_aliased_imports() {
        let src = r#"
import { Button as Btn } from './Button';
"#;
        let result = parse_file(src, "App.tsx");
        let sym = result.symbols.iter().find(|s| s.name == "Btn");
        assert!(sym.is_some());
        let sym = sym.unwrap();
        assert_eq!(
            sym.imported_from.as_ref().unwrap().original_name.as_deref(),
            Some("Button")
        );
    }

    #[test]
    fn parse_namespace_import() {
        let src = "import * as Icons from './icons';";
        let result = parse_file(src, "App.tsx");
        let sym = result.symbols.iter().find(|s| s.name == "Icons");
        assert!(sym.is_some());
        assert_eq!(sym.unwrap().kind, SymbolKind::Namespace);
    }

    #[test]
    fn parse_event_handlers() {
        let src = r#"
function Form() {
    return (
        <form onSubmit={handleSubmit}>
            <input onChange={(e) => setName(e.target.value)} />
            <button onClick={handleClick}>Submit</button>
        </form>
    );
}
"#;
        let result = parse_file(src, "Form.tsx");
        assert!(!result.components.is_empty());
        let handlers = &result.components[0].event_handlers;
        assert!(
            handlers
                .iter()
                .any(|h| h.event_name == "onSubmit" && !h.is_inline)
        );
        assert!(
            handlers
                .iter()
                .any(|h| h.event_name == "onChange" && h.is_inline)
        );
    }

    #[test]
    fn parse_exported_constants() {
        let src = r#"
export const MAX_RETRIES = 3;
export const API_URL = 'https://api.example.com';
"#;
        let result = parse_file(src, "config.ts");
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "MAX_RETRIES" && s.kind == SymbolKind::Constant)
        );
    }

    #[test]
    fn parse_exported_function() {
        let src = r#"
export function formatDate(d: Date): string {
    return d.toISOString();
}
"#;
        let result = parse_file(src, "utils.ts");
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "formatDate" && s.kind == SymbolKind::Function)
        );
    }

    #[test]
    fn custom_hook_definition() {
        let src = r#"
export function useTheme() {
    const [theme, setTheme] = useState('light');
    return { theme, setTheme };
}
"#;
        let result = parse_file(src, "hooks.ts");
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "useTheme" && s.kind == SymbolKind::Hook)
        );
    }

    #[test]
    fn file_parse_serializes_to_json() {
        let src = r#"
export function App() {
    const [x, setX] = useState(0);
    return <div>{x}</div>;
}
"#;
        let result = parse_file(src, "App.tsx");
        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("App"));
        assert!(json.contains("useState"));

        // Round-trip.
        let parsed: FileParse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.components.len(), result.components.len());
    }

    #[test]
    fn project_parse_with_tempdir() {
        use std::fs;
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("App.tsx"),
            r#"
import { Button } from './Button';
export function App() {
    return <Button label="Hi" />;
}
"#,
        )
        .unwrap();
        fs::write(
            src_dir.join("Button.tsx"),
            r#"
export interface ButtonProps { label: string; }
export function Button({ label }: ButtonProps) {
    return <button>{label}</button>;
}
"#,
        )
        .unwrap();

        let project = parse_project(
            dir.path(),
            &["src/App.tsx".to_string(), "src/Button.tsx".to_string()],
        );

        assert_eq!(project.files.len(), 2);
        assert_eq!(
            project.file_contents.get("src/App.tsx").map(String::as_str),
            Some(
                "\nimport { Button } from './Button';\nexport function App() {\n    return <Button label=\"Hi\" />;\n}\n"
            )
        );
        assert_eq!(project.component_count, 2);
        assert!(project.type_count >= 1);
        assert!(!project.symbol_table.is_empty());
    }

    #[test]
    fn recoverable_error_for_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = parse_project(dir.path(), &["nonexistent.tsx".to_string()]);

        assert_eq!(project.files.len(), 0);
        assert!(project.file_contents.is_empty());
        assert_eq!(project.diagnostics.len(), 1);
        assert_eq!(project.diagnostics[0].severity, DiagnosticSeverity::Error);
        assert_eq!(project.diagnostics[0].code, "E001");
    }

    #[test]
    fn empty_file_produces_empty_parse() {
        let result = parse_file("", "empty.ts");
        assert!(result.components.is_empty());
        assert!(result.hooks.is_empty());
        assert!(result.jsx_elements.is_empty());
        assert!(result.types.is_empty());
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn comments_skipped_in_components() {
        let src = r#"
// function Fake() {}
/* function AlsoFake() {} */
function Real() {
    return <div />;
}
"#;
        let result = parse_file(src, "test.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].name, "Real");
    }

    #[test]
    fn truncate_snippet_works() {
        assert_eq!(truncate_snippet("short", 10), "short");
        assert_eq!(truncate_snippet("a long string here", 5), "a lon...");
    }

    #[test]
    fn truncate_snippet_handles_unicode_boundaries() {
        assert_eq!(truncate_snippet("éclair", 1), "é...");
        assert_eq!(truncate_snippet("αβγδε", 3), "αβγ...");
    }

    #[test]
    fn extract_props_type_from_params() {
        assert_eq!(
            extract_props_type("props: ButtonProps"),
            Some("ButtonProps".to_string())
        );
        assert_eq!(
            extract_props_type("{ label }: Props"),
            Some("Props".to_string())
        );
        assert_eq!(extract_props_type(""), None);
    }

    #[test]
    fn multiple_components_per_file() {
        let src = r#"
export function Header() {
    return <header />;
}

export function Footer() {
    return <footer />;
}

export function Sidebar() {
    return <aside />;
}
"#;
        let result = parse_file(src, "layout.tsx");
        assert_eq!(result.components.len(), 3);
        assert_eq!(result.components[0].name, "Header");
        assert_eq!(result.components[1].name, "Footer");
        assert_eq!(result.components[2].name, "Sidebar");
    }

    #[test]
    fn generic_component_detected() {
        let src = r#"
export function List<T extends { id: string }>(props: ListProps<T>) {
    return <ul />;
}
"#;
        let result = parse_file(src, "List.tsx");
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].name, "List");
    }

    #[test]
    fn diagnostic_severity_variants() {
        let w = DiagnosticSeverity::Warning;
        let e = DiagnosticSeverity::Error;
        let i = DiagnosticSeverity::Info;
        // Ensure they serialize distinctly.
        assert_ne!(
            serde_json::to_string(&w).unwrap(),
            serde_json::to_string(&e).unwrap()
        );
        assert_ne!(
            serde_json::to_string(&e).unwrap(),
            serde_json::to_string(&i).unwrap()
        );
    }

    #[test]
    fn external_imports_tracked_in_project() {
        use std::fs;
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("app.tsx"),
            r#"
import React from 'react';
import { motion } from 'framer-motion';
import { Button } from '@chakra-ui/react';
"#,
        )
        .unwrap();

        let project = parse_project(dir.path(), &["app.tsx".to_string()]);
        assert!(project.external_imports.contains("react"));
        assert!(project.external_imports.contains("framer-motion"));
        assert!(project.external_imports.contains("@chakra-ui/react"));
    }
}
