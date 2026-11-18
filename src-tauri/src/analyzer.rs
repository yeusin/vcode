use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tree_sitter::{Language, Node, Parser};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeProgress {
    pub phase: String,
    pub processed: usize,
    pub total: usize,
    pub elapsed_ms: u64,
    pub eta_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Text,
}

impl Lang {
    fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.eq_ignore_ascii_case(".DS_Store") || name == "Thumbs.db" {
            return None;
        }
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            return match ext.to_ascii_lowercase().as_str() {
                "py" => Some(Lang::Python),
                "ts" => Some(Lang::TypeScript),
                "tsx" => Some(Lang::Tsx),
                "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
                "rs" => Some(Lang::Rust),
                "go" => Some(Lang::Go),
                "java" => Some(Lang::Java),
                "c" | "h" => Some(Lang::C),
                "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some(Lang::Cpp),
                "cs" => Some(Lang::CSharp),
                "rb" => Some(Lang::Ruby),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "icns" | "tiff"
                | "tif" | "avif" | "pdf" | "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2"
                | "7z" | "rar" | "exe" | "dll" | "so" | "dylib" | "a" | "lib" | "o"
                | "obj" | "pdb" | "wasm" | "ttf" | "otf" | "woff" | "woff2" | "eot"
                | "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" | "mp4" | "mov" | "avi"
                | "mkv" | "webm" | "pyc" | "pyo" | "pyd" | "class" | "jar" | "war"
                | "db" | "sqlite" | "sqlite3" | "bin" | "dat" | "dmg" | "pkg" | "deb"
                | "rpm" | "msi" | "appimage" => None,
                _ => Some(Lang::Text),
            };
        }
        Some(Lang::Text)
    }

    fn ts_language(self) -> Option<Language> {
        match self {
            Lang::Python => Some(tree_sitter_python::LANGUAGE.into()),
            Lang::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            Lang::Tsx => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            Lang::JavaScript => Some(tree_sitter_javascript::LANGUAGE.into()),
            Lang::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
            Lang::Go => Some(tree_sitter_go::LANGUAGE.into()),
            Lang::Java => Some(tree_sitter_java::LANGUAGE.into()),
            Lang::C => Some(tree_sitter_c::LANGUAGE.into()),
            Lang::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
            Lang::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
            Lang::Ruby => Some(tree_sitter_ruby::LANGUAGE.into()),
            Lang::Text => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GroupNode {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub parent_id: Option<String>,
    pub file_path: Option<String>,
    pub is_package: bool,
    pub docstring: Option<String>,
    pub line_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FunctionNode {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub parent_id: String,
    pub file_path: String,
    pub is_async: bool,
    pub is_method: bool,
    pub decorators: Vec<String>,
    pub signature: String,
    pub docstring: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub code: String,
    pub complexity: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub kind: String,
    pub source_id: String,
    pub target_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ParseError {
    pub file_path: String,
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStats {
    pub total_files: usize,
    pub total_classes: usize,
    pub total_functions: usize,
    pub total_import_edges: usize,
    pub total_call_edges: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGraph {
    pub root_name: String,
    pub root_path: String,
    pub groups: Vec<GroupNode>,
    pub functions: Vec<FunctionNode>,
    pub edges: Vec<GraphEdge>,
    pub parse_errors: Vec<ParseError>,
    pub stats: ProjectStats,
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    ".env",
    "node_modules",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    "dist",
    "build",
    ".eggs",
    "target",
    ".next",
    ".turbo",
];

fn should_skip_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name) || name.ends_with(".egg-info")
}

pub fn is_watched_path(path: &Path) -> bool {
    for comp in path.components() {
        if let std::path::Component::Normal(s) = comp {
            if let Some(name) = s.to_str() {
                if should_skip_dir(name) {
                    return false;
                }
            }
        }
    }
    Lang::from_path(path).is_some()
}

fn node_text<'a>(node: Node<'a>, src: &'a str) -> &'a str {
    &src[node.byte_range()]
}

fn dedent(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
        .min()
        .unwrap_or(0);

    lines
        .iter()
        .map(|l| {
            if l.len() >= min_indent && !l.trim().is_empty() {
                &l[min_indent..]
            } else {
                l.trim_end()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn slice_lines(src: &str, start_line_1: usize, end_line_1: usize) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let s = start_line_1.saturating_sub(1).min(lines.len());
    let e = end_line_1.min(lines.len());
    if s >= e {
        return String::new();
    }
    dedent(&lines[s..e].join("\n"))
}

fn to_posix(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

struct PendingCall {
    fn_id: String,
    module_id: String,
    class_id: Option<String>,
    receiver: Option<String>,
    callee: String,
}

struct ParsedModuleInfo {
    module_id: String,
    rel_posix: String,
    lang: Lang,
    raw_imports: Vec<String>,
    symbol_imports: HashMap<String, (String, String)>, // local -> (module_hint, symbol)
    module_aliases: HashMap<String, String>,           // local_alias -> module_hint
}

const GRAPH_CACHE_SCHEMA_VERSION: u64 = 1;

#[derive(Serialize, Deserialize)]
struct CachedProjectGraph {
    schema_version: u64,
    fingerprint: u64,
    graph: ProjectGraph,
}

fn fnv1a_64_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn graph_cache_file(root: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("vcode")
        .join("graph-cache");
    let root_str = root.to_string_lossy();
    let hash = fnv1a_64_bytes(0xcbf29ce484222325, root_str.as_bytes());
    base.join(format!("{hash:016x}.json"))
}

fn compute_source_fingerprint(root: &Path, source_files: &[(PathBuf, Lang)]) -> u64 {
    let mut h = fnv1a_64_bytes(0xcbf29ce484222325, &GRAPH_CACHE_SCHEMA_VERSION.to_le_bytes());
    for (path, _) in source_files {
        let rel = path.strip_prefix(root).unwrap_or(path);
        h = fnv1a_64_bytes(h, to_posix(rel).as_bytes());
        if let Ok(meta) = fs::metadata(path) {
            h = fnv1a_64_bytes(h, &meta.len().to_le_bytes());
            if let Ok(mtime) = meta.modified() {
                if let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH) {
                    h = fnv1a_64_bytes(h, &dur.as_nanos().to_le_bytes());
                }
            }
        }
    }
    h
}

fn load_cached_graph(cache_file: &Path, fingerprint: u64) -> Option<ProjectGraph> {
    let bytes = fs::read(cache_file).ok()?;
    let cached: CachedProjectGraph = serde_json::from_slice(&bytes).ok()?;
    if cached.schema_version == GRAPH_CACHE_SCHEMA_VERSION && cached.fingerprint == fingerprint {
        Some(cached.graph)
    } else {
        None
    }
}

fn save_cached_graph(cache_file: &Path, fingerprint: u64, graph: &ProjectGraph) {
    if let Some(parent) = cache_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let payload = CachedProjectGraph {
        schema_version: GRAPH_CACHE_SCHEMA_VERSION,
        fingerprint,
        graph: graph.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec(&payload) {
        let tmp = cache_file.with_extension("json.tmp");
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(&tmp, cache_file);
        }
    }
}

pub fn analyze_project(root_path: &Path) -> Result<ProjectGraph, String> {
    analyze_project_with_progress(root_path, |_| {})
}

pub fn analyze_project_with_progress<F>(
    root_path: &Path,
    mut on_progress: F,
) -> Result<ProjectGraph, String>
where
    F: FnMut(AnalyzeProgress),
{
    let root = root_path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve path {}: {e}", root_path.display()))?;
    if !root.is_dir() {
        return Err(format!("Not a directory: {}", root.display()));
    }

    let root_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();

    let scan_start = Instant::now();
    let mut last_emit = scan_start;
    on_progress(AnalyzeProgress {
        phase: "Scanning files...".to_string(),
        processed: 0,
        total: 0,
        elapsed_ms: 0,
        eta_ms: None,
    });

    let mut source_files: Vec<(PathBuf, Lang)> = Vec::new();
    let mut walker = WalkBuilder::new(&root);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                let name = e.file_name().to_str().unwrap_or("");
                return !should_skip_dir(name);
            }
            true
        });

    for entry in walker.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            if let Some(lang) = Lang::from_path(entry.path()) {
                source_files.push((entry.path().to_path_buf(), lang));
                if last_emit.elapsed() >= Duration::from_millis(120) {
                    last_emit = Instant::now();
                    on_progress(AnalyzeProgress {
                        phase: "Scanning files...".to_string(),
                        processed: source_files.len(),
                        total: 0,
                        elapsed_ms: scan_start.elapsed().as_millis() as u64,
                        eta_ms: None,
                    });
                }
            }
        }
    }

    let cache_file = graph_cache_file(&root);
    let fingerprint = compute_source_fingerprint(&root, &source_files);
    if let Some(cached_graph) = load_cached_graph(&cache_file, fingerprint) {
        let total = source_files.len();
        on_progress(AnalyzeProgress {
            phase: "Loaded from cache".to_string(),
            processed: total,
            total,
            elapsed_ms: scan_start.elapsed().as_millis() as u64,
            eta_ms: Some(0),
        });
        return Ok(cached_graph);
    }

    let mut groups: Vec<GroupNode> = Vec::new();
    let mut group_ids: HashSet<String> = HashSet::new();
    let mut functions: Vec<FunctionNode> = Vec::new();
    let mut parse_errors: Vec<ParseError> = Vec::new();

    let ensure_dir_chain = |rel_dir: &Path,
                            groups: &mut Vec<GroupNode>,
                            group_ids: &mut HashSet<String>|
     -> String {
        let mut chain: Vec<PathBuf> = Vec::new();
        let mut cur = rel_dir;
        loop {
            chain.push(cur.to_path_buf());
            match cur.parent() {
                Some(p) if !cur.as_os_str().is_empty() && cur != Path::new(".") => {
                    cur = p;
                }
                _ => break,
            }
        }
        chain.reverse();

        let mut last_id = String::from("dir:.");
        for dir in chain {
            let rel_str = if dir.as_os_str().is_empty() || dir == Path::new(".") {
                ".".to_string()
            } else {
                to_posix(&dir)
            };
            let dir_id = format!("dir:{rel_str}");
            if !group_ids.contains(&dir_id) {
                let parent_id = if rel_str == "." {
                    None
                } else {
                    let p = dir.parent().unwrap_or(Path::new(""));
                    let p_str = if p.as_os_str().is_empty() || p == Path::new(".") {
                        ".".to_string()
                    } else {
                        to_posix(p)
                    };
                    Some(format!("dir:{p_str}"))
                };
                let name = if rel_str == "." {
                    root_name.clone()
                } else {
                    dir.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&rel_str)
                        .to_string()
                };
                let qualified = if rel_str == "." {
                    root_name.clone()
                } else {
                    rel_str.replace('/', ".")
                };
                let abs_dir = if rel_str == "." {
                    root.clone()
                } else {
                    root.join(&dir)
                };
                let is_pkg = abs_dir.join("__init__.py").is_file()
                    || abs_dir.join("mod.rs").is_file()
                    || abs_dir.join("index.ts").is_file()
                    || abs_dir.join("index.js").is_file();

                group_ids.insert(dir_id.clone());
                groups.push(GroupNode {
                    id: dir_id.clone(),
                    kind: "directory".to_string(),
                    name,
                    qualified_name: qualified,
                    parent_id,
                    file_path: if rel_str == "." { None } else { Some(rel_str) },
                    is_package: is_pkg,
                    docstring: None,
                    line_count: 0,
                });
            }
            last_id = dir_id;
        }
        last_id
    };

    ensure_dir_chain(Path::new("."), &mut groups, &mut group_ids);

    // Lookup indexes for import and call resolution
    let mut module_by_dotted: HashMap<String, String> = HashMap::new();
    let mut module_by_rel_no_ext: HashMap<String, String> = HashMap::new();
    let mut module_by_dir: HashMap<String, Vec<String>> = HashMap::new();
    let mut fn_by_module_name: HashMap<(String, String), String> = HashMap::new();
    let mut fn_by_class_method: HashMap<(String, String), String> = HashMap::new();
    let mut fn_by_global_name: HashMap<String, Vec<String>> = HashMap::new();

    let mut parsed_modules: Vec<ParsedModuleInfo> = Vec::new();
    let mut pending_calls: Vec<PendingCall> = Vec::new();
    let mut parsers: HashMap<u8, Parser> = HashMap::new();

    let total_files = source_files.len();
    let parse_start = Instant::now();
    let mut last_parse_emit = parse_start;
    on_progress(AnalyzeProgress {
        phase: "Parsing files".to_string(),
        processed: 0,
        total: total_files,
        elapsed_ms: 0,
        eta_ms: None,
    });

    for (idx, (file_path, lang)) in source_files.into_iter().enumerate() {
        let processed = idx + 1;
        if processed == total_files || last_parse_emit.elapsed() >= Duration::from_millis(120) {
            last_parse_emit = Instant::now();
            let elapsed_ms = parse_start.elapsed().as_millis() as u64;
            let eta_ms = if processed >= 10 && elapsed_ms >= 200 && total_files > processed {
                Some((elapsed_ms as f64 * (total_files - processed) as f64 / processed as f64) as u64)
            } else {
                None
            };
            on_progress(AnalyzeProgress {
                phase: "Parsing files".to_string(),
                processed,
                total: total_files,
                elapsed_ms,
                eta_ms,
            });
        }
        let rel_path = match file_path.strip_prefix(&root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let rel_posix = to_posix(&rel_path);
        let parent_dir = rel_path.parent().unwrap_or(Path::new(""));
        let parent_dir_id = ensure_dir_chain(parent_dir, &mut groups, &mut group_ids);

        let src = match fs::read_to_string(&file_path) {
            Ok(s) => {
                if lang == Lang::Text && s.contains('\0') {
                    continue;
                }
                s
            }
            Err(e) => {
                if lang != Lang::Text {
                    parse_errors.push(ParseError {
                        file_path: rel_posix,
                        error: e.to_string(),
                    });
                }
                continue;
            }
        };

        let file_name = rel_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&rel_posix)
            .to_string();
        let rel_no_ext = to_posix(&rel_path.with_extension(""));
        let is_init = file_name == "__init__.py" || file_name == "mod.rs" || file_name.starts_with("index.");
        let dotted_name = if is_init {
            let p = to_posix(parent_dir);
            if p.is_empty() || p == "." {
                root_name.clone()
            } else {
                p.replace('/', ".")
            }
        } else {
            rel_no_ext.replace('/', ".")
        };

        let module_id = format!("file:{rel_posix}");
        let line_count = src.lines().count().max(1);

        let Some(ts_lang) = lang.ts_language() else {
            groups.push(GroupNode {
                id: module_id,
                kind: "module".to_string(),
                name: file_name,
                qualified_name: dotted_name,
                parent_id: Some(parent_dir_id),
                file_path: Some(rel_posix),
                is_package: false,
                docstring: None,
                line_count,
            });
            continue;
        };

        let lang_key = lang as u8;
        let parser = parsers.entry(lang_key).or_insert_with(|| {
            let mut p = Parser::new();
            let _ = p.set_language(&ts_lang);
            p
        });

        let tree = match parser.parse(&src, None) {
            Some(t) => t,
            None => {
                parse_errors.push(ParseError {
                    file_path: rel_posix,
                    error: "Tree-sitter failed to parse file".to_string(),
                });
                continue;
            }
        };

        module_by_dotted.insert(dotted_name.clone(), module_id.clone());
        module_by_dotted.insert(rel_no_ext.replace('/', "."), module_id.clone());
        module_by_rel_no_ext.insert(rel_no_ext.clone(), module_id.clone());
        module_by_rel_no_ext.insert(rel_posix.clone(), module_id.clone());

        let dir_posix = {
            let d = to_posix(parent_dir);
            if d.is_empty() { ".".to_string() } else { d }
        };
        module_by_dir.entry(dir_posix).or_default().push(module_id.clone());

        let root_node = tree.root_node();

        let mod_doc = extract_module_docstring(root_node, &src, lang);
        groups.push(GroupNode {
            id: module_id.clone(),
            kind: "module".to_string(),
            name: file_name,
            qualified_name: dotted_name.clone(),
            parent_id: Some(parent_dir_id),
            file_path: Some(rel_posix.clone()),
            is_package: is_init,
            docstring: mod_doc,
            line_count,
        });

        let mut mod_info = ParsedModuleInfo {
            module_id: module_id.clone(),
            rel_posix: rel_posix.clone(),
            lang,
            raw_imports: Vec::new(),
            symbol_imports: HashMap::new(),
            module_aliases: HashMap::new(),
        };

        let mut ctx = ExtractContext {
            src: &src,
            lang,
            rel_posix: &rel_posix,
            dotted_name: &dotted_name,
            module_id: &module_id,
            groups: &mut groups,
            group_ids: &mut group_ids,
            functions: &mut functions,
            fn_by_module_name: &mut fn_by_module_name,
            fn_by_class_method: &mut fn_by_class_method,
            fn_by_global_name: &mut fn_by_global_name,
            pending_calls: &mut pending_calls,
            mod_info: &mut mod_info,
        };

        extract_module(root_node, &mut ctx);
        parsed_modules.push(mod_info);
    }

    // Resolve Import & Call Edges
    let edge_start = Instant::now();
    let mut last_edge_emit = edge_start;
    on_progress(AnalyzeProgress {
        phase: "Resolving edges...".to_string(),
        processed: total_files,
        total: total_files,
        elapsed_ms: parse_start.elapsed().as_millis() as u64,
        eta_ms: None,
    });

    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_imports: HashSet<(String, String)> = HashSet::new();
    let mut seen_calls: HashSet<(String, String)> = HashSet::new();

    let mut resolved_mod_lookup: HashMap<(String, String), String> = HashMap::new();

    for m in &parsed_modules {
        for raw in &m.raw_imports {
            if let Some(target_id) = resolve_import_target(
                &m.rel_posix,
                raw,
                m.lang,
                &module_by_dotted,
                &module_by_rel_no_ext,
                &module_by_dir,
            ) {
                resolved_mod_lookup.insert((m.module_id.clone(), raw.clone()), target_id.clone());
                if target_id != m.module_id && seen_imports.insert((m.module_id.clone(), target_id.clone())) {
                    edges.push(GraphEdge {
                        id: format!("imp:{}->{}", m.module_id, target_id),
                        kind: "import".to_string(),
                        source_id: m.module_id.clone(),
                        target_id,
                        label: Some("imports".to_string()),
                    });
                }
            }
        }
    }

    let mod_map: HashMap<&str, &ParsedModuleInfo> = parsed_modules
        .iter()
        .map(|m| (m.module_id.as_str(), m))
        .collect();

    let mut out_call_count: HashMap<String, usize> = HashMap::new();
    let mut in_call_count: HashMap<String, usize> = HashMap::new();
    const MAX_CALL_EDGES_PER_FN: usize = 100;

    let total_calls = pending_calls.len();
    for (call_idx, call) in pending_calls.into_iter().enumerate() {
        let done_calls = call_idx + 1;
        if done_calls == total_calls || last_edge_emit.elapsed() >= Duration::from_millis(120) {
            last_edge_emit = Instant::now();
            let elapsed_ms = edge_start.elapsed().as_millis() as u64;
            let eta_ms = if done_calls >= 100 && elapsed_ms >= 200 && total_calls > done_calls {
                Some((elapsed_ms as f64 * (total_calls - done_calls) as f64 / done_calls as f64) as u64)
            } else {
                None
            };
            on_progress(AnalyzeProgress {
                phase: "Resolving edges".to_string(),
                processed: done_calls,
                total: total_calls,
                elapsed_ms,
                eta_ms,
            });
        }
        let mut target_fn_id: Option<String> = None;

        if let Some(recv) = &call.receiver {
            if (recv == "self" || recv == "cls" || recv == "this") && call.class_id.is_some() {
                let cid = call.class_id.as_ref().unwrap();
                target_fn_id = fn_by_class_method.get(&(cid.clone(), call.callee.clone())).cloned();
            } else if let Some(m) = mod_map.get(call.module_id.as_str()) {
                if let Some(hint) = m.module_aliases.get(recv) {
                    if let Some(target_mod_id) = resolved_mod_lookup.get(&(m.module_id.clone(), hint.clone())) {
                        target_fn_id = fn_by_module_name
                            .get(&(target_mod_id.clone(), call.callee.clone()))
                            .cloned();
                    }
                }
            }
            if target_fn_id.is_none() {
                if let Some(candidates) = fn_by_global_name.get(&call.callee) {
                    if candidates.len() == 1 {
                        target_fn_id = Some(candidates[0].clone());
                    }
                }
            }
        } else {
            if let Some(m) = mod_map.get(call.module_id.as_str()) {
                if let Some((hint, sym)) = m.symbol_imports.get(&call.callee) {
                    if let Some(target_mod_id) = resolved_mod_lookup.get(&(m.module_id.clone(), hint.clone())) {
                        target_fn_id = fn_by_module_name
                            .get(&(target_mod_id.clone(), sym.clone()))
                            .cloned();
                    }
                }
            }
            if target_fn_id.is_none() {
                target_fn_id = fn_by_module_name
                    .get(&(call.module_id.clone(), call.callee.clone()))
                    .cloned();
            }
            if target_fn_id.is_none() {
                if let Some(cid) = &call.class_id {
                    target_fn_id = fn_by_class_method.get(&(cid.clone(), call.callee.clone())).cloned();
                }
            }
            if target_fn_id.is_none() {
                if let Some(candidates) = fn_by_global_name.get(&call.callee) {
                    if candidates.len() == 1 {
                        target_fn_id = Some(candidates[0].clone());
                    }
                }
            }
        }

        if let Some(target_id) = target_fn_id {
            if target_id != call.fn_id && seen_calls.insert((call.fn_id.clone(), target_id.clone())) {
                let out_n = out_call_count.get(&call.fn_id).copied().unwrap_or(0);
                let in_n = in_call_count.get(&target_id).copied().unwrap_or(0);
                if out_n < MAX_CALL_EDGES_PER_FN && in_n < MAX_CALL_EDGES_PER_FN {
                    out_call_count.insert(call.fn_id.clone(), out_n + 1);
                    in_call_count.insert(target_id.clone(), in_n + 1);
                    edges.push(GraphEdge {
                        id: format!("call:{}->{}", call.fn_id, target_id),
                        kind: "call".to_string(),
                        source_id: call.fn_id,
                        target_id,
                        label: Some(call.callee),
                    });
                }
            }
        }
    }

    let total_files = groups.iter().filter(|g| g.kind == "module").count();
    let total_classes = groups.iter().filter(|g| g.kind == "class").count();
    let total_functions = functions.len();

    let graph = ProjectGraph {
        root_name,
        root_path: to_posix(&root),
        groups,
        functions,
        edges,
        parse_errors,
        stats: ProjectStats {
            total_files,
            total_classes,
            total_functions,
            total_import_edges: seen_imports.len(),
            total_call_edges: seen_calls.len(),
        },
    };
    save_cached_graph(&cache_file, fingerprint, &graph);
    Ok(graph)
}

struct ExtractContext<'a> {
    src: &'a str,
    lang: Lang,
    rel_posix: &'a str,
    dotted_name: &'a str,
    module_id: &'a str,
    groups: &'a mut Vec<GroupNode>,
    group_ids: &'a mut HashSet<String>,
    functions: &'a mut Vec<FunctionNode>,
    fn_by_module_name: &'a mut HashMap<(String, String), String>,
    fn_by_class_method: &'a mut HashMap<(String, String), String>,
    fn_by_global_name: &'a mut HashMap<String, Vec<String>>,
    pending_calls: &'a mut Vec<PendingCall>,
    mod_info: &'a mut ParsedModuleInfo,
}

