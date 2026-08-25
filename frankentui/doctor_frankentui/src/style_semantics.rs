//! Style and theming semantics extraction.
//!
//! Bridges the raw JSX prop data from `tsx_parser` to the canonical
//! `migration_ir::StyleIntent` representation. The extractor detects:
//!
//! - **Inline style objects** (`style={{ color: 'red' }}`)
//! - **ClassName bindings** (static strings, template literals, `clsx`/`cn` calls)
//! - **CSS-module / styled-component patterns** (`styles.foo`, `styled.div`)
//! - **Theme provider usage** (`ThemeProvider`, `useTheme`, `createTheme`)
//! - **Design token declarations** (token objects, CSS custom properties)
//! - **Style precedence and conflict detection**
//! - **Accessibility-relevant color metadata** (contrast hints)

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::migration_ir::{
    self, IrNodeId, LayoutIntent, LayoutKind, Provenance, StyleIntent, StyleToken, ThemeDecl,
    TokenCategory,
};
use crate::tsx_parser::{ComponentDecl, FileParse, HookCall, JsxElement, JsxProp, ProjectParse};

// ── Intermediate semantic types ─────────────────────────────────────────

/// Complete result of style semantics extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleSemanticsResult {
    /// Discovered design tokens keyed by canonical name.
    pub tokens: BTreeMap<String, DesignTokenInfo>,
    /// Theme declarations found in the source.
    pub themes: Vec<ThemeDeclarationInfo>,
    /// Per-component style bindings keyed by a stable node ID.
    pub style_bindings: BTreeMap<IrNodeId, Vec<StyleBindingInfo>>,
    /// Per-component layout intents.
    pub layout_intents: BTreeMap<IrNodeId, LayoutIntentInfo>,
    /// Style source type index (which techniques are used across the project).
    pub style_sources_used: BTreeSet<StyleSourceKind>,
    /// Warnings emitted during extraction.
    pub warnings: Vec<StyleWarning>,
}

/// A single style binding attached to a JSX element or component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleBindingInfo {
    /// The prop name that carries the style (e.g. `style`, `className`, `sx`).
    pub prop_name: String,
    /// How the style value is sourced.
    pub source: StyleSourceKind,
    /// Raw value snippet from the JSX prop.
    pub raw_value: Option<String>,
    /// Parsed inline style properties (key → value).
    pub inline_properties: BTreeMap<String, String>,
    /// CSS class references.
    pub class_refs: Vec<String>,
    /// Whether this binding is conditional (ternary, logical, clsx).
    pub is_conditional: bool,
    /// Is this a spread that may carry style props.
    pub is_spread: bool,
    /// Source location.
    pub provenance: Provenance,
}

/// A discovered design token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignTokenInfo {
    pub name: String,
    pub category: TokenCategory,
    pub value: String,
    pub references: Vec<String>,
    pub provenance: Option<Provenance>,
}

/// A theme declaration extracted from source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDeclarationInfo {
    pub name: String,
    pub tokens: BTreeMap<String, String>,
    pub is_default: bool,
    pub provider_component: Option<String>,
    pub provenance: Provenance,
}

/// Layout intent extracted from style properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutIntentInfo {
    pub kind: LayoutKind,
    pub direction: Option<String>,
    pub alignment: Option<String>,
    pub sizing: Option<String>,
    pub provenance: Provenance,
}

/// Classification of how a style is sourced.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StyleSourceKind {
    /// `style={{ ... }}` — inline style object.
    InlineObject,
    /// `className="foo"` — static class string.
    StaticClassName,
    /// `className={clsx(...)}` or template literal — dynamic class.
    DynamicClassName,
    /// `className={styles.foo}` — CSS module reference.
    CssModule,
    /// `styled.div\`...\`` or `styled(Component)` — styled component.
    StyledComponent,
    /// `sx={{ ... }}` — MUI/theme-aware style prop.
    ThemeAwareProp,
    /// Spread prop that may carry style attributes.
    Spread,
    /// Source could not be determined.
    Unknown,
}

/// Accessibility-relevant style metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityStyleMeta {
    /// Components with explicit color declarations (potential contrast issues).
    pub components_with_colors: Vec<IrNodeId>,
    /// Components with font-size declarations (readability).
    pub components_with_font_sizes: Vec<IrNodeId>,
    /// Components using opacity (visibility concerns).
    pub components_with_opacity: Vec<IrNodeId>,
}

/// A warning from the style extraction process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleWarning {
    pub kind: StyleWarningKind,
    pub message: String,
    pub provenance: Option<Provenance>,
}

/// Warning classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StyleWarningKind {
    /// Multiple style sources compete for the same element.
    PrecedenceConflict,
    /// A class reference could not be resolved.
    UnresolvedClassRef,
    /// A theme token is referenced but not declared.
    UnresolvedToken,
    /// Inline style overrides a class-based style.
    InlineOverride,
    /// Hard-coded color value (not a token).
    HardcodedColor,
    /// Potential accessibility concern.
    AccessibilityConcern,
}

// ── Extraction ──────────────────────────────────────────────────────────

