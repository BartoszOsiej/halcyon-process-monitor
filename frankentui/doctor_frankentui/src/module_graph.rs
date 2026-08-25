// SPDX-License-Identifier: Apache-2.0
//! Module graph construction and migration entrypoint detection.
//!
//! Builds a static module dependency graph from a project snapshot by
//! extracting import/export edges via lightweight regex scanning. Detects
//! migration entrypoints for monorepo and single-app layouts with
//! deterministic ranked candidates when ambiguous.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use regex_lite::Regex;
use serde::{Deserialize, Serialize};

// ── Types ────────────────────────────────────────────────────────────────

/// Unique identifier for a module within the project snapshot.
/// Always a forward-slash-separated relative path from the snapshot root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModuleId(pub String);

impl std::fmt::Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Kind of import relationship between two modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportKind {
    /// ES module `import` statement.
    Static,
    /// `import()` expression (code splitting / lazy loading).
    Dynamic,
    /// `export { ... } from '...'` or `export * from '...'`.
    ReExport,
    /// CommonJS `require(...)`.
    Require,
    /// Side-effect only import (`import './setup'`).
    SideEffect,
}

/// A directed edge in the module graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportEdge {
    pub from: ModuleId,
    pub to: ModuleId,
    pub kind: ImportKind,
    /// Raw specifier string as written in source (e.g. `./utils`, `react`).
    pub specifier: String,
    /// Line number where the import was found (1-based).
    pub line: usize,
}

/// Kind of export from a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportKind {
    Named,
    Default,
    Namespace,
    ReExport,
}

/// A single exported symbol from a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportItem {
    pub name: String,
    pub kind: ExportKind,
    /// If this is a re-export, the source specifier.
    pub source: Option<String>,
}

/// A module node in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleNode {
    pub id: ModuleId,
    pub path: String,
    pub exports: Vec<ExportItem>,
    pub import_count: usize,
    pub dynamic_import_count: usize,
    pub is_entrypoint: bool,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Source of evidence for an entrypoint candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntrypointSource {
    /// Listed in package.json main/module/exports.
    PackageJson,
    /// Conventional filename (index, main, app, _app, etc.).
    Convention,
    /// Workspace package root.
    WorkspaceRoot,
    /// Has zero incoming imports (graph root).
    GraphRoot,
    /// Referenced in scripts (start, dev, build).
    ScriptReference,
}

/// A candidate migration entrypoint with scoring rationale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrypointCandidate {
    pub path: String,
    pub score: f64,
    pub source: EntrypointSource,
    pub rationale: String,
}

/// Summary statistics for the module graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_modules: usize,
    pub total_edges: usize,
    pub static_imports: usize,
    pub dynamic_imports: usize,
    pub re_exports: usize,
    pub require_calls: usize,
    pub side_effect_imports: usize,
    pub max_depth: usize,
    pub cycle_count: usize,
    pub orphan_count: usize,
    pub external_specifiers: BTreeSet<String>,
}

/// The complete module dependency graph for a project snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleGraph {
    pub modules: BTreeMap<ModuleId, ModuleNode>,
    pub edges: Vec<ImportEdge>,
    pub entrypoints: Vec<EntrypointCandidate>,
    pub stats: GraphStats,
    pub snapshot_root: String,
}

// ── Extraction (regex-based) ─────────────────────────────────────────────

/// Raw extraction result from scanning a single file.
#[derive(Debug, Default)]
struct FileExtracts {
    imports: Vec<(String, ImportKind, usize)>,
    exports: Vec<ExportItem>,
}