fn extract_module_docstring(root: Node, src: &str, lang: Lang) -> Option<String> {
    if lang == Lang::Python {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if child.kind() == "expression_statement" {
                if let Some(str_node) = child.named_child(0) {
                    if str_node.kind() == "string" {
                        return clean_docstring(node_text(str_node, src));
                    }
                }
            }
            if child.kind() != "comment" {
                break;
            }
        }
    }
    None
}

fn clean_docstring(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_start_matches("\"\"\"")
        .trim_end_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("'''")
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
    let first = trimmed.lines().next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.chars().take(120).collect())
    }
}

fn ensure_class_group(
    ctx: &mut ExtractContext,
    class_name: &str,
    start_line: usize,
    end_line: usize,
    docstring: Option<String>,
) -> String {
    let class_id = format!("class:{}::{}", ctx.rel_posix, class_name);
    if !ctx.group_ids.contains(&class_id) {
        ctx.group_ids.insert(class_id.clone());
        ctx.groups.push(GroupNode {
            id: class_id.clone(),
            kind: "class".to_string(),
            name: class_name.to_string(),
            qualified_name: format!("{}.{}", ctx.dotted_name, class_name),
            parent_id: Some(ctx.module_id.to_string()),
            file_path: Some(ctx.rel_posix.to_string()),
            is_package: false,
            docstring,
            line_count: (end_line.saturating_sub(start_line) + 1).max(1),
        });
    }
    class_id
}