/// Extract style and theming semantics from a parsed project.
pub fn extract_style_semantics(project: &ProjectParse) -> StyleSemanticsResult {
    let mut result = StyleSemanticsResult {
        tokens: BTreeMap::new(),
        themes: Vec::new(),
        style_bindings: BTreeMap::new(),
        layout_intents: BTreeMap::new(),
        style_sources_used: BTreeSet::new(),
        warnings: Vec::new(),
    };

    for (file_path, file_parse) in &project.files {
        process_file(file_path, file_parse, &mut result);
    }

    // Detect hard-coded colors across all bindings.
    detect_hardcoded_colors(&mut result);

    result
}

/// Convert extraction result to the canonical IR `StyleIntent`.
pub fn to_style_intent(result: &StyleSemanticsResult) -> StyleIntent {
    let tokens = result
        .tokens
        .iter()
        .map(|(name, info)| {
            (
                name.clone(),
                StyleToken {
                    name: info.name.clone(),
                    category: info.category.clone(),
                    value: info.value.clone(),
                    provenance: info.provenance.clone(),
                },
            )
        })
        .collect();

    let layouts = result
        .layout_intents
        .iter()
        .map(|(id, info)| {
            (
                id.clone(),
                LayoutIntent {
                    kind: info.kind.clone(),
                    direction: info.direction.clone(),
                    alignment: info.alignment.clone(),
                    sizing: info.sizing.clone(),
                },
            )
        })
        .collect();

    let themes = result
        .themes
        .iter()
        .map(|t| ThemeDecl {
            name: t.name.clone(),
            tokens: t.tokens.clone(),
            is_default: t.is_default,
        })
        .collect();

    StyleIntent {
        tokens,
        layouts,
        themes,
    }
}

/// Compute accessibility-relevant style metadata.
pub fn accessibility_meta(result: &StyleSemanticsResult) -> AccessibilityStyleMeta {
    let mut meta = AccessibilityStyleMeta {
        components_with_colors: Vec::new(),
        components_with_font_sizes: Vec::new(),
        components_with_opacity: Vec::new(),
    };

    for (id, bindings) in &result.style_bindings {
        for binding in bindings {
            for prop in binding.inline_properties.keys() {
                let prop_lower = prop.to_lowercase();
                if (prop_lower.contains("color") || prop_lower.contains("background"))
                    && !meta.components_with_colors.contains(id)
                {
                    meta.components_with_colors.push(id.clone());
                }
                if (prop_lower.contains("fontsize") || prop_lower.contains("font-size"))
                    && !meta.components_with_font_sizes.contains(id)
                {
                    meta.components_with_font_sizes.push(id.clone());
                }
                if prop_lower == "opacity" && !meta.components_with_opacity.contains(id) {
                    meta.components_with_opacity.push(id.clone());
                }
            }
        }
    }

    meta
}

// ── File processing ─────────────────────────────────────────────────────

fn process_file(file_path: &str, file_parse: &FileParse, result: &mut StyleSemanticsResult) {
    // Extract theme declarations from hooks.
    for component in &file_parse.components {
        extract_theme_hooks(file_path, component, result);
    }

    // Extract style bindings from JSX elements.
    for element in &file_parse.jsx_elements {
        process_jsx_element(file_path, element, result);
    }

    // Look for token declarations in component bodies.
    for component in &file_parse.components {
        extract_tokens_from_component(file_path, component, result);
    }
}

fn extract_theme_hooks(
    file_path: &str,
    component: &ComponentDecl,
    result: &mut StyleSemanticsResult,
) {
    for hook in &component.hooks {
        // Detect `useTheme()`, `useStyles()`, `makeStyles()`, `createTheme()`
        if is_theme_hook(&hook.name) {
            let theme_info = extract_theme_from_hook(file_path, hook);
            if let Some(info) = theme_info {
                result.themes.push(info);
            }
        }
    }
}

fn is_theme_hook(name: &str) -> bool {
    matches!(
        name,
        "useTheme"
            | "useStyles"
            | "makeStyles"
            | "createTheme"
            | "createMuiTheme"
            | "useColorScheme"
            | "useMediaQuery"
    )
}

fn extract_theme_from_hook(file_path: &str, hook: &HookCall) -> Option<ThemeDeclarationInfo> {
    let name = hook.name.as_str();
    // Only `createTheme` and `makeStyles` produce theme declarations.
    if name != "createTheme" && name != "createMuiTheme" && name != "makeStyles" {
        return None;
    }

    let tokens = extract_token_map_from_snippet(&hook.args_snippet);
    let theme_name = hook.binding.as_deref().unwrap_or("default").to_string();

    Some(ThemeDeclarationInfo {
        name: theme_name,
        tokens,
        is_default: name == "createTheme" || name == "createMuiTheme",
        provider_component: None,
        provenance: Provenance {
            file: file_path.to_string(),
            line: hook.line,
            column: None,
            source_name: Some(hook.name.clone()),
            policy_category: Some("theme".to_string()),
        },
    })
}