/// Extract import/export statements from JS/TS source using regex.
fn extract_from_source(content: &str) -> FileExtracts {
    let mut result = FileExtracts::default();

    // Patterns compiled per call; in production we'd cache these, but for
    // diagnostic tooling the cost is acceptable.

    // Static imports: import X from 'y'; import { A } from 'y'; import 'y'
    let re_static_import =
        Regex::new(r#"(?m)^[^\S\n]*import\s+(?:(?:type\s+)?(?:\{[^}]*\}|[*]\s+as\s+\w+|\w+(?:\s*,\s*\{[^}]*\})?)\s+from\s+)?['"]([^'"]+)['"]"#)
            .expect("static import regex");

    // Dynamic imports: import('...')
    let re_dynamic_import =
        Regex::new(r#"import\s*\(\s*['"]([^'"]+)['"]\s*\)"#).expect("dynamic import regex");

    // Re-exports: export { ... } from '...'; export * from '...'
    let re_reexport = Regex::new(
        r#"(?m)^[^\S\n]*export\s+(?:(?:type\s+)?\{[^}]*\}\s+from|[*]\s+(?:as\s+\w+\s+)?from)\s+['"]([^'"]+)['"]"#,
    )
    .expect("reexport regex");

    // CommonJS require: require('...')
    let re_require =
        Regex::new(r#"(?:^|[=\s(,])require\s*\(\s*['"]([^'"]+)['"]\s*\)"#).expect("require regex");

    // Named exports: export const/let/var/function/class/enum/interface/type
    let re_named_export = Regex::new(
        r#"(?m)^[^\S\n]*export\s+(?:declare\s+)?(?:const|let|var|function\*?|class|enum|interface|type|abstract\s+class)\s+(\w+)"#,
    )
    .expect("named export regex");

    // Default export
    let re_default_export =
        Regex::new(r#"(?m)^[^\S\n]*export\s+default\s+"#).expect("default export regex");

    for (line_idx, line) in content.lines().enumerate() {
        let lineno = line_idx + 1;
        let trimmed = line.trim();

        // Skip comments.
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }

        // Re-exports (check before static imports to avoid double-counting).
        if let Some(caps) = re_reexport.captures(line)
            && let Some(m) = caps.get(1)
        {
            let spec = m.as_str().to_string();
            result
                .imports
                .push((spec.clone(), ImportKind::ReExport, lineno));

            // Extract re-exported names if possible.
            if line.contains('*') {
                result.exports.push(ExportItem {
                    name: "*".to_string(),
                    kind: ExportKind::Namespace,
                    source: Some(spec),
                });
            } else {
                result.exports.push(ExportItem {
                    name: "(re-export)".to_string(),
                    kind: ExportKind::ReExport,
                    source: Some(spec),
                });
            }
            continue;
        }

        // Static imports.
        if let Some(caps) = re_static_import.captures(line)
            && let Some(m) = caps.get(1)
        {
            let spec = m.as_str().to_string();
            // Distinguish side-effect imports.
            let kind = if trimmed.starts_with("import '") || trimmed.starts_with("import \"") {
                ImportKind::SideEffect
            } else {
                ImportKind::Static
            };
            result.imports.push((spec, kind, lineno));
        }

        // Dynamic imports.
        for caps in re_dynamic_import.captures_iter(line) {
            if let Some(m) = caps.get(1) {
                result
                    .imports
                    .push((m.as_str().to_string(), ImportKind::Dynamic, lineno));
            }
        }

        // Require calls.
        for caps in re_require.captures_iter(line) {
            if let Some(m) = caps.get(1) {
                result
                    .imports
                    .push((m.as_str().to_string(), ImportKind::Require, lineno));
            }
        }

        // Named exports.
        if let Some(caps) = re_named_export.captures(line)
            && let Some(m) = caps.get(1)
        {
            result.exports.push(ExportItem {
                name: m.as_str().to_string(),
                kind: ExportKind::Named,
                source: None,
            });
        }

        // Default export.
        if re_default_export.is_match(line) {
            result.exports.push(ExportItem {
                name: "default".to_string(),
                kind: ExportKind::Default,
                source: None,
            });
        }
    }

    result
}

// ── Specifier Resolution ─────────────────────────────────────────────────

/// JS/TS file extensions to probe when resolving bare specifiers.
const JS_EXTENSIONS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mts", ".mjs", ".cts", ".cjs"];

/// Index filenames to probe when resolving directory imports.
const INDEX_FILES: &[&str] = &[
    "index.ts",
    "index.tsx",
    "index.js",
    "index.jsx",
    "index.mts",
    "index.mjs",
];

/// Returns true if the specifier looks like a relative or absolute path.
fn is_relative_specifier(specifier: &str) -> bool {
    specifier.starts_with("./") || specifier.starts_with("../") || specifier.starts_with('/')
}

/// Normalize a path by resolving `.` and `..` components without touching
/// the filesystem (no symlink resolution).
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {} // skip `.`
            std::path::Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                }
            }
            other => components.push(other),
        }
    }
    components.iter().collect()
}

/// Resolve a relative specifier to a module ID within the snapshot.
fn resolve_specifier(
    from_dir: &Path,
    specifier: &str,
    snapshot_root: &Path,
    known_files: &HashSet<PathBuf>,
) -> Option<ModuleId> {
    if !is_relative_specifier(specifier) {
        return None; // External package — not in our graph.
    }

    let candidate_base = normalize_path(&from_dir.join(specifier));

    // 1. Exact match (already has extension).
    if known_files.contains(&candidate_base) {
        return path_to_module_id(&candidate_base, snapshot_root);
    }

    // 2. Try appending extensions.
    for ext in JS_EXTENSIONS {
        let with_ext = candidate_base.with_extension(ext.trim_start_matches('.'));
        if known_files.contains(&with_ext) {
            return path_to_module_id(&with_ext, snapshot_root);
        }
    }

    // 3. Directory import: look for index files.
    if candidate_base.is_dir() || {
        // The path might not exist yet in known_files as a dir,
        // but index files under it might.
        INDEX_FILES
            .iter()
            .any(|idx| known_files.contains(&candidate_base.join(idx)))
    } {
        for idx in INDEX_FILES {
            let index_path = candidate_base.join(idx);
            if known_files.contains(&index_path) {
                return path_to_module_id(&index_path, snapshot_root);
            }
        }
    }

    // 4. Try .ts/.tsx extension on directory import path.
    for ext in JS_EXTENSIONS {
        let dir_with_ext = candidate_base.with_extension(ext.trim_start_matches('.'));
        if known_files.contains(&dir_with_ext) {
            return path_to_module_id(&dir_with_ext, snapshot_root);
        }
    }

    None
}

/// Convert an absolute path to a ModuleId relative to the snapshot root.
fn path_to_module_id(path: &Path, snapshot_root: &Path) -> Option<ModuleId> {
    path.strip_prefix(snapshot_root)
        .ok()
        .map(|rel| ModuleId(rel.to_string_lossy().replace('\\', "/")))
}

// ── File Discovery ──────────────────────────────────────────────────────