fn estimate_complexity(node: Node, src: &str) -> usize {
    fn walk(n: Node, src: &str, count: &mut usize) {
        match n.kind() {
            "if_statement"
            | "if_expression"
            | "elif_clause"
            | "elsif"
            | "for_statement"
            | "for_in_statement"
            | "for_expression"
            | "enhanced_for_statement"
            | "foreach_statement"
            | "while_statement"
            | "while_expression"
            | "do_statement"
            | "match_arm"
            | "switch_case"
            | "case_clause"
            | "when"
            | "catch_clause"
            | "rescue"
            | "conditional_expression"
            | "ternary_expression" => {
                *count += 1;
            }
            "binary_expression" => {
                let txt = node_text(n, src);
                if txt.contains("&&") || txt.contains("||") {
                    *count += 1;
                }
            }
            _ => {}
        }
        let mut cursor = n.walk();
        for child in n.named_children(&mut cursor) {
            walk(child, src, count);
        }
    }
    let mut count = 1usize;
    walk(node, src, &mut count);
    count
}

fn register_function(
    ctx: &mut ExtractContext,
    name: String,
    parent_id: String,
    class_id: Option<String>,
    prefix: &str,
    is_async: bool,
    decorators: Vec<String>,
    signature: String,
    docstring: Option<String>,
    start_line: usize,
    end_line: usize,
    body_node: Option<Node>,
) {
    let base_id = format!("fn:{}::{}{}", ctx.rel_posix, prefix, name);
    let fn_id = if ctx.functions.iter().any(|f| f.id == base_id) {
        format!("{base_id}@{start_line}")
    } else {
        base_id
    };

    let code = slice_lines(ctx.src, start_line, end_line);
    let complexity = body_node
        .map(|b| estimate_complexity(b, ctx.src))
        .unwrap_or(1);
    ctx.functions.push(FunctionNode {
        id: fn_id.clone(),
        name: name.clone(),
        qualified_name: format!("{}.{}{}", ctx.dotted_name, prefix, name),
        parent_id,
        file_path: ctx.rel_posix.to_string(),
        is_async,
        is_method: class_id.is_some(),
        decorators,
        signature,
        docstring,
        start_line,
        end_line,
        code,
        complexity,
    });

    if let Some(cid) = &class_id {
        ctx.fn_by_class_method
            .insert((cid.clone(), name.clone()), fn_id.clone());
    } else {
        ctx.fn_by_module_name
            .insert((ctx.module_id.to_string(), name.clone()), fn_id.clone());
    }
    ctx.fn_by_global_name
        .entry(name)
        .or_default()
        .push(fn_id.clone());

    if let Some(body) = body_node {
        collect_calls(body, ctx, &fn_id, class_id.as_deref());
    }
}