fn process_jsx_element(file_path: &str, element: &JsxElement, result: &mut StyleSemanticsResult) {
    // Check for ThemeProvider usage.
    if is_theme_provider(&element.tag) {
        extract_theme_from_provider(file_path, element, result);
    }

    // Extract style bindings from props.
    let node_id = make_element_id(file_path, element);
    let mut bindings = Vec::new();

    for prop in &element.props {
        if let Some(binding) = classify_style_prop(file_path, element, prop) {
            result.style_sources_used.insert(binding.source.clone());
            bindings.push(binding);
        }
    }

    // Detect precedence conflicts.
    detect_precedence_conflicts(&node_id, &bindings, result);

    // Extract layout intent from inline styles.
    if let Some(layout) = extract_layout_intent(file_path, element, &bindings) {
        result.layout_intents.insert(node_id.clone(), layout);
    }

    if !bindings.is_empty() {
        result.style_bindings.insert(node_id, bindings);
    }
}

fn is_theme_provider(tag: &str) -> bool {
    matches!(
        tag,
        "ThemeProvider" | "MuiThemeProvider" | "StyledEngineProvider" | "ThemeContext.Provider"
    )
}

fn extract_theme_from_provider(
    file_path: &str,
    element: &JsxElement,
    result: &mut StyleSemanticsResult,
) {
    let theme_prop = element.props.iter().find(|p| p.name == "theme");

    if let Some(prop) = theme_prop {
        let tokens = prop
            .value_snippet
            .as_deref()
            .map(extract_token_map_from_snippet)
            .unwrap_or_default();

        let name = prop
            .value_snippet
            .as_deref()
            .and_then(extract_identifier)
            .unwrap_or_else(|| "provider-theme".to_string());

        result.themes.push(ThemeDeclarationInfo {
            name,
            tokens,
            is_default: true,
            provider_component: Some(element.tag.clone()),
            provenance: Provenance {
                file: file_path.to_string(),
                line: element.line,
                column: None,
                source_name: Some(element.tag.clone()),
                policy_category: Some("theme-provider".to_string()),
            },
        });
    }
}

fn classify_style_prop(
    file_path: &str,
    element: &JsxElement,
    prop: &JsxProp,
) -> Option<StyleBindingInfo> {
    let provenance = Provenance {
        file: file_path.to_string(),
        line: element.line,
        column: None,
        source_name: Some(format!("{}:{}", element.tag, prop.name)),
        policy_category: Some("style".to_string()),
    };

    if prop.is_spread {
        return Some(StyleBindingInfo {
            prop_name: prop.name.clone(),
            source: StyleSourceKind::Spread,
            raw_value: prop.value_snippet.clone(),
            inline_properties: BTreeMap::new(),
            class_refs: Vec::new(),
            is_conditional: false,
            is_spread: true,
            provenance,
        });
    }

    match prop.name.as_str() {
        "style" => {
            let raw = prop.value_snippet.as_deref().unwrap_or("");
            let properties = parse_inline_style_object(raw);
            Some(StyleBindingInfo {
                prop_name: "style".to_string(),
                source: StyleSourceKind::InlineObject,
                raw_value: prop.value_snippet.clone(),
                inline_properties: properties,
                class_refs: Vec::new(),
                is_conditional: raw.contains('?') || raw.contains("&&"),
                is_spread: false,
                provenance,
            })
        }
        "className" => {
            let raw = prop.value_snippet.as_deref().unwrap_or("");
            let (source, classes) = classify_classname(raw);
            Some(StyleBindingInfo {
                prop_name: "className".to_string(),
                source,
                raw_value: prop.value_snippet.clone(),
                inline_properties: BTreeMap::new(),
                class_refs: classes,
                is_conditional: raw.contains('?')
                    || raw.contains("&&")
                    || raw.contains("clsx")
                    || raw.contains("cx(")
                    || raw.contains("cn("),
                is_spread: false,
                provenance,
            })
        }
        "sx" => {
            let raw = prop.value_snippet.as_deref().unwrap_or("");
            let properties = parse_inline_style_object(raw);
            Some(StyleBindingInfo {
                prop_name: "sx".to_string(),
                source: StyleSourceKind::ThemeAwareProp,
                raw_value: prop.value_snippet.clone(),
                inline_properties: properties,
                class_refs: Vec::new(),
                is_conditional: raw.contains('?') || raw.contains("&&"),
                is_spread: false,
                provenance,
            })
        }
        "css" => {
            let raw = prop.value_snippet.as_deref().unwrap_or("");
            let properties = parse_inline_style_object(raw);
            Some(StyleBindingInfo {
                prop_name: "css".to_string(),
                source: StyleSourceKind::ThemeAwareProp,
                raw_value: prop.value_snippet.clone(),
                inline_properties: properties,
                class_refs: Vec::new(),
                is_conditional: false,
                is_spread: false,
                provenance,
            })
        }
        _ => None,
    }
}

fn classify_classname(raw: &str) -> (StyleSourceKind, Vec<String>) {
    let trimmed = raw.trim();

    // CSS module: `styles.foo` or `styles['foo']`
    if trimmed.contains("styles.") || trimmed.contains("styles[") {
        let refs = extract_css_module_refs(trimmed);
        return (StyleSourceKind::CssModule, refs);
    }

    // Dynamic class: template literal, clsx(), cn(), cx()
    if trimmed.starts_with('`')
        || trimmed.contains("clsx")
        || trimmed.contains("cx(")
        || trimmed.contains("cn(")
        || trimmed.contains("classNames(")
    {
        let refs = extract_class_names_from_dynamic(trimmed);
        return (StyleSourceKind::DynamicClassName, refs);
    }

    // Static class: plain string
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        let inner = trimmed.trim_matches(|c| c == '"' || c == '\'').to_string();
        let refs: Vec<String> = inner.split_whitespace().map(|s| s.to_string()).collect();
        return (StyleSourceKind::StaticClassName, refs);
    }

    // Styled component pattern
    if trimmed.starts_with("styled.") || trimmed.starts_with("styled(") {
        return (StyleSourceKind::StyledComponent, vec![trimmed.to_string()]);
    }

    (StyleSourceKind::Unknown, vec![trimmed.to_string()])
}