/// Collect all JS/TS source files under a directory, skipping node_modules
/// and common non-source directories.
fn collect_source_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(root.to_path_buf());

    let skip_dirs: HashSet<&str> = [
        "node_modules",
        ".git",
        ".next",
        ".nuxt",
        "dist",
        "build",
        "out",
        "coverage",
        ".turbo",
        ".cache",
        "__pycache__",
    ]
    .into_iter()
    .collect();

    while let Some(dir) = queue.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                if !skip_dirs.contains(name) && !name.starts_with('.') {
                    queue.push_back(path);
                }
            } else if is_js_ts_file(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

/// Check if a path is a JS/TS source file by extension.
fn is_js_ts_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            matches!(
                ext,
                "js" | "jsx" | "ts" | "tsx" | "mjs" | "mts" | "cjs" | "cts"
            )
        })
}

// ── Graph Building ──────────────────────────────────────────────────────

/// Build a module dependency graph from a project snapshot directory.
///
/// Scans all JS/TS files, extracts import/export statements via regex,
/// resolves relative specifiers, and constructs the directed graph.
pub fn build_module_graph(snapshot_root: &Path) -> ModuleGraph {
    let source_files = collect_source_files(snapshot_root);
    let known_files: HashSet<PathBuf> = source_files.iter().cloned().collect();

    let mut modules: BTreeMap<ModuleId, ModuleNode> = BTreeMap::new();
    let mut edges: Vec<ImportEdge> = Vec::new();
    let mut external_specifiers: BTreeSet<String> = BTreeSet::new();

    let mut static_imports = 0usize;
    let mut dynamic_imports = 0usize;
    let mut re_exports = 0usize;
    let mut require_calls = 0usize;
    let mut side_effect_imports = 0usize;

    for file_path in &source_files {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let module_id = match path_to_module_id(file_path, snapshot_root) {
            Some(id) => id,
            None => continue,
        };

        let file_size = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);

        let extracts = extract_from_source(&content);
        let from_dir = file_path.parent().unwrap_or(snapshot_root);

        let mut dyn_count = 0usize;
        let mut import_count = 0usize;

        for (specifier, kind, line) in &extracts.imports {
            match kind {
                ImportKind::Static => static_imports += 1,
                ImportKind::Dynamic => {
                    dynamic_imports += 1;
                    dyn_count += 1;
                }
                ImportKind::ReExport => re_exports += 1,
                ImportKind::Require => require_calls += 1,
                ImportKind::SideEffect => side_effect_imports += 1,
            }

            if is_relative_specifier(specifier) {
                if let Some(target_id) =
                    resolve_specifier(from_dir, specifier, snapshot_root, &known_files)
                {
                    edges.push(ImportEdge {
                        from: module_id.clone(),
                        to: target_id,
                        kind: kind.clone(),
                        specifier: specifier.clone(),
                        line: *line,
                    });
                    import_count += 1;
                }
            } else {
                // Track external package specifiers.
                let pkg_name = extract_package_name(specifier);
                external_specifiers.insert(pkg_name);
                import_count += 1;
            }
        }

        modules.insert(
            module_id.clone(),
            ModuleNode {
                id: module_id,
                path: file_path
                    .strip_prefix(snapshot_root)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .replace('\\', "/"),
                exports: extracts.exports,
                import_count,
                dynamic_import_count: dyn_count,
                is_entrypoint: false,
                size_bytes: file_size,
            },
        );
    }

    // Compute graph metrics.
    let incoming: HashMap<&ModuleId, usize> = {
        let mut map: HashMap<&ModuleId, usize> = HashMap::new();
        for edge in &edges {
            *map.entry(&edge.to).or_default() += 1;
        }
        map
    };

    let orphan_count = modules
        .keys()
        .filter(|id| !incoming.contains_key(id) && !edges.iter().any(|e| &e.from == *id))
        .count();

    let cycles = find_cycles_internal(&modules, &edges);
    let max_depth = compute_max_depth(&modules, &edges);

    let stats = GraphStats {
        total_modules: modules.len(),
        total_edges: edges.len(),
        static_imports,
        dynamic_imports,
        re_exports,
        require_calls,
        side_effect_imports,
        max_depth,
        cycle_count: cycles.len(),
        orphan_count,
        external_specifiers,
    };

    // Detect entrypoints.
    let entrypoints = detect_entrypoints(snapshot_root, &modules, &edges);

    // Mark entrypoint modules.
    for candidate in &entrypoints {
        let mid = ModuleId(candidate.path.clone());
        if let Some(node) = modules.get_mut(&mid) {
            node.is_entrypoint = true;
        }
    }

    ModuleGraph {
        modules,
        edges,
        entrypoints,
        stats,
        snapshot_root: snapshot_root.display().to_string(),
    }
}

/// Extract the bare package name from a specifier (handles scoped packages).
fn extract_package_name(specifier: &str) -> String {
    if specifier.starts_with('@') {
        // Scoped: @scope/pkg/subpath → @scope/pkg
        let parts: Vec<&str> = specifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            format!("{}/{}", parts[0], parts[1])
        } else {
            specifier.to_string()
        }
    } else {
        // Unscoped: pkg/subpath → pkg
        specifier.split('/').next().unwrap_or(specifier).to_string()
    }
}

// ── Entrypoint Detection ─────────────────────────────────────────────────

/// Conventional entrypoint filenames, scored by decreasing confidence.
const CONVENTIONAL_ENTRYPOINTS: &[(&str, f64)] = &[
    ("src/index.tsx", 0.90),
    ("src/index.ts", 0.90),
    ("src/index.js", 0.85),
    ("src/index.jsx", 0.85),
    ("src/main.tsx", 0.85),
    ("src/main.ts", 0.85),
    ("src/main.js", 0.80),
    ("src/App.tsx", 0.80),
    ("src/App.ts", 0.75),
    ("src/App.jsx", 0.75),
    ("src/App.js", 0.70),
    ("pages/_app.tsx", 0.95),
    ("pages/_app.ts", 0.90),
    ("pages/_app.js", 0.85),
    ("app/layout.tsx", 0.95),
    ("app/layout.ts", 0.90),
    ("app/page.tsx", 0.85),
    ("index.tsx", 0.70),
    ("index.ts", 0.70),
    ("index.js", 0.65),
    ("index.jsx", 0.65),
];