fn extract_module(root: Node, ctx: &mut ExtractContext) {
    match ctx.lang {
        Lang::Python => extract_python(root, ctx),
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => extract_ts_js(root, ctx),
        Lang::Rust => extract_rust(root, ctx),
        Lang::Go => extract_go(root, ctx),
        Lang::Java => extract_java(root, ctx),
        Lang::C | Lang::Cpp => extract_c_cpp(root, ctx),
        Lang::CSharp => extract_csharp(root, ctx),
        Lang::Ruby => extract_ruby(root, ctx),
        Lang::Text => {}
    }
}

// ---------------- PYTHON ----------------
fn extract_python(root: Node, ctx: &mut ExtractContext) {
    let mut cursor = root.walk();
    for stmt in root.named_children(&mut cursor) {
        match stmt.kind() {
            "import_statement" => {
                let mut c = stmt.walk();
                for child in stmt.named_children(&mut c) {
                    if child.kind() == "dotted_name" {
                        let name = node_text(child, ctx.src).to_string();
                        let alias = name.split('.').next().unwrap_or(&name).to_string();
                        ctx.mod_info.module_aliases.insert(alias, name.clone());
                        ctx.mod_info.raw_imports.push(name);
                    } else if child.kind() == "aliased_import" {
                        if let Some(n) = child.child_by_field_name("name") {
                            let mod_name = node_text(n, ctx.src).to_string();
                            let alias = child
                                .child_by_field_name("alias")
                                .map(|a| node_text(a, ctx.src).to_string())
                                .unwrap_or_else(|| mod_name.split('.').next().unwrap_or(&mod_name).to_string());
                            ctx.mod_info.module_aliases.insert(alias, mod_name.clone());
                            ctx.mod_info.raw_imports.push(mod_name);
                        }
                    }
                }
            }
            "import_from_statement" => {
                let raw_mod = stmt
                    .child_by_field_name("module_name")
                    .map(|n| node_text(n, ctx.src).to_string())
                    .unwrap_or_default();
                if !raw_mod.is_empty() {
                    ctx.mod_info.raw_imports.push(raw_mod.clone());
                    let mut c = stmt.walk();
                    for child in stmt.named_children(&mut c) {
                        if child.kind() == "dotted_name" && Some(child) != stmt.child_by_field_name("module_name") {
                            let sym = node_text(child, ctx.src).to_string();
                            ctx.mod_info
                                .symbol_imports
                                .insert(sym.clone(), (raw_mod.clone(), sym));
                        } else if child.kind() == "aliased_import" {
                            if let Some(n) = child.child_by_field_name("name") {
                                let sym = node_text(n, ctx.src).to_string();
                                let local = child
                                    .child_by_field_name("alias")
                                    .map(|a| node_text(a, ctx.src).to_string())
                                    .unwrap_or_else(|| sym.clone());
                                ctx.mod_info
                                    .symbol_imports
                                    .insert(local, (raw_mod.clone(), sym));
                            }
                        }
                    }
                }
            }
            "function_definition" => extract_py_fn(stmt, ctx, ctx.module_id, None, "", Vec::new(), None),
            "class_definition" => extract_py_class(stmt, ctx, None),
            "decorated_definition" => {
                let mut decorators = Vec::new();
                let mut c = stmt.walk();
                let mut inner = None;
                for child in stmt.named_children(&mut c) {
                    if child.kind() == "decorator" {
                        decorators.push(node_text(child, ctx.src).trim().to_string());
                    } else {
                        inner = Some(child);
                    }
                }
                let start_override = Some(stmt.start_position().row + 1);
                if let Some(def) = inner {
                    if def.kind() == "function_definition" {
                        extract_py_fn(def, ctx, ctx.module_id, None, "", decorators, start_override);
                    } else if def.kind() == "class_definition" {
                        extract_py_class(def, ctx, start_override);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_py_class(node: Node, ctx: &mut ExtractContext, start_override: Option<usize>) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let class_name = node_text(name_node, ctx.src).to_string();
    let start_line = start_override.unwrap_or(node.start_position().row + 1);
    let end_line = node.end_position().row + 1;
    let body = node.child_by_field_name("body");
    let doc = body.and_then(|b| extract_module_docstring(b, ctx.src, Lang::Python));
    let class_id = ensure_class_group(ctx, &class_name, start_line, end_line, doc);
    let prefix = format!("{class_name}.");

    if let Some(body_node) = body {
        let mut cursor = body_node.walk();
        for member in body_node.named_children(&mut cursor) {
            if member.kind() == "function_definition" {
                extract_py_fn(member, ctx, &class_id, Some(class_id.clone()), &prefix, Vec::new(), None);
            } else if member.kind() == "decorated_definition" {
                let mut decorators = Vec::new();
                let mut c = member.walk();
                let mut inner = None;
                for child in member.named_children(&mut c) {
                    if child.kind() == "decorator" {
                        decorators.push(node_text(child, ctx.src).trim().to_string());
                    } else {
                        inner = Some(child);
                    }
                }
                let m_start = Some(member.start_position().row + 1);
                if let Some(def) = inner {
                    if def.kind() == "function_definition" {
                        extract_py_fn(def, ctx, &class_id, Some(class_id.clone()), &prefix, decorators, m_start);
                    }
                }
            }
        }
    }
}

fn extract_py_fn(
    node: Node,
    ctx: &mut ExtractContext,
    parent_id: &str,
    class_id: Option<String>,
    prefix: &str,
    decorators: Vec<String>,
    start_override: Option<usize>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.src).to_string();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, ctx.src).to_string())
        .unwrap_or_else(|| "()".to_string());
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(" -> {}", node_text(r, ctx.src)))
        .unwrap_or_default();
    let signature = format!("{params}{ret}");
    let is_async = node_text(node, ctx.src).trim_start().starts_with("async ");
    let start_line = start_override.unwrap_or(node.start_position().row + 1);
    let end_line = node.end_position().row + 1;
    let body = node.child_by_field_name("body");
    let doc = body.and_then(|b| extract_module_docstring(b, ctx.src, Lang::Python));

    register_function(
        ctx,
        name,
        parent_id.to_string(),
        class_id,
        prefix,
        is_async,
        decorators,
        signature,
        doc,
        start_line,
        end_line,
        body,
    );
}

// ---------------- TYPESCRIPT / JAVASCRIPT ----------------
fn extract_ts_js(root: Node, ctx: &mut ExtractContext) {
    let mut cursor = root.walk();
    for stmt in root.named_children(&mut cursor) {
        let actual = if stmt.kind() == "export_statement" {
            stmt.named_child(0).unwrap_or(stmt)
        } else {
            stmt
        };
        let start_override = Some(stmt.start_position().row + 1);

        match actual.kind() {
            "import_statement" => {
                if let Some(source) = actual.child_by_field_name("source") {
                    let raw = node_text(source, ctx.src).trim_matches(['"', '\'']).to_string();
                    if !raw.is_empty() {
                        ctx.mod_info.raw_imports.push(raw);
                    }
                }
            }
            "function_declaration" | "generator_function_declaration" => {
                extract_ts_fn(actual, ctx, ctx.module_id, None, "", None, start_override);
            }
            "class_declaration" => {
                extract_ts_class(actual, ctx, start_override);
            }
            "lexical_declaration" | "variable_declaration" => {
                let mut c = actual.walk();
                for decl in actual.named_children(&mut c) {
                    if decl.kind() == "variable_declarator" {
                        let name_node = decl.child_by_field_name("name");
                        let val_node = decl.child_by_field_name("value");
                        if let (Some(nn), Some(vn)) = (name_node, val_node) {
                            if matches!(vn.kind(), "arrow_function" | "function_expression" | "function") {
                                let fn_name = node_text(nn, ctx.src).to_string();
                                extract_ts_fn(
                                    vn,
                                    ctx,
                                    ctx.module_id,
                                    None,
                                    "",
                                    Some(fn_name),
                                    start_override,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_ts_class(node: Node, ctx: &mut ExtractContext, start_override: Option<usize>) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let class_name = node_text(name_node, ctx.src).to_string();
    let start_line = start_override.unwrap_or(node.start_position().row + 1);
    let end_line = node.end_position().row + 1;
    let class_id = ensure_class_group(ctx, &class_name, start_line, end_line, None);
    let prefix = format!("{class_name}.");

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.named_children(&mut cursor) {
            if member.kind() == "method_definition" {
                extract_ts_fn(member, ctx, &class_id, Some(class_id.clone()), &prefix, None, None);
            }
        }
    }
}

fn extract_ts_fn(
    node: Node,
    ctx: &mut ExtractContext,
    parent_id: &str,
    class_id: Option<String>,
    prefix: &str,
    name_override: Option<String>,
    start_override: Option<usize>,
) {
    let name = name_override.or_else(|| {
        node.child_by_field_name("name")
            .map(|n| node_text(n, ctx.src).to_string())
    });
    let Some(name) = name else { return };
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, ctx.src).to_string())
        .unwrap_or_else(|| "()".to_string());
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| node_text(r, ctx.src).to_string())
        .unwrap_or_default();
    let signature = format!("{params}{ret}");
    let is_async = node_text(node, ctx.src).trim_start().starts_with("async ");
    let start_line = start_override.unwrap_or(node.start_position().row + 1);
    let end_line = node.end_position().row + 1;
    let body = node.child_by_field_name("body");

    register_function(
        ctx,
        name,
        parent_id.to_string(),
        class_id,
        prefix,
        is_async,
        Vec::new(),
        signature,
        None,
        start_line,
        end_line,
        body,
    );
}

// ---------------- RUST ----------------
fn extract_rust(root: Node, ctx: &mut ExtractContext) {
    let mut cursor = root.walk();
    for item in root.named_children(&mut cursor) {
        match item.kind() {
            "use_declaration" => {
                if let Some(arg) = item.child_by_field_name("argument") {
                    let raw = node_text(arg, ctx.src).to_string();
                    ctx.mod_info.raw_imports.push(raw);
                }
            }
            "function_item" => {
                extract_rs_fn(item, ctx, ctx.module_id, None, "");
            }
            "struct_item" | "enum_item" | "trait_item" => {
                if let Some(name_node) = item.child_by_field_name("name") {
                    let cname = node_text(name_node, ctx.src).to_string();
                    ensure_class_group(
                        ctx,
                        &cname,
                        item.start_position().row + 1,
                        item.end_position().row + 1,
                        None,
                    );
                }
            }
            "impl_item" => {
                if let Some(type_node) = item.child_by_field_name("type") {
                    let raw_type = node_text(type_node, ctx.src);
                    let cname = raw_type
                        .split('<')
                        .next()
                        .unwrap_or(raw_type)
                        .trim()
                        .to_string();
                    let class_id = ensure_class_group(
                        ctx,
                        &cname,
                        item.start_position().row + 1,
                        item.end_position().row + 1,
                        None,
                    );
                    let prefix = format!("{cname}.");
                    if let Some(body) = item.child_by_field_name("body") {
                        let mut c = body.walk();
                        for member in body.named_children(&mut c) {
                            if member.kind() == "function_item" {
                                extract_rs_fn(member, ctx, &class_id, Some(class_id.clone()), &prefix);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_rs_fn(
    node: Node,
    ctx: &mut ExtractContext,
    parent_id: &str,
    class_id: Option<String>,
    prefix: &str,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.src).to_string();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, ctx.src).to_string())
        .unwrap_or_else(|| "()".to_string());
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(" -> {}", node_text(r, ctx.src)))
        .unwrap_or_default();
    let signature = format!("{params}{ret}");
    let is_async = node_text(node, ctx.src).contains("async fn ");
    let start_line = node.start_position().row + 1;
    let end_line = node.end_position().row + 1;
    let body = node.child_by_field_name("body");

    register_function(
        ctx,
        name,
        parent_id.to_string(),
        class_id,
        prefix,
        is_async,
        Vec::new(),
        signature,
        None,
        start_line,
        end_line,
        body,
    );
}

// ---------------- GO ----------------
fn extract_go(root: Node, ctx: &mut ExtractContext) {
    let mut cursor = root.walk();
    for decl in root.named_children(&mut cursor) {
        match decl.kind() {
            "import_declaration" => {
                collect_go_imports(decl, ctx);
            }
            "type_declaration" => {
                let mut c = decl.walk();
                for spec in decl.named_children(&mut c) {
                    if spec.kind() == "type_spec" {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            let cname = node_text(name_node, ctx.src).to_string();
                            ensure_class_group(
                                ctx,
                                &cname,
                                spec.start_position().row + 1,
                                spec.end_position().row + 1,
                                None,
                            );
                        }
                    }
                }
            }
            "function_declaration" => {
                extract_go_fn(decl, ctx, ctx.module_id, None, "");
            }
            "method_declaration" => {
                let recv_type = decl
                    .child_by_field_name("receiver")
                    .and_then(|r| find_first_type_ident(r, ctx.src));
                if let Some(cname) = recv_type {
                    let class_id = ensure_class_group(
                        ctx,
                        &cname,
                        decl.start_position().row + 1,
                        decl.end_position().row + 1,
                        None,
                    );
                    let prefix = format!("{cname}.");
                    extract_go_fn(decl, ctx, &class_id, Some(class_id.clone()), &prefix);
                } else {
                    extract_go_fn(decl, ctx, ctx.module_id, None, "");
                }
            }
            _ => {}
        }
    }
}

fn collect_go_imports(node: Node, ctx: &mut ExtractContext) {
    if node.kind() == "import_spec" {
        if let Some(path_node) = node.child_by_field_name("path") {
            let raw = node_text(path_node, ctx.src).trim_matches('"').to_string();
            if !raw.is_empty() {
                ctx.mod_info.raw_imports.push(raw);
            }
        }
        return;
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        collect_go_imports(child, ctx);
    }
}

fn find_first_type_ident(node: Node, src: &str) -> Option<String> {
    if node.kind() == "type_identifier" {
        return Some(node_text(node, src).to_string());
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        if let Some(found) = find_first_type_ident(child, src) {
            return Some(found);
        }
    }
    None
}

fn extract_go_fn(
    node: Node,
    ctx: &mut ExtractContext,
    parent_id: &str,
    class_id: Option<String>,
    prefix: &str,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.src).to_string();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, ctx.src).to_string())
        .unwrap_or_else(|| "()".to_string());
    let ret = node
        .child_by_field_name("result")
        .map(|r| format!(" {}", node_text(r, ctx.src)))
        .unwrap_or_default();
    let signature = format!("{params}{ret}");
    let start_line = node.start_position().row + 1;
    let end_line = node.end_position().row + 1;
    let body = node.child_by_field_name("body");

    register_function(
        ctx,
        name,
        parent_id.to_string(),
        class_id,
        prefix,
        false,
        Vec::new(),
        signature,
        None,
        start_line,
        end_line,
        body,
    );
}

// ---------------- CALL EXTRACTION ----------------
// ---------------- JAVA / C / C++ / C# / RUBY ----------------
fn extract_java(root: Node, ctx: &mut ExtractContext) {
    fn walk(node: Node, ctx: &mut ExtractContext, class_id: Option<&str>, prefix: &str) {
        match node.kind() {
            "import_declaration" => {
                let raw = node_text(node, ctx.src)
                    .trim_start_matches("import")
                    .trim_start_matches("static")
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                if !raw.is_empty() {
                    ctx.mod_info.raw_imports.push(raw.to_string());
                }
            }
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let cname = node_text(name_node, ctx.src).to_string();
                    let cid = ensure_class_group(
                        ctx,
                        &cname,
                        node.start_position().row + 1,
                        node.end_position().row + 1,
                        None,
                    );
                    let mut c = node.walk();
                    for child in node.named_children(&mut c) {
                        if child.id() != name_node.id() {
                            walk(child, ctx, Some(&cid), &format!("{cname}."));
                        }
                    }
                    return;
                }
            }
            "method_declaration" | "constructor_declaration" => {
                let mut c = node.walk();
                let name_node = node.child_by_field_name("name").or_else(|| {
                    node.named_children(&mut c)
                        .find(|ch| ch.kind() == "identifier")
                });
                if let Some(name_node) = name_node {
                    let name = node_text(name_node, ctx.src).to_string();
                    let params = node
                        .child_by_field_name("parameters")
                        .map(|p| node_text(p, ctx.src))
                        .unwrap_or("()");
                    let start = node.start_position().row + 1;
                    let end = node.end_position().row + 1;
                    let parent = class_id.unwrap_or(ctx.module_id).to_string();
                    register_function(
                        ctx,
                        name,
                        parent,
                        class_id.map(str::to_string),
                        prefix,
                        false,
                        Vec::new(),
                        params.to_string(),
                        None,
                        start,
                        end,
                        node.child_by_field_name("body"),
                    );
                    return;
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, ctx, class_id, prefix);
        }
    }
    walk(root, ctx, None, "");
}

fn extract_c_cpp(root: Node, ctx: &mut ExtractContext) {
    fn find_fn_name(decl: Node, src: &str) -> Option<String> {
        if decl.kind() == "identifier" || decl.kind() == "field_identifier" {
            return Some(node_text(decl, src).to_string());
        }
        if let Some(inner) = decl.child_by_field_name("declarator") {
            return find_fn_name(inner, src);
        }
        let mut cursor = decl.walk();
        for ch in decl.named_children(&mut cursor) {
            if let Some(n) = find_fn_name(ch, src) {
                return Some(n);
            }
        }
        None
    }

    fn walk(node: Node, ctx: &mut ExtractContext, class_id: Option<&str>, prefix: &str) {
        match node.kind() {
            "preproc_include" => {
                if let Some(path_node) = node.child_by_field_name("path") {
                    let raw = node_text(path_node, ctx.src)
                        .trim_matches(|c| c == '"' || c == '<' || c == '>');
                    if !raw.is_empty() {
                        ctx.mod_info.raw_imports.push(raw.to_string());
                    }
                }
            }
            "class_specifier" | "struct_specifier" => {
                if let (Some(name_node), Some(body)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("body"),
                ) {
                    let cname = node_text(name_node, ctx.src).to_string();
                    let cid = ensure_class_group(
                        ctx,
                        &cname,
                        node.start_position().row + 1,
                        node.end_position().row + 1,
                        None,
                    );
                    let mut c = body.walk();
                    for child in body.named_children(&mut c) {
                        walk(child, ctx, Some(&cid), &format!("{cname}::"));
                    }
                    return;
                }
            }
            "function_definition" => {
                if let Some(decl) = node.child_by_field_name("declarator") {
                    if let Some(name) = find_fn_name(decl, ctx.src) {
                        let start = node.start_position().row + 1;
                        let end = node.end_position().row + 1;
                        let parent = class_id.unwrap_or(ctx.module_id).to_string();
                        register_function(
                            ctx,
                            name,
                            parent,
                            class_id.map(str::to_string),
                            prefix,
                            false,
                            Vec::new(),
                            "()".to_string(),
                            None,
                            start,
                            end,
                            node.child_by_field_name("body"),
                        );
                        return;
                    }
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, ctx, class_id, prefix);
        }
    }
    walk(root, ctx, None, "");
}

fn extract_csharp(root: Node, ctx: &mut ExtractContext) {
    fn walk(node: Node, ctx: &mut ExtractContext, class_id: Option<&str>, prefix: &str) {
        match node.kind() {
            "using_directive" => {
                let raw = node_text(node, ctx.src)
                    .trim_start_matches("using")
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                if !raw.is_empty() {
                    ctx.mod_info.raw_imports.push(raw.to_string());
                }
            }
            "class_declaration" | "interface_declaration" | "struct_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let cname = node_text(name_node, ctx.src).to_string();
                    let cid = ensure_class_group(
                        ctx,
                        &cname,
                        node.start_position().row + 1,
                        node.end_position().row + 1,
                        None,
                    );
                    let mut c = node.walk();
                    for child in node.named_children(&mut c) {
                        if child.id() != name_node.id() {
                            walk(child, ctx, Some(&cid), &format!("{cname}."));
                        }
                    }
                    return;
                }
            }
            "method_declaration" | "constructor_declaration" => {
                let mut c = node.walk();
                let name_node = node.child_by_field_name("name").or_else(|| {
                    node.named_children(&mut c)
                        .find(|ch| ch.kind() == "identifier")
                });
                if let Some(name_node) = name_node {
                    let name = node_text(name_node, ctx.src).to_string();
                    let params = node
                        .child_by_field_name("parameters")
                        .map(|p| node_text(p, ctx.src))
                        .unwrap_or("()");
                    let is_async = node_text(node, ctx.src).contains("async ");
                    let start = node.start_position().row + 1;
                    let end = node.end_position().row + 1;
                    let parent = class_id.unwrap_or(ctx.module_id).to_string();
                    register_function(
                        ctx,
                        name,
                        parent,
                        class_id.map(str::to_string),
                        prefix,
                        is_async,
                        Vec::new(),
                        params.to_string(),
                        None,
                        start,
                        end,
                        node.child_by_field_name("body"),
                    );
                    return;
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, ctx, class_id, prefix);
        }
    }
    walk(root, ctx, None, "");
}

fn extract_ruby(root: Node, ctx: &mut ExtractContext) {
    fn walk(node: Node, ctx: &mut ExtractContext, class_id: Option<&str>, prefix: &str) {
        match node.kind() {
            "call" => {
                if let Some(method_node) = node.child_by_field_name("method") {
                    let m = node_text(method_node, ctx.src);
                    if m == "require" || m == "require_relative" {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            let raw = node_text(args, ctx.src)
                                .trim()
                                .trim_matches(|c| c == '(' || c == ')' || c == '\'' || c == '"');
                            if !raw.is_empty() {
                                ctx.mod_info.raw_imports.push(raw.to_string());
                            }
                        }
                    }
                }
            }
            "class" | "module" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let cname = node_text(name_node, ctx.src).to_string();
                    let cid = ensure_class_group(
                        ctx,
                        &cname,
                        node.start_position().row + 1,
                        node.end_position().row + 1,
                        None,
                    );
                    let mut c = node.walk();
                    for child in node.named_children(&mut c) {
                        if child.id() != name_node.id() {
                            walk(child, ctx, Some(&cid), &format!("{cname}."));
                        }
                    }
                    return;
                }
            }
            "method" | "singleton_method" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, ctx.src).to_string();
                    let params = node
                        .child_by_field_name("parameters")
                        .map(|p| node_text(p, ctx.src))
                        .unwrap_or("()");
                    let start = node.start_position().row + 1;
                    let end = node.end_position().row + 1;
                    let parent = class_id.unwrap_or(ctx.module_id).to_string();
                    register_function(
                        ctx,
                        name,
                        parent,
                        class_id.map(str::to_string),
                        prefix,
                        false,
                        Vec::new(),
                        params.to_string(),
                        None,
                        start,
                        end,
                        node.child_by_field_name("body"),
                    );
                    return;
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, ctx, class_id, prefix);
        }
    }
    walk(root, ctx, None, "");
}