fn extract_css_module_refs(snippet: &str) -> Vec<String> {
    let mut refs = Vec::new();
    // Match `styles.xxx` patterns.
    let mut remaining = snippet;
    while let Some(idx) = remaining.find("styles.") {
        let after = &remaining[idx + 7..];
        let end = after
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(after.len());
        if end > 0 {
            refs.push(after[..end].to_string());
        }
        remaining = &after[end..];
    }
    // Match `styles['xxx']` patterns.
    let mut remaining = snippet;
    while let Some(idx) = remaining.find("styles[") {
        let after = &remaining[idx + 7..];
        if let Some(end) = after.find(']') {
            let key = after[..end]
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            if !key.is_empty() {
                refs.push(key);
            }
        }
        remaining = &remaining[idx + 7..];
    }
    refs
}

fn extract_class_names_from_dynamic(snippet: &str) -> Vec<String> {
    let mut refs = Vec::new();
    // Extract quoted strings from the snippet.
    let mut chars = snippet.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == '\'' || c == '"' {
            let quote = c;
            chars.next();
            let mut name = String::new();
            for nc in chars.by_ref() {
                if nc == quote {
                    break;
                }
                name.push(nc);
            }
            if !name.is_empty() {
                for part in name.split_whitespace() {
                    refs.push(part.to_string());
                }
            }
        } else {
            chars.next();
        }
    }
    refs
}

/// Parse a simplified inline style object.
/// Handles patterns like `{{ color: 'red', fontSize: 16 }}`.
fn parse_inline_style_object(snippet: &str) -> BTreeMap<String, String> {
    let mut props = BTreeMap::new();
    let trimmed = snippet.trim();

    // Strip outer {{ }} or { }.
    let inner = trimmed.trim_start_matches('{').trim_end_matches('}').trim();

    if inner.is_empty() {
        return props;
    }

    // Split on commas (simplified — doesn't handle nested objects perfectly).
    for part in split_style_entries(inner) {
        let part = part.trim();
        if let Some(colon_pos) = find_top_level_colon(part) {
            let key = part[..colon_pos].trim().to_string();
            let value = part[colon_pos + 1..].trim().to_string();
            if !key.is_empty() && !value.is_empty() {
                let clean_key = key.trim_matches(|c| c == '\'' || c == '"');
                let clean_value = value.trim_matches(|c| c == '\'' || c == '"' || c == '`');
                props.insert(clean_key.to_string(), clean_value.to_string());
            }
        }
    }

    props
}