/// Detect migration entrypoints for the project.
///
/// Produces deterministic ranked candidates with rationale. Sources checked:
/// 1. package.json fields (main, module, exports, source, browser)
/// 2. Workspace package roots (monorepo sub-apps)
/// 3. Conventional filenames
/// 4. Graph roots (modules with zero incoming edges)
fn detect_entrypoints(
    snapshot_root: &Path,
    modules: &BTreeMap<ModuleId, ModuleNode>,
    edges: &[ImportEdge],
) -> Vec<EntrypointCandidate> {
    let mut candidates: Vec<EntrypointCandidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1. package.json fields.
    if let Some(pkg_entries) = read_package_json_entries(snapshot_root) {
        for (field, path) in pkg_entries {
            if !seen.contains(&path) && modules.contains_key(&ModuleId(path.clone())) {
                seen.insert(path.clone());
                candidates.push(EntrypointCandidate {
                    path: path.clone(),
                    score: 1.0,
                    source: EntrypointSource::PackageJson,
                    rationale: format!("Listed in package.json \"{field}\" field"),
                });
            }
        }
    }

    // 2. Script references (start, dev, build scripts pointing to files).
    if let Some(script_refs) = read_script_references(snapshot_root) {
        for path in script_refs {
            if !seen.contains(&path) && modules.contains_key(&ModuleId(path.clone())) {
                seen.insert(path.clone());
                candidates.push(EntrypointCandidate {
                    path: path.clone(),
                    score: 0.75,
                    source: EntrypointSource::ScriptReference,
                    rationale: "Referenced in package.json scripts".to_string(),
                });
            }
        }
    }

    // 3. Workspace sub-app roots.
    let workspace_roots = detect_workspace_entrypoints(snapshot_root);
    for path in workspace_roots {
        if !seen.contains(&path) && modules.contains_key(&ModuleId(path.clone())) {
            seen.insert(path.clone());
            candidates.push(EntrypointCandidate {
                path: path.clone(),
                score: 0.85,
                source: EntrypointSource::WorkspaceRoot,
                rationale: "Workspace package entrypoint".to_string(),
            });
        }
    }

    // 4. Conventional filenames.
    for (filename, score) in CONVENTIONAL_ENTRYPOINTS {
        let path = filename.to_string();
        if !seen.contains(&path) && modules.contains_key(&ModuleId(path.clone())) {
            seen.insert(path.clone());
            candidates.push(EntrypointCandidate {
                path,
                score: *score,
                source: EntrypointSource::Convention,
                rationale: format!("Conventional entrypoint filename: {filename}"),
            });
        }
    }

    // 5. Graph roots: modules with no incoming edges but with outgoing edges.
    let incoming: HashSet<&ModuleId> = edges.iter().map(|e| &e.to).collect();
    let has_outgoing: HashSet<&ModuleId> = edges.iter().map(|e| &e.from).collect();

    let mut graph_roots: Vec<&ModuleId> = modules
        .keys()
        .filter(|id| !incoming.contains(id) && has_outgoing.contains(id))
        .collect();
    graph_roots.sort(); // Deterministic ordering.

    for root_id in graph_roots {
        let path = root_id.0.clone();
        if !seen.contains(&path) {
            seen.insert(path.clone());
            let outgoing_count = edges.iter().filter(|e| &e.from == root_id).count();
            candidates.push(EntrypointCandidate {
                path,
                score: 0.5 + (outgoing_count as f64 * 0.02).min(0.3),
                source: EntrypointSource::GraphRoot,
                rationale: format!(
                    "Graph root with {outgoing_count} outgoing imports, zero incoming"
                ),
            });
        }
    }

    // Sort by score descending, then path for stability.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    candidates
}