fn collect_calls(node: Node, ctx: &mut ExtractContext, fn_id: &str, class_id: Option<&str>) {
    match node.kind() {
        "call" | "call_expression" => {
            let func_node = node.child_by_field_name("function");
            if let Some(fn_node) = func_node {
                match fn_node.kind() {
                    "identifier" => {
                        let callee = node_text(fn_node, ctx.src).to_string();
                        ctx.pending_calls.push(PendingCall {
                            fn_id: fn_id.to_string(),
                            module_id: ctx.module_id.to_string(),
                            class_id: class_id.map(str::to_string),
                            receiver: None,
                            callee,
                        });
                    }
                    "attribute" | "member_expression" | "selector_expression" | "field_expression" => {
                        let obj = fn_node
                            .child_by_field_name("object")
                            .or_else(|| fn_node.child_by_field_name("operand"))
                            .or_else(|| fn_node.child_by_field_name("value"));
                        let prop = fn_node
                            .child_by_field_name("attribute")
                            .or_else(|| fn_node.child_by_field_name("property"))
                            .or_else(|| fn_node.child_by_field_name("field"));
                        if let Some(p) = prop {
                            let callee = node_text(p, ctx.src).to_string();
                            let receiver = obj.map(|o| node_text(o, ctx.src).to_string());
                            ctx.pending_calls.push(PendingCall {
                                fn_id: fn_id.to_string(),
                                module_id: ctx.module_id.to_string(),
                                class_id: class_id.map(str::to_string),
                                receiver,
                                callee,
                            });
                        }
                    }
                    "scoped_identifier" => {
                        let txt = node_text(fn_node, ctx.src);
                        if let Some(last) = txt.rsplit("::").next() {
                            ctx.pending_calls.push(PendingCall {
                                fn_id: fn_id.to_string(),
                                module_id: ctx.module_id.to_string(),
                                class_id: class_id.map(str::to_string),
                                receiver: None,
                                callee: last.to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "method_invocation" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let receiver = node
                    .child_by_field_name("object")
                    .map(|r| node_text(r, ctx.src).to_string());
                ctx.pending_calls.push(PendingCall {
                    fn_id: fn_id.to_string(),
                    module_id: ctx.module_id.to_string(),
                    class_id: class_id.map(str::to_string),
                    receiver,
                    callee: node_text(name_node, ctx.src).to_string(),
                });
            }
        }
        "invocation_expression" => {
            if let Some(fn_node) = node.child_by_field_name("function") {
                let txt = node_text(fn_node, ctx.src);
                let callee = txt.rsplit('.').next().unwrap_or(txt).to_string();
                let receiver = txt.rsplit_once('.').map(|(r, _)| r.to_string());
                ctx.pending_calls.push(PendingCall {
                    fn_id: fn_id.to_string(),
                    module_id: ctx.module_id.to_string(),
                    class_id: class_id.map(str::to_string),
                    receiver,
                    callee,
                });
            }
        }
        "method_call_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let receiver = node
                    .child_by_field_name("receiver")
                    .map(|r| node_text(r, ctx.src).to_string());
                ctx.pending_calls.push(PendingCall {
                    fn_id: fn_id.to_string(),
                    module_id: ctx.module_id.to_string(),
                    class_id: class_id.map(str::to_string),
                    receiver,
                    callee: node_text(name_node, ctx.src).to_string(),
                });
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_calls(child, ctx, fn_id, class_id);
    }
}

// ---------------- IMPORT RESOLUTION ----------------
fn resolve_import_target(
    current_rel_posix: &str,
    raw_import: &str,
    lang: Lang,
    module_by_dotted: &HashMap<String, String>,
    module_by_rel_no_ext: &HashMap<String, String>,
    module_by_dir: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if let Some(found) = module_by_dotted.get(raw_import) {
        return Some(found.clone());
    }

    match lang {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            if raw_import.starts_with('.') {
                let cur_dir = Path::new(current_rel_posix).parent().unwrap_or(Path::new(""));
                let joined = normalize_rel(cur_dir.join(raw_import));
                if let Some(found) = module_by_rel_no_ext.get(&joined) {
                    return Some(found.clone());
                }
                let index_path = format!("{joined}/index");
                if let Some(found) = module_by_rel_no_ext.get(&index_path) {
                    return Some(found.clone());
                }
            }
        }
        Lang::Rust => {
            let cleaned = raw_import
                .trim_start_matches("crate::")
                .trim_start_matches("self::")
                .trim_start_matches("super::")
                .replace("::", ".");
            if let Some(found) = module_by_dotted.get(&cleaned) {
                return Some(found.clone());
            }
            if let Some(prefix) = cleaned.split('.').next() {
                if let Some(found) = module_by_dotted.get(prefix) {
                    return Some(found.clone());
                }
            }
        }
        Lang::Go => {
            for (dir, mods) in module_by_dir {
                if dir != "." && raw_import.ends_with(dir) {
                    return mods.first().cloned();
                }
            }
        }
        Lang::Python => {
            if raw_import.starts_with('.') {
                let dots = raw_import.chars().take_while(|&c| c == '.').count();
                let rest = raw_import[dots..].replace('.', "/");
                let mut dir = Path::new(current_rel_posix).parent().unwrap_or(Path::new(""));
                for _ in 1..dots {
                    dir = dir.parent().unwrap_or(Path::new(""));
                }
                let joined = if rest.is_empty() {
                    normalize_rel(dir.to_path_buf())
                } else {
                    normalize_rel(dir.join(&rest))
                };
                if let Some(found) = module_by_rel_no_ext.get(&joined) {
                    return Some(found.clone());
                }
                let init_path = format!("{joined}/__init__");
                if let Some(found) = module_by_rel_no_ext.get(&init_path) {
                    return Some(found.clone());
                }
            }
        }
        Lang::Java | Lang::CSharp => {
            if let Some(last) = raw_import.rsplit('.').next() {
                for (dotted, mod_id) in module_by_dotted {
                    if dotted.ends_with(&format!(".{last}")) || dotted == last {
                        return Some(mod_id.clone());
                    }
                }
            }
        }
        Lang::C | Lang::Cpp | Lang::Ruby => {
            let cur_dir = Path::new(current_rel_posix).parent().unwrap_or(Path::new(""));
            let joined = normalize_rel(cur_dir.join(raw_import));
            let no_ext = joined
                .rsplit_once('.')
                .map(|(a, _)| a)
                .unwrap_or(&joined);
            if let Some(found) = module_by_rel_no_ext.get(no_ext) {
                return Some(found.clone());
            }
            let raw_no_ext = raw_import.rsplit_once('.').map(|(a, _)| a).unwrap_or(raw_import);
            if let Some(found) = module_by_rel_no_ext.get(raw_no_ext) {
                return Some(found.clone());
            }
        }
        Lang::Text => {}
    }
    None
}

fn normalize_rel(path: PathBuf) -> String {
    let mut parts: Vec<String> = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(s) => {
                parts.push(s.to_string_lossy().to_string());
            }
            _ => {}
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_multi_language_project() {
        let tmp = std::env::temp_dir().join(format!("vcode_ts_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("py")).unwrap();
        fs::create_dir_all(tmp.join("web")).unwrap();
        fs::create_dir_all(tmp.join("rust_mod")).unwrap();
        fs::create_dir_all(tmp.join("go_mod")).unwrap();

        fs::write(
            tmp.join("py/app.py"),
            "class Engine:\n    def run(self) -> int:\n        return self.step()\n\n    def step(self) -> int:\n        return 1\n",
        )
        .unwrap();
        fs::write(
            tmp.join("web/utils.ts"),
            "export function helper(x: number): number {\n  return x + 1;\n}\n",
        )
        .unwrap();
        fs::write(
            tmp.join("web/main.ts"),
            "import { helper } from './utils';\nexport const start = (n: number) => {\n  return helper(n);\n};\n",
        )
        .unwrap();
        fs::write(
            tmp.join("rust_mod/lib.rs"),
            "struct Worker;\nimpl Worker {\n    fn work(&self) { self.done(); }\n    fn done(&self) {}\n}\n",
        )
        .unwrap();
        fs::write(
            tmp.join("go_mod/srv.go"),
            "package go_mod\ntype Server struct{}\nfunc (s *Server) Handle() { s.reply() }\nfunc (s *Server) reply() {}\n",
        )
        .unwrap();

        let graph = analyze_project(&tmp).unwrap();
        assert_eq!(graph.stats.total_files, 5);
        assert_eq!(graph.stats.total_classes, 3);
        assert_eq!(graph.stats.total_functions, 8);
        assert!(graph.edges.iter().any(|e| e.kind == "import" && e.source_id == "file:web/main.ts" && e.target_id == "file:web/utils.ts"));
        assert!(graph.edges.iter().any(|e| e.kind == "call" && e.source_id == "fn:py/app.py::Engine.run" && e.target_id == "fn:py/app.py::Engine.step"));
        assert!(graph.edges.iter().any(|e| e.kind == "call" && e.source_id == "fn:rust_mod/lib.rs::Worker.work" && e.target_id == "fn:rust_mod/lib.rs::Worker.done"));
        assert!(graph.edges.iter().any(|e| e.kind == "call" && e.source_id == "fn:go_mod/srv.go::Server.Handle" && e.target_id == "fn:go_mod/srv.go::Server.reply"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn excludes_files_and_dirs_matching_gitignore() {
        let tmp = std::env::temp_dir().join(format!("vcode_gitignore_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        fs::create_dir_all(tmp.join("ignored_dir")).unwrap();

        fs::write(tmp.join(".gitignore"), "ignored_dir/\n*.gen.py\n").unwrap();
        fs::write(tmp.join("src/app.py"), "def main():\n    pass\n").unwrap();
        fs::write(tmp.join("src/schema.gen.py"), "def gen():\n    pass\n").unwrap();
        fs::write(tmp.join("ignored_dir/secret.py"), "def hidden():\n    pass\n").unwrap();

        let graph = analyze_project(&tmp).unwrap();
        assert_eq!(graph.stats.total_files, 2);
        assert_eq!(graph.stats.total_functions, 1);
        assert!(graph.groups.iter().any(|g| g.id == "file:src/app.py"));
        assert!(graph.groups.iter().any(|g| g.id == "file:.gitignore"));
        assert!(!graph.groups.iter().any(|g| g.id == "file:src/schema.gen.py"));
        assert!(!graph.groups.iter().any(|g| g.id == "dir:ignored_dir"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn includes_text_files_like_json_and_markdown() {
        let tmp = std::env::temp_dir().join(format!("vcode_text_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("docs")).unwrap();

        fs::write(tmp.join("package.json"), "{\n  \"name\": \"demo\"\n}\n").unwrap();
        fs::write(tmp.join("docs/README.md"), "# Hello\nWorld\n").unwrap();
        fs::write(tmp.join("docs/notes.customext"), "line 1\n").unwrap();

        let graph = analyze_project(&tmp).unwrap();
        assert_eq!(graph.stats.total_files, 3);
        assert!(graph.groups.iter().any(|g| g.id == "file:package.json" && g.line_count == 3));
        assert!(graph.groups.iter().any(|g| g.id == "file:docs/README.md" && g.line_count == 2));
        assert!(graph.groups.iter().any(|g| g.id == "file:docs/notes.customext" && g.line_count == 1));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn analyzes_java_cpp_csharp_ruby_and_complexity() {
        let tmp = std::env::temp_dir().join(format!("vcode_langs_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(
            tmp.join("App.java"),
            "public class App {\n    public int run(int x) {\n        if (x > 0) { return helper(x); }\n        return 0;\n    }\n    private int helper(int v) { return v; }\n}\n",
        )
        .unwrap();
        fs::write(
            tmp.join("engine.cpp"),
            "#include \"engine.hpp\"\nint compute(int n) { for (int i=0;i<n;i++) {} return n; }\n",
        )
        .unwrap();
        fs::write(
            tmp.join("Worker.cs"),
            "class Worker { void Run() { Step(); } void Step() {} }\n",
        )
        .unwrap();
        fs::write(
            tmp.join("service.rb"),
            "class Service\n  def call(x)\n    x > 0 ? x : 0\n  end\nend\n",
        )
        .unwrap();

        let graph = analyze_project(&tmp).unwrap();
        let fn_names: Vec<_> = graph.functions.iter().map(|f| (&f.file_path, &f.name, f.complexity)).collect();
        assert!(graph.functions.iter().any(|f| f.name == "run" && f.complexity >= 2), "fns: {fn_names:?}");
        assert!(graph.functions.iter().any(|f| f.name == "compute" && f.complexity >= 2), "fns: {fn_names:?}");
        assert!(graph.functions.iter().any(|f| f.name == "Run"), "fns: {fn_names:?}, errs: {:?}", graph.parse_errors);
        assert!(graph.functions.iter().any(|f| f.name == "call"), "fns: {fn_names:?}");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn caches_parsed_graph_and_invalidates_on_file_change() {
        let tmp = std::env::temp_dir().join(format!("vcode_cache_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("main.py"), "def hello():\n    return 1\n").unwrap();

        let g1 = analyze_project(&tmp).unwrap();
        let mut phases = Vec::new();
        let g2 = analyze_project_with_progress(&tmp, |p| phases.push(p.phase)).unwrap();
        assert_eq!(g1.functions.len(), g2.functions.len());
        assert!(phases.iter().any(|p| p == "Loaded from cache"), "phases: {phases:?}");

        fs::write(
            tmp.join("main.py"),
            "def hello():\n    return 1\n\ndef world():\n    return hello()\n",
        )
        .unwrap();
        let g3 = analyze_project(&tmp).unwrap();
        assert!(g3.functions.iter().any(|f| f.name == "world"));
        assert!(!g3.edges.is_empty());

        let _ = fs::remove_file(graph_cache_file(&tmp.canonicalize().unwrap()));
        let _ = fs::remove_dir_all(&tmp);
    }
}