/// Split style entries on top-level commas (respecting nesting).
fn split_style_entries(s: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_char = ' ';

    for c in s.chars() {
        if in_string {
            current.push(c);
            if c == string_char {
                in_string = false;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                in_string = true;
                string_char = c;
                current.push(c);
            }
            '{' | '(' | '[' => {
                depth += 1;
                current.push(c);
            }
            '}' | ')' | ']' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    entries.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        entries.push(trimmed);
    }
    entries
}

/// Find the first colon at nesting depth 0.
fn find_top_level_colon(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_char = ' ';

    for (i, c) in s.char_indices() {
        if in_string {
            if c == string_char {
                in_string = false;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                in_string = true;
                string_char = c;
            }
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            ':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

// ── Token extraction ────────────────────────────────────────────────────

fn extract_tokens_from_component(
    file_path: &str,
    component: &ComponentDecl,
    result: &mut StyleSemanticsResult,
) {
    for hook in &component.hooks {
        // Detect `createTheme({ palette: { primary: '#1976d2' } })`
        if hook.name == "createTheme" || hook.name == "createMuiTheme" {
            let tokens = extract_token_map_from_snippet(&hook.args_snippet);
            for (name, value) in &tokens {
                let category = categorize_token_value(name, value);
                let token_name = format!("{}:{}", component.name, name);
                result.tokens.insert(
                    token_name.clone(),
                    DesignTokenInfo {
                        name: token_name,
                        category,
                        value: value.clone(),
                        references: vec![component.name.clone()],
                        provenance: Some(Provenance {
                            file: file_path.to_string(),
                            line: hook.line,
                            column: None,
                            source_name: Some(component.name.clone()),
                            policy_category: Some("design-token".to_string()),
                        }),
                    },
                );
            }
        }
    }
}

/// Extract a flat key→value map from a snippet that looks like an object literal.
fn extract_token_map_from_snippet(snippet: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let trimmed = snippet.trim().trim_start_matches('(').trim_end_matches(')');
    let inner = trimmed.trim_start_matches('{').trim_end_matches('}').trim();

    for part in split_style_entries(inner) {
        let part = part.trim();
        if let Some(colon_pos) = find_top_level_colon(part) {
            let key = part[..colon_pos]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            let value = part[colon_pos + 1..]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            if !key.is_empty() && !value.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

fn extract_identifier(snippet: &str) -> Option<String> {
    let trimmed = snippet
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    let end = trimmed
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(trimmed.len());
    if end > 0 {
        Some(trimmed[..end].to_string())
    } else {
        None
    }
}

fn categorize_token_value(name: &str, value: &str) -> TokenCategory {
    let name_lower = name.to_lowercase();
    let value_lower = value.to_lowercase();

    // Color patterns.
    if name_lower.contains("color")
        || name_lower.contains("palette")
        || name_lower.contains("background")
        || value_lower.starts_with('#')
        || value_lower.starts_with("rgb")
        || value_lower.starts_with("hsl")
    {
        return TokenCategory::Color;
    }

    // Spacing patterns.
    if name_lower.contains("spacing")
        || name_lower.contains("margin")
        || name_lower.contains("padding")
        || name_lower.contains("gap")
    {
        return TokenCategory::Spacing;
    }

    // Typography patterns.
    if name_lower.contains("font")
        || name_lower.contains("text")
        || name_lower.contains("typography")
        || name_lower.contains("lineheight")
        || name_lower.contains("letterspacing")
    {
        return TokenCategory::Typography;
    }

    // Border patterns.
    if name_lower.contains("border") || name_lower.contains("radius") {
        return TokenCategory::Border;
    }

    // Shadow patterns.
    if name_lower.contains("shadow") || name_lower.contains("elevation") {
        return TokenCategory::Shadow;
    }

    // Animation patterns.
    if name_lower.contains("animation")
        || name_lower.contains("transition")
        || name_lower.contains("duration")
    {
        return TokenCategory::Animation;
    }

    // Breakpoint patterns.
    if name_lower.contains("breakpoint") || name_lower.contains("screen") {
        return TokenCategory::Breakpoint;
    }

    // Z-index patterns.
    if name_lower.contains("zindex") || name_lower.contains("z-index") {
        return TokenCategory::ZIndex;
    }

    // Default based on value shape.
    if value.ends_with("px") || value.ends_with("rem") || value.ends_with("em") {
        return TokenCategory::Spacing;
    }

    TokenCategory::Color // Fallback.
}

// ── Layout intent detection ─────────────────────────────────────────────

fn extract_layout_intent(
    file_path: &str,
    element: &JsxElement,
    bindings: &[StyleBindingInfo],
) -> Option<LayoutIntentInfo> {
    for binding in bindings {
        if let Some(display) = binding.inline_properties.get("display") {
            let kind = match display.as_str() {
                "flex" => LayoutKind::Flex,
                "grid" => LayoutKind::Grid,
                "inline-flex" => LayoutKind::Flex,
                "inline-grid" => LayoutKind::Grid,
                _ => continue,
            };

            let direction = binding
                .inline_properties
                .get("flexDirection")
                .or_else(|| binding.inline_properties.get("flex-direction"))
                .cloned();

            let alignment = binding
                .inline_properties
                .get("alignItems")
                .or_else(|| binding.inline_properties.get("align-items"))
                .or_else(|| binding.inline_properties.get("justifyContent"))
                .or_else(|| binding.inline_properties.get("justify-content"))
                .cloned();

            let sizing = binding
                .inline_properties
                .get("width")
                .or_else(|| binding.inline_properties.get("height"))
                .cloned();

            return Some(LayoutIntentInfo {
                kind,
                direction,
                alignment,
                sizing,
                provenance: Provenance {
                    file: file_path.to_string(),
                    line: element.line,
                    column: None,
                    source_name: Some(element.tag.clone()),
                    policy_category: Some("layout".to_string()),
                },
            });
        }

        // Detect layout from class names (common utility class patterns).
        for class_ref in &binding.class_refs {
            if let Some(kind) = layout_kind_from_class(class_ref) {
                return Some(LayoutIntentInfo {
                    kind,
                    direction: None,
                    alignment: None,
                    sizing: None,
                    provenance: Provenance {
                        file: file_path.to_string(),
                        line: element.line,
                        column: None,
                        source_name: Some(element.tag.clone()),
                        policy_category: Some("layout".to_string()),
                    },
                });
            }
        }
    }
    None
}

fn layout_kind_from_class(class: &str) -> Option<LayoutKind> {
    if class == "flex" || class.starts_with("flex-") || class == "inline-flex" {
        Some(LayoutKind::Flex)
    } else if class == "grid" || class.starts_with("grid-") || class == "inline-grid" {
        Some(LayoutKind::Grid)
    } else if class == "absolute" || class == "fixed" || class == "relative" {
        Some(LayoutKind::Absolute)
    } else if class.starts_with("stack") {
        Some(LayoutKind::Stack)
    } else {
        None
    }
}

// ── Precedence & conflict detection ─────────────────────────────────────

fn detect_precedence_conflicts(
    node_id: &IrNodeId,
    bindings: &[StyleBindingInfo],
    result: &mut StyleSemanticsResult,
) {
    let has_inline = bindings
        .iter()
        .any(|b| b.source == StyleSourceKind::InlineObject);
    let has_class = bindings.iter().any(|b| {
        matches!(
            b.source,
            StyleSourceKind::StaticClassName
                | StyleSourceKind::DynamicClassName
                | StyleSourceKind::CssModule
        )
    });

    if has_inline && has_class {
        result.warnings.push(StyleWarning {
            kind: StyleWarningKind::InlineOverride,
            message: format!(
                "Element {node_id} has both inline style and className — inline takes precedence"
            ),
            provenance: bindings.first().map(|b| b.provenance.clone()),
        });
    }

    // Check for multiple class-based sources.
    let class_count = bindings
        .iter()
        .filter(|b| {
            matches!(
                b.source,
                StyleSourceKind::StaticClassName
                    | StyleSourceKind::DynamicClassName
                    | StyleSourceKind::CssModule
            )
        })
        .count();

    if class_count > 1 {
        result.warnings.push(StyleWarning {
            kind: StyleWarningKind::PrecedenceConflict,
            message: format!(
                "Element {node_id} has {class_count} class-based style sources — precedence unclear"
            ),
            provenance: bindings.first().map(|b| b.provenance.clone()),
        });
    }
}

fn detect_hardcoded_colors(result: &mut StyleSemanticsResult) {
    let mut warnings = Vec::new();

    for (id, bindings) in &result.style_bindings {
        for binding in bindings {
            for (prop, value) in &binding.inline_properties {
                let prop_lower = prop.to_lowercase();
                if (prop_lower.contains("color") || prop_lower.contains("background"))
                    && is_hardcoded_color(value)
                {
                    warnings.push(StyleWarning {
                        kind: StyleWarningKind::HardcodedColor,
                        message: format!(
                            "Element {id} uses hard-coded color '{value}' for {prop} — consider a design token"
                        ),
                        provenance: Some(binding.provenance.clone()),
                    });
                }
            }
        }
    }

    result.warnings.extend(warnings);
}

fn is_hardcoded_color(value: &str) -> bool {
    let v = value.trim().to_lowercase();
    v.starts_with('#')
        || v.starts_with("rgb")
        || v.starts_with("hsl")
        || matches!(
            v.as_str(),
            "red"
                | "blue"
                | "green"
                | "black"
                | "white"
                | "gray"
                | "grey"
                | "yellow"
                | "orange"
                | "purple"
                | "pink"
                | "cyan"
                | "magenta"
        )
}

fn make_element_id(file_path: &str, element: &JsxElement) -> IrNodeId {
    let content = format!("{}:{}:{}", file_path, element.tag, element.line);
    migration_ir::make_node_id(content.as_bytes())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsx_parser::ComponentKind;
    use std::collections::BTreeSet;

    fn make_project(files: Vec<(&str, FileParse)>) -> ProjectParse {
        ProjectParse {
            files: files.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            file_contents: BTreeMap::new(),
            symbol_table: BTreeMap::new(),
            component_count: 0,
            hook_usage_count: 0,
            type_count: 0,
            diagnostics: Vec::new(),
            external_imports: BTreeSet::new(),
        }
    }

    fn make_file(
        path: &str,
        components: Vec<ComponentDecl>,
        elements: Vec<JsxElement>,
    ) -> FileParse {
        FileParse {
            file: path.to_string(),
            components,
            hooks: Vec::new(),
            jsx_elements: elements,
            types: Vec::new(),
            symbols: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn make_component(name: &str, hooks: Vec<HookCall>) -> ComponentDecl {
        ComponentDecl {
            name: name.to_string(),
            kind: ComponentKind::FunctionComponent,
            is_default_export: false,
            is_named_export: true,
            props_type: None,
            hooks,
            event_handlers: Vec::new(),
            line: 1,
        }
    }

    fn make_element(tag: &str, props: Vec<JsxProp>, line: usize) -> JsxElement {
        JsxElement {
            tag: tag.to_string(),
            is_component: tag.chars().next().is_some_and(|c| c.is_uppercase()),
            is_fragment: false,
            is_self_closing: false,
            props,
            line,
        }
    }

    fn make_prop(name: &str, value: &str) -> JsxProp {
        JsxProp {
            name: name.to_string(),
            is_spread: false,
            value_snippet: Some(value.to_string()),
        }
    }

    fn make_spread_prop(name: &str) -> JsxProp {
        JsxProp {
            name: name.to_string(),
            is_spread: true,
            value_snippet: None,
        }
    }

    fn make_hook(name: &str, binding: Option<&str>, args: &str, line: usize) -> HookCall {
        HookCall {
            name: name.to_string(),
            binding: binding.map(|s| s.to_string()),
            args_snippet: args.to_string(),
            line,
        }
    }

    // ── Basic extraction ────────────────────────────────────────────────

    #[test]
    fn empty_project_produces_empty_result() {
        let project = make_project(vec![]);
        let result = extract_style_semantics(&project);
        assert!(result.tokens.is_empty());
        assert!(result.themes.is_empty());
        assert!(result.style_bindings.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn inline_style_extracted() {
        let elem = make_element(
            "div",
            vec![make_prop("style", "{{ color: 'red', fontSize: 16 }}")],
            5,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert_eq!(result.style_bindings.len(), 1);
        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].source, StyleSourceKind::InlineObject);
        assert_eq!(bindings[0].inline_properties.get("color").unwrap(), "red");
        assert_eq!(bindings[0].inline_properties.get("fontSize").unwrap(), "16");
    }

    #[test]
    fn static_classname_extracted() {
        let elem = make_element(
            "div",
            vec![make_prop("className", "\"container header\"")],
            3,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings[0].source, StyleSourceKind::StaticClassName);
        assert_eq!(bindings[0].class_refs, vec!["container", "header"]);
    }

    #[test]
    fn css_module_classname_detected() {
        let elem = make_element("div", vec![make_prop("className", "styles.container")], 3);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings[0].source, StyleSourceKind::CssModule);
        assert_eq!(bindings[0].class_refs, vec!["container"]);
    }

    #[test]
    fn dynamic_classname_with_clsx() {
        let elem = make_element(
            "div",
            vec![make_prop("className", "clsx('base', isActive && 'active')")],
            3,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings[0].source, StyleSourceKind::DynamicClassName);
        assert!(bindings[0].is_conditional);
        assert!(bindings[0].class_refs.contains(&"base".to_string()));
        assert!(bindings[0].class_refs.contains(&"active".to_string()));
    }

    #[test]
    fn sx_prop_detected_as_theme_aware() {
        let elem = make_element(
            "Box",
            vec![make_prop("sx", "{{ p: 2, bgcolor: 'primary.main' }}")],
            3,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings[0].source, StyleSourceKind::ThemeAwareProp);
        assert_eq!(bindings[0].prop_name, "sx");
    }

    #[test]
    fn spread_prop_captured() {
        let elem = make_element("div", vec![make_spread_prop("...styleProps")], 3);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings[0].source, StyleSourceKind::Spread);
        assert!(bindings[0].is_spread);
    }

    // ── Theme extraction ────────────────────────────────────────────────

    #[test]
    fn theme_from_create_theme_hook() {
        let hook = make_hook(
            "createTheme",
            Some("theme"),
            "({ primary: '#1976d2', spacing: 8 })",
            5,
        );
        let comp = make_component("App", vec![hook]);
        let file = make_file("src/App.tsx", vec![comp], vec![]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert!(!result.themes.is_empty());
        assert_eq!(result.themes[0].name, "theme");
        assert!(result.themes[0].is_default);
    }

    #[test]
    fn theme_provider_element_detected() {
        let elem = make_element("ThemeProvider", vec![make_prop("theme", "{myTheme}")], 10);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert!(!result.themes.is_empty());
        assert_eq!(
            result.themes[0].provider_component.as_deref(),
            Some("ThemeProvider")
        );
    }

    // ── Token extraction ────────────────────────────────────────────────

    #[test]
    fn design_tokens_from_create_theme() {
        let hook = make_hook(
            "createTheme",
            Some("theme"),
            "({ primaryColor: '#1976d2', spacing: '8px' })",
            5,
        );
        let comp = make_component("App", vec![hook]);
        let file = make_file("src/App.tsx", vec![comp], vec![]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert!(!result.tokens.is_empty());
        let color_token = result
            .tokens
            .values()
            .find(|t| t.name.contains("primaryColor"));
        assert!(color_token.is_some());
        assert_eq!(color_token.unwrap().category, TokenCategory::Color);
    }

    // ── Layout intent ───────────────────────────────────────────────────

    #[test]
    fn layout_intent_from_display_flex() {
        let elem = make_element(
            "div",
            vec![make_prop(
                "style",
                "{{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}",
            )],
            3,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert_eq!(result.layout_intents.len(), 1);
        let layout = result.layout_intents.values().next().unwrap();
        assert_eq!(layout.kind, LayoutKind::Flex);
        assert_eq!(layout.direction.as_deref(), Some("column"));
        assert_eq!(layout.alignment.as_deref(), Some("center"));
    }

    #[test]
    fn layout_intent_from_class_name() {
        let elem = make_element(
            "div",
            vec![make_prop("className", "\"flex items-center\"")],
            3,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert_eq!(result.layout_intents.len(), 1);
        let layout = result.layout_intents.values().next().unwrap();
        assert_eq!(layout.kind, LayoutKind::Flex);
    }

    #[test]
    fn layout_intent_grid_from_display() {
        let elem = make_element("div", vec![make_prop("style", "{{ display: 'grid' }}")], 3);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let layout = result.layout_intents.values().next().unwrap();
        assert_eq!(layout.kind, LayoutKind::Grid);
    }

    // ── Precedence & warnings ───────────────────────────────────────────

    #[test]
    fn inline_and_classname_triggers_warning() {
        let elem = make_element(
            "div",
            vec![
                make_prop("style", "{{ color: 'red' }}"),
                make_prop("className", "\"text-blue\""),
            ],
            3,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.kind == StyleWarningKind::InlineOverride)
        );
    }

    #[test]
    fn hardcoded_color_warning() {
        let elem = make_element("div", vec![make_prop("style", "{{ color: '#ff0000' }}")], 3);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.kind == StyleWarningKind::HardcodedColor)
        );
    }

    // ── Conversion ──────────────────────────────────────────────────────

    #[test]
    fn to_style_intent_conversion() {
        let elem = make_element(
            "div",
            vec![make_prop("style", "{{ display: 'flex', color: 'red' }}")],
            3,
        );
        let hook = make_hook("createTheme", Some("theme"), "({ primary: '#1976d2' })", 1);
        let comp = make_component("App", vec![hook]);
        let file = make_file("src/App.tsx", vec![comp], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);
        let intent = to_style_intent(&result);

        assert!(!intent.themes.is_empty());
        assert!(!intent.layouts.is_empty());
    }

    #[test]
    fn accessibility_meta_detects_colors() {
        let elem = make_element(
            "div",
            vec![make_prop(
                "style",
                "{{ color: 'red', fontSize: '12px', opacity: 0.5 }}",
            )],
            3,
        );
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);
        let meta = accessibility_meta(&result);

        assert!(!meta.components_with_colors.is_empty());
        assert!(!meta.components_with_font_sizes.is_empty());
        assert!(!meta.components_with_opacity.is_empty());
    }

    // ── Style sources tracking ──────────────────────────────────────────

    #[test]
    fn style_sources_tracked() {
        let elem1 = make_element("div", vec![make_prop("style", "{{ color: 'red' }}")], 3);
        let elem2 = make_element("span", vec![make_prop("className", "\"container\"")], 5);
        let file = make_file("src/App.tsx", vec![], vec![elem1, elem2]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        assert!(
            result
                .style_sources_used
                .contains(&StyleSourceKind::InlineObject)
        );
        assert!(
            result
                .style_sources_used
                .contains(&StyleSourceKind::StaticClassName)
        );
    }

    // ── Multi-file extraction ───────────────────────────────────────────

    #[test]
    fn multi_file_extraction() {
        let elem1 = make_element("div", vec![make_prop("style", "{{ margin: '8px' }}")], 3);
        let file1 = make_file("src/App.tsx", vec![], vec![elem1]);

        let elem2 = make_element("span", vec![make_prop("className", "styles.highlight")], 7);
        let file2 = make_file("src/Header.tsx", vec![], vec![elem2]);

        let project = make_project(vec![("src/App.tsx", file1), ("src/Header.tsx", file2)]);
        let result = extract_style_semantics(&project);

        assert_eq!(result.style_bindings.len(), 2);
        assert!(
            result
                .style_sources_used
                .contains(&StyleSourceKind::InlineObject)
        );
        assert!(
            result
                .style_sources_used
                .contains(&StyleSourceKind::CssModule)
        );
    }

    // ── CSS module bracket notation ─────────────────────────────────────

    #[test]
    fn css_module_bracket_notation() {
        let elem = make_element("div", vec![make_prop("className", "styles['my-class']")], 3);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings[0].source, StyleSourceKind::CssModule);
        assert!(bindings[0].class_refs.contains(&"my-class".to_string()));
    }

    // ── Serialization roundtrip ─────────────────────────────────────────

    #[test]
    fn serialization_roundtrip() {
        let elem = make_element("div", vec![make_prop("style", "{{ color: 'blue' }}")], 3);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: StyleSemanticsResult = serde_json::from_str(&json).unwrap();
        assert_eq!(
            result.style_bindings.len(),
            deserialized.style_bindings.len()
        );
    }

    // ── Token categorization ────────────────────────────────────────────

    #[test]
    fn token_categorization() {
        assert_eq!(
            categorize_token_value("primaryColor", "#1976d2"),
            TokenCategory::Color
        );
        assert_eq!(
            categorize_token_value("spacing", "8px"),
            TokenCategory::Spacing
        );
        assert_eq!(
            categorize_token_value("fontSize", "14px"),
            TokenCategory::Typography
        );
        assert_eq!(
            categorize_token_value("borderRadius", "4px"),
            TokenCategory::Border
        );
        assert_eq!(
            categorize_token_value("boxShadow", "0 2px 4px"),
            TokenCategory::Shadow
        );
        assert_eq!(
            categorize_token_value("transitionDuration", "300ms"),
            TokenCategory::Animation
        );
        assert_eq!(
            categorize_token_value("breakpointSm", "640px"),
            TokenCategory::Breakpoint
        );
        assert_eq!(
            categorize_token_value("zIndex", "100"),
            TokenCategory::ZIndex
        );
    }

    // ── Parse inline style object edge cases ────────────────────────────

    #[test]
    fn parse_empty_style_object() {
        let props = parse_inline_style_object("{{ }}");
        assert!(props.is_empty());
    }

    #[test]
    fn parse_nested_style_values() {
        let props =
            parse_inline_style_object("{{ border: '1px solid red', transform: 'rotate(45deg)' }}");
        assert_eq!(props.get("border").unwrap(), "1px solid red");
        assert_eq!(props.get("transform").unwrap(), "rotate(45deg)");
    }

    #[test]
    fn css_prop_detected() {
        let elem = make_element("div", vec![make_prop("css", "{{ color: 'green' }}")], 3);
        let file = make_file("src/App.tsx", vec![], vec![elem]);
        let project = make_project(vec![("src/App.tsx", file)]);
        let result = extract_style_semantics(&project);

        let bindings = result.style_bindings.values().next().unwrap();
        assert_eq!(bindings[0].source, StyleSourceKind::ThemeAwareProp);
        assert_eq!(bindings[0].prop_name, "css");
    }
}