/// Read entrypoint fields from package.json.
fn read_package_json_entries(root: &Path) -> Option<Vec<(String, String)>> {
    let pkg_path = root.join("package.json");
    let content = std::fs::read_to_string(pkg_path).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mut entries = Vec::new();

    for field in ["main", "module", "source", "browser"] {
        if let Some(val) = pkg.get(field).and_then(|v| v.as_str()) {
            let normalized = normalize_entry_path(val);
            entries.push((field.to_string(), normalized));
        }
    }

    // exports field can be a string or an object with "." key.
    if let Some(exports) = pkg.get("exports") {
        if let Some(val) = exports.as_str() {
            entries.push(("exports".to_string(), normalize_entry_path(val)));
        } else if let Some(obj) = exports.as_object()
            && let Some(dot) = obj.get(".")
        {
            if let Some(val) = dot.as_str() {
                entries.push(("exports[\".\"]".to_string(), normalize_entry_path(val)));
            } else if let Some(inner) = dot.as_object() {
                for key in ["import", "require", "default"] {
                    if let Some(val) = inner.get(key).and_then(|v| v.as_str()) {
                        entries.push((format!("exports[\".\"].{key}"), normalize_entry_path(val)));
                    }
                }
            }
        }
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Normalize an entry path by stripping leading `./`.
fn normalize_entry_path(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}

/// Extract file references from package.json scripts.
fn read_script_references(root: &Path) -> Option<Vec<String>> {
    let pkg_path = root.join("package.json");
    let content = std::fs::read_to_string(pkg_path).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&content).ok()?;
    let scripts = pkg.get("scripts")?.as_object()?;

    let re = Regex::new(r#"(?:node|ts-node|tsx|npx ts-node)\s+(\S+\.(?:ts|tsx|js|jsx|mjs))"#)
        .expect("script ref regex");

    let mut refs = Vec::new();
    for (_key, val) in scripts {
        if let Some(script) = val.as_str() {
            for caps in re.captures_iter(script) {
                if let Some(m) = caps.get(1) {
                    refs.push(normalize_entry_path(m.as_str()));
                }
            }
        }
    }

    if refs.is_empty() { None } else { Some(refs) }
}

/// Detect workspace sub-app entrypoints by scanning workspace package.json files.
fn detect_workspace_entrypoints(root: &Path) -> Vec<String> {
    let mut entrypoints = Vec::new();

    // Read root package.json for workspaces field.
    let pkg_path = root.join("package.json");
    let content = match std::fs::read_to_string(pkg_path) {
        Ok(c) => c,
        Err(_) => return entrypoints,
    };
    let pkg: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return entrypoints,
    };

    let workspace_globs = match pkg.get("workspaces") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect::<Vec<_>>(),
        Some(serde_json::Value::Object(obj)) => {
            if let Some(serde_json::Value::Array(arr)) = obj.get("packages") {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    };

    if workspace_globs.is_empty() {
        return entrypoints;
    }

    // For each workspace glob pattern, find matching directories.
    for glob in &workspace_globs {
        let base = glob.trim_end_matches("/*").trim_end_matches('*');
        let base_dir = root.join(base);
        if !base_dir.is_dir() {
            continue;
        }

        let entries = match std::fs::read_dir(&base_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let sub_pkg = path.join("package.json");
            if !sub_pkg.exists() {
                continue;
            }

            // Look for main/module/source field in sub-package.
            if let Ok(sub_content) = std::fs::read_to_string(&sub_pkg)
                && let Ok(sub_val) = serde_json::from_str::<serde_json::Value>(&sub_content)
            {
                for field in ["main", "module", "source"] {
                    if let Some(val) = sub_val.get(field).and_then(|v| v.as_str()) {
                        let rel = path
                            .strip_prefix(root)
                            .unwrap_or(&path)
                            .join(normalize_entry_path(val));
                        entrypoints.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }

    entrypoints.sort();
    entrypoints
}

// ── Cycle Detection ──────────────────────────────────────────────────────

/// Find all cycles in the module graph using iterative DFS with Tarjan-like
/// back-edge detection. Returns groups of module IDs forming cycles.
pub fn find_cycles(graph: &ModuleGraph) -> Vec<Vec<ModuleId>> {
    find_cycles_internal(&graph.modules, &graph.edges)
}

fn find_cycles_internal(
    modules: &BTreeMap<ModuleId, ModuleNode>,
    edges: &[ImportEdge],
) -> Vec<Vec<ModuleId>> {
    // Build adjacency list.
    let mut adj: HashMap<&ModuleId, Vec<&ModuleId>> = HashMap::new();
    for edge in edges {
        adj.entry(&edge.from).or_default().push(&edge.to);
    }

    let mut visited: HashSet<&ModuleId> = HashSet::new();
    let mut on_stack: HashSet<&ModuleId> = HashSet::new();
    let mut cycles: Vec<Vec<ModuleId>> = Vec::new();

    for module_id in modules.keys() {
        if visited.contains(module_id) {
            continue;
        }

        // Iterative DFS.
        let mut stack: Vec<(&ModuleId, usize)> = vec![(module_id, 0)];
        let mut path: Vec<&ModuleId> = Vec::new();

        while let Some((node, idx)) = stack.last_mut() {
            if *idx == 0 {
                visited.insert(node);
                on_stack.insert(node);
                path.push(node);
            }

            let neighbors = adj.get(node).map(|v| v.as_slice()).unwrap_or(&[]);

            if *idx < neighbors.len() {
                let next = neighbors[*idx];
                *idx += 1;

                if on_stack.contains(next) {
                    // Found a cycle — extract it.
                    if let Some(pos) = path.iter().position(|n| *n == next) {
                        let cycle: Vec<ModuleId> =
                            path[pos..].iter().map(|n| (*n).clone()).collect();
                        if cycle.len() > 1 {
                            cycles.push(cycle);
                        }
                    }
                } else if !visited.contains(next) {
                    stack.push((next, 0));
                }
            } else {
                on_stack.remove(node);
                path.pop();
                stack.pop();
            }
        }
    }

    // Deduplicate cycles by sorting each cycle and deduplicating.
    let mut seen_cycles: HashSet<Vec<ModuleId>> = HashSet::new();
    cycles.retain(|cycle| {
        let mut normalized = cycle.clone();
        // Rotate so smallest element is first for canonical form.
        if let Some(min_pos) = normalized
            .iter()
            .enumerate()
            .min_by_key(|(_, id)| &id.0)
            .map(|(i, _)| i)
        {
            normalized.rotate_left(min_pos);
        }
        seen_cycles.insert(normalized)
    });

    cycles
}

// ── Max Depth ────────────────────────────────────────────────────────────

/// Compute the maximum depth (longest path from any root) in the DAG.
/// For cyclic graphs, uses BFS with cycle breaking.
fn compute_max_depth(modules: &BTreeMap<ModuleId, ModuleNode>, edges: &[ImportEdge]) -> usize {
    let mut adj: HashMap<&ModuleId, Vec<&ModuleId>> = HashMap::new();
    let mut incoming: HashMap<&ModuleId, usize> = HashMap::new();

    for id in modules.keys() {
        incoming.entry(id).or_insert(0);
    }

    for edge in edges {
        if modules.contains_key(&edge.to) {
            adj.entry(&edge.from).or_default().push(&edge.to);
            *incoming.entry(&edge.to).or_default() += 1;
        }
    }

    // Kahn's algorithm for topological BFS with depth tracking.
    let mut queue: VecDeque<&ModuleId> = VecDeque::new();
    let mut depth: HashMap<&ModuleId, usize> = HashMap::new();

    for (id, &count) in &incoming {
        if count == 0 {
            queue.push_back(id);
            depth.insert(id, 0);
        }
    }

    let mut max_depth = 0;

    while let Some(node) = queue.pop_front() {
        let node_depth = depth[node];
        max_depth = max_depth.max(node_depth);

        if let Some(neighbors) = adj.get(node) {
            for &next in neighbors {
                let next_count = incoming.get_mut(next).unwrap();
                *next_count -= 1;

                let candidate_depth = node_depth + 1;
                let current = depth.entry(next).or_insert(0);
                if candidate_depth > *current {
                    *current = candidate_depth;
                }

                if *next_count == 0 {
                    queue.push_back(next);
                }
            }
        }
    }

    max_depth
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a minimal project layout for testing.
    fn setup_project(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (path, content) in files {
            let full = dir.path().join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, content).unwrap();
        }
        dir
    }

    #[test]
    fn extract_static_imports() {
        let src = r#"
import React from 'react';
import { useState } from 'react';
import * as Utils from './utils';
import './globals.css';
"#;
        let extracts = extract_from_source(src);
        assert_eq!(extracts.imports.len(), 4);
        assert_eq!(extracts.imports[0].0, "react");
        assert_eq!(extracts.imports[0].1, ImportKind::Static);
        assert_eq!(extracts.imports[1].0, "react");
        assert_eq!(extracts.imports[2].0, "./utils");
        assert_eq!(extracts.imports[3].0, "./globals.css");
        assert_eq!(extracts.imports[3].1, ImportKind::SideEffect);
    }

    #[test]
    fn extract_dynamic_imports() {
        let src = r#"
const Lazy = React.lazy(() => import('./LazyComponent'));
const mod = await import('./dynamic');
"#;
        let extracts = extract_from_source(src);
        let dynamics: Vec<_> = extracts
            .imports
            .iter()
            .filter(|(_, k, _)| *k == ImportKind::Dynamic)
            .collect();
        assert_eq!(dynamics.len(), 2);
        assert_eq!(dynamics[0].0, "./LazyComponent");
        assert_eq!(dynamics[1].0, "./dynamic");
    }

    #[test]
    fn extract_reexports() {
        let src = r#"
export { Button } from './Button';
export * from './utils';
export * as Icons from './icons';
export type { Props } from './types';
"#;
        let extracts = extract_from_source(src);
        let reexports: Vec<_> = extracts
            .imports
            .iter()
            .filter(|(_, k, _)| *k == ImportKind::ReExport)
            .collect();
        assert_eq!(reexports.len(), 4);
        assert_eq!(reexports[0].0, "./Button");
        assert_eq!(reexports[1].0, "./utils");
        assert_eq!(reexports[2].0, "./icons");
        assert_eq!(reexports[3].0, "./types");
    }

    #[test]
    fn extract_require_calls() {
        let src = r#"
const fs = require('fs');
const path = require('path');
const local = require('./local');
"#;
        let extracts = extract_from_source(src);
        let requires: Vec<_> = extracts
            .imports
            .iter()
            .filter(|(_, k, _)| *k == ImportKind::Require)
            .collect();
        assert_eq!(requires.len(), 3);
        assert_eq!(requires[2].0, "./local");
    }

    #[test]
    fn extract_named_exports() {
        let src = r#"
export const FOO = 42;
export function bar() {}
export class Baz {}
export default function main() {}
"#;
        let extracts = extract_from_source(src);
        let named: Vec<_> = extracts
            .exports
            .iter()
            .filter(|e| e.kind == ExportKind::Named)
            .collect();
        assert_eq!(named.len(), 3);
        assert_eq!(named[0].name, "FOO");
        assert_eq!(named[1].name, "bar");
        assert_eq!(named[2].name, "Baz");

        let defaults: Vec<_> = extracts
            .exports
            .iter()
            .filter(|e| e.kind == ExportKind::Default)
            .collect();
        assert_eq!(defaults.len(), 1);
    }

    #[test]
    fn package_name_extraction() {
        assert_eq!(extract_package_name("react"), "react");
        assert_eq!(extract_package_name("react/jsx-runtime"), "react");
        assert_eq!(extract_package_name("@scope/pkg"), "@scope/pkg");
        assert_eq!(extract_package_name("@scope/pkg/deep"), "@scope/pkg");
    }

    #[test]
    fn build_graph_simple_project() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test","main":"src/index.ts"}"#),
            (
                "src/index.ts",
                r#"
import { greet } from './utils';
import React from 'react';
export default function App() {}
"#,
            ),
            (
                "src/utils.ts",
                r#"
export function greet() { return 'hello'; }
"#,
            ),
        ]);

        let graph = build_module_graph(dir.path());
        assert_eq!(graph.modules.len(), 2);
        assert!(
            graph
                .modules
                .contains_key(&ModuleId("src/index.ts".to_string()))
        );
        assert!(
            graph
                .modules
                .contains_key(&ModuleId("src/utils.ts".to_string()))
        );

        // One internal edge: index.ts → utils.ts.
        let internal_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == ImportKind::Static)
            .collect();
        assert_eq!(internal_edges.len(), 1);
        assert_eq!(internal_edges[0].from.0, "src/index.ts");
        assert_eq!(internal_edges[0].to.0, "src/utils.ts");

        // react is tracked as external.
        assert!(graph.stats.external_specifiers.contains("react"));
    }

    #[test]
    fn entrypoint_from_package_json() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test","main":"./src/index.ts"}"#),
            ("src/index.ts", "export default 42;"),
        ]);

        let graph = build_module_graph(dir.path());
        assert!(!graph.entrypoints.is_empty());
        assert_eq!(graph.entrypoints[0].path, "src/index.ts");
        assert_eq!(graph.entrypoints[0].source, EntrypointSource::PackageJson);
        assert!((graph.entrypoints[0].score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn entrypoint_from_convention() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/index.tsx", "export default function App() {}"),
        ]);

        let graph = build_module_graph(dir.path());
        assert!(!graph.entrypoints.is_empty());
        assert_eq!(graph.entrypoints[0].path, "src/index.tsx");
        assert_eq!(graph.entrypoints[0].source, EntrypointSource::Convention);
    }

    #[test]
    fn entrypoint_from_graph_root() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            (
                "src/bootstrap.ts",
                "import { init } from './core';\nexport const run = init;",
            ),
            ("src/core.ts", "export function init() {}"),
        ]);

        let graph = build_module_graph(dir.path());
        let graph_roots: Vec<_> = graph
            .entrypoints
            .iter()
            .filter(|c| c.source == EntrypointSource::GraphRoot)
            .collect();
        assert!(!graph_roots.is_empty());
        assert_eq!(graph_roots[0].path, "src/bootstrap.ts");
    }

    #[test]
    fn cycle_detection() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/a.ts", "import { b } from './b';\nexport const a = 1;"),
            ("src/b.ts", "import { a } from './a';\nexport const b = 2;"),
        ]);

        let graph = build_module_graph(dir.path());
        assert!(graph.stats.cycle_count > 0);
    }

    #[test]
    fn no_cycles_in_dag() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/a.ts", "import { b } from './b';\nexport const a = 1;"),
            ("src/b.ts", "import { c } from './c';\nexport const b = 2;"),
            ("src/c.ts", "export const c = 3;"),
        ]);

        let graph = build_module_graph(dir.path());
        assert_eq!(graph.stats.cycle_count, 0);
        assert_eq!(graph.stats.max_depth, 2);
    }

    #[test]
    fn graph_stats_are_correct() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            (
                "src/index.ts",
                r#"
import { a } from './a';
import('./lazy');
const x = require('./req');
import './side-effect';
export { b } from './b';
"#,
            ),
            ("src/a.ts", "export const a = 1;"),
            ("src/lazy.ts", "export const lazy = 2;"),
            ("src/req.ts", "module.exports = 3;"),
            ("src/side-effect.ts", "console.log('init');"),
            ("src/b.ts", "export const b = 4;"),
        ]);

        let graph = build_module_graph(dir.path());
        assert_eq!(graph.stats.total_modules, 6);
        // Internal edges: ./a (static), ./lazy (dynamic), ./req (require),
        // ./side-effect (side-effect), ./b (re-export) = 5
        assert_eq!(graph.stats.total_edges, 5);
        assert_eq!(graph.stats.static_imports, 1);
        assert_eq!(graph.stats.dynamic_imports, 1);
        assert_eq!(graph.stats.require_calls, 1);
        assert_eq!(graph.stats.side_effect_imports, 1);
        assert_eq!(graph.stats.re_exports, 1);
    }

    #[test]
    fn skips_node_modules() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/index.ts", "export const x = 1;"),
            ("node_modules/pkg/index.js", "module.exports = {};"),
        ]);

        let graph = build_module_graph(dir.path());
        assert_eq!(graph.modules.len(), 1);
        assert!(
            graph
                .modules
                .contains_key(&ModuleId("src/index.ts".to_string()))
        );
    }

    #[test]
    fn resolves_extension_omission() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/index.ts", "import { foo } from './foo';"),
            ("src/foo.ts", "export const foo = 1;"),
        ]);

        let graph = build_module_graph(dir.path());
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].to.0, "src/foo.ts");
    }

    #[test]
    fn resolves_directory_index() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/index.ts", "import { bar } from './components';"),
            ("src/components/index.ts", "export { bar } from './bar';"),
            ("src/components/bar.ts", "export const bar = 1;"),
        ]);

        let graph = build_module_graph(dir.path());
        let edge = graph
            .edges
            .iter()
            .find(|e| e.from.0 == "src/index.ts")
            .expect("should have edge from index.ts");
        assert_eq!(edge.to.0, "src/components/index.ts");
    }

    #[test]
    fn orphan_modules_detected() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/index.ts", "export const x = 1;"),
            ("src/orphan.ts", "export const y = 2;"),
        ]);

        let graph = build_module_graph(dir.path());
        // Both modules have zero edges, so both are orphans.
        assert_eq!(graph.stats.orphan_count, 2);
    }

    #[test]
    fn entrypoints_are_deterministically_sorted() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test","main":"./src/index.ts"}"#),
            ("src/index.ts", "export default 1;"),
            ("src/index.tsx", "export default 2;"),
        ]);

        let graph1 = build_module_graph(dir.path());
        let graph2 = build_module_graph(dir.path());

        assert_eq!(graph1.entrypoints.len(), graph2.entrypoints.len());
        for (a, b) in graph1.entrypoints.iter().zip(graph2.entrypoints.iter()) {
            assert_eq!(a.path, b.path);
            assert!((a.score - b.score).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn graph_serializes_to_json() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            (
                "src/index.ts",
                "import { a } from './a';\nexport default 1;",
            ),
            ("src/a.ts", "export const a = 1;"),
        ]);

        let graph = build_module_graph(dir.path());
        let json = serde_json::to_string_pretty(&graph).unwrap();
        assert!(json.contains("src/index.ts"));
        assert!(json.contains("src/a.ts"));

        // Round-trip.
        let parsed: ModuleGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.modules.len(), graph.modules.len());
        assert_eq!(parsed.edges.len(), graph.edges.len());
    }

    #[test]
    fn type_imports_are_extracted() {
        let src = r#"
import type { Foo } from './types';
import { type Bar, baz } from './mixed';
"#;
        let extracts = extract_from_source(src);
        assert_eq!(extracts.imports.len(), 2);
        assert_eq!(extracts.imports[0].0, "./types");
        assert_eq!(extracts.imports[1].0, "./mixed");
    }

    #[test]
    fn comments_are_skipped() {
        let src = r#"
// import { unused } from './unused';
/* import { also_unused } from './also_unused'; */
import { real } from './real';
"#;
        let extracts = extract_from_source(src);
        // Should only find the real import.
        let real_imports: Vec<_> = extracts
            .imports
            .iter()
            .filter(|(spec, _, _)| spec == "./real")
            .collect();
        assert_eq!(real_imports.len(), 1);
    }

    #[test]
    fn monorepo_workspace_detection() {
        let dir = setup_project(&[
            (
                "package.json",
                r#"{"name":"root","workspaces":["packages/*"]}"#,
            ),
            (
                "packages/app/package.json",
                r#"{"name":"@test/app","main":"./src/index.ts"}"#,
            ),
            ("packages/app/src/index.ts", "export default 1;"),
            (
                "packages/lib/package.json",
                r#"{"name":"@test/lib","main":"./src/index.ts"}"#,
            ),
            ("packages/lib/src/index.ts", "export const lib = 1;"),
        ]);

        let graph = build_module_graph(dir.path());
        let workspace_entries: Vec<_> = graph
            .entrypoints
            .iter()
            .filter(|c| c.source == EntrypointSource::WorkspaceRoot)
            .collect();
        assert_eq!(workspace_entries.len(), 2);
    }

    #[test]
    fn empty_project_produces_empty_graph() {
        let dir = setup_project(&[("package.json", r#"{"name":"empty"}"#)]);

        let graph = build_module_graph(dir.path());
        assert_eq!(graph.modules.len(), 0);
        assert_eq!(graph.edges.len(), 0);
        assert!(graph.entrypoints.is_empty());
        assert_eq!(graph.stats.total_modules, 0);
    }

    #[test]
    fn exports_field_object_resolution() {
        let dir = setup_project(&[
            (
                "package.json",
                r#"{"name":"test","exports":{".":{"import":"./src/index.mjs","require":"./src/index.cjs"}}}"#,
            ),
            ("src/index.mjs", "export default 1;"),
            ("src/index.cjs", "module.exports = 1;"),
        ]);

        let graph = build_module_graph(dir.path());
        let pkg_entries: Vec<_> = graph
            .entrypoints
            .iter()
            .filter(|c| c.source == EntrypointSource::PackageJson)
            .collect();
        assert!(pkg_entries.len() >= 2);
    }

    #[test]
    fn max_depth_linear_chain() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/a.ts", "import { b } from './b';"),
            ("src/b.ts", "import { c } from './c';"),
            ("src/c.ts", "import { d } from './d';"),
            ("src/d.ts", "export const d = 1;"),
        ]);

        let graph = build_module_graph(dir.path());
        assert_eq!(graph.stats.max_depth, 3);
    }

    #[test]
    fn find_cycles_api() {
        let dir = setup_project(&[
            ("package.json", r#"{"name":"test"}"#),
            ("src/a.ts", "import { b } from './b';\nexport const a = 1;"),
            ("src/b.ts", "import { c } from './c';\nexport const b = 1;"),
            ("src/c.ts", "import { a } from './a';\nexport const c = 1;"),
        ]);

        let graph = build_module_graph(dir.path());
        let cycles = find_cycles(&graph);
        assert!(!cycles.is_empty());
        // The cycle should contain a, b, c.
        let cycle = &cycles[0];
        assert_eq!(cycle.len(), 3);
    }

    #[test]
    fn module_id_display() {
        let id = ModuleId("src/index.ts".to_string());
        assert_eq!(format!("{id}"), "src/index.ts");
    }

    #[test]
    fn normalize_entry_strips_prefix() {
        assert_eq!(normalize_entry_path("./src/index.ts"), "src/index.ts");
        assert_eq!(normalize_entry_path("src/index.ts"), "src/index.ts");
    }

    #[test]
    fn is_relative_specifier_cases() {
        assert!(is_relative_specifier("./foo"));
        assert!(is_relative_specifier("../bar"));
        assert!(is_relative_specifier("/abs"));
        assert!(!is_relative_specifier("react"));
        assert!(!is_relative_specifier("@scope/pkg"));
    }
}
