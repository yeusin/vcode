use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::{closure::Closure, JsCast};

use crate::canvas::{CanvasView, ReviewOverlay};
use crate::layout::compute_layout;
use crate::sample::sample_project;
use crate::tauri_bridge;
use crate::types::{
    AnalyzeProgress, DiffLine, DiffStatus, FileDiff, FunctionNode, GitDiffSummary, GroupKind,
    GroupNode, LodLevel, ProjectGraph, ProjectSettings, RecentProject, Viewport,
};

fn ease_out_cubic(t: f64) -> f64 {
    let clamped = t.clamp(0.0, 1.0);
    1.0 - (1.0 - clamped).powi(3)
}

fn lerp_viewport(from: Viewport, to: Viewport, t: f64) -> Viewport {
    let e = ease_out_cubic(t);
    Viewport {
        x: from.x + (to.x - from.x) * e,
        y: from.y + (to.y - from.y) * e,
        scale: from.scale + (to.scale - from.scale) * e,
    }
}

fn animate_viewport(
    viewport: RwSignal<Viewport>,
    anim_gen: RwSignal<u64>,
    target: Viewport,
    duration_ms: f64,
) {
    let from = viewport.get_untracked();
    if (from.x - target.x).abs() < 0.5
        && (from.y - target.y).abs() < 0.5
        && (from.scale - target.scale).abs() < 1e-4
    {
        viewport.set(target);
        return;
    }
    let Some(win) = web_sys::window() else {
        viewport.set(target);
        return;
    };
    let gen = anim_gen.get_untracked().wrapping_add(1);
    anim_gen.set(gen);
    let start_ms = js_sys::Date::now();
    let expected = Rc::new(RefCell::new(from));
    let raf_cb: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let raf_cb_clone = raf_cb.clone();

    *raf_cb.borrow_mut() = Some(Closure::new(move || {
        if anim_gen.get_untracked() != gen {
            *raf_cb_clone.borrow_mut() = None;
            return;
        }
        let cur = viewport.get_untracked();
        let prev = *expected.borrow();
        // Stop immediately if the user manually panned or zoomed mid-transition
        if (cur.x - prev.x).abs() > 1.0
            || (cur.y - prev.y).abs() > 1.0
            || (cur.scale - prev.scale).abs() > 1e-3
        {
            *raf_cb_clone.borrow_mut() = None;
            return;
        }
        let t = ((js_sys::Date::now() - start_ms) / duration_ms).clamp(0.0, 1.0);
        let next = lerp_viewport(from, target, t);
        *expected.borrow_mut() = next;
        viewport.set(next);
        if t < 1.0 {
            if let Some(w) = web_sys::window() {
                if let Some(cb) = raf_cb_clone.borrow().as_ref() {
                    let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
                }
            }
        } else {
            *raf_cb_clone.borrow_mut() = None;
        }
    }));

    if let Some(cb) = raf_cb.borrow().as_ref() {
        let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
    };
}

fn format_duration(ms: u64) -> String {
    let secs = (ms + 500) / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else {
        let mins = secs / 60;
        let rem = secs % 60;
        format!("{mins}m {rem:02}s")
    }
}

#[derive(Clone, Debug, PartialEq)]
struct DiffGutterMarker {
    top_pct: f64,
    height_pct: f64,
    kind: &'static str,
}

fn merge_gutter_markers(raw: Vec<DiffGutterMarker>) -> Vec<DiffGutterMarker> {
    let mut merged: Vec<DiffGutterMarker> = Vec::with_capacity(raw.len());
    for item in raw {
        if let Some(last) = merged.last_mut() {
            let last_end = last.top_pct + last.height_pct;
            if last.kind == item.kind && item.top_pct <= last_end + 0.25 {
                let new_end = (item.top_pct + item.height_pct).max(last_end);
                last.height_pct = new_end - last.top_pct;
                continue;
            }
        }
        merged.push(item);
    }
    merged
}

fn build_split_rows(lines: &[DiffLine]) -> Vec<(Option<DiffLine>, Option<DiffLine>)> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind.as_str() {
            "del" => {
                let del_start = i;
                while i < lines.len() && lines[i].kind == "del" {
                    i += 1;
                }
                let add_start = i;
                while i < lines.len() && lines[i].kind == "add" {
                    i += 1;
                }
                let dels = &lines[del_start..add_start];
                let adds = &lines[add_start..i];
                let n = dels.len().max(adds.len());
                for k in 0..n {
                    rows.push((dels.get(k).cloned(), adds.get(k).cloned()));
                }
            }
            "add" => {
                rows.push((None, Some(lines[i].clone())));
                i += 1;
            }
            _ => {
                rows.push((Some(lines[i].clone()), Some(lines[i].clone())));
                i += 1;
            }
        }
    }
    rows
}

fn paths_match(node_path: &str, root_path: &str, fd: &FileDiff) -> bool {
    let norm_node = node_path.replace('\\', "/");
    let norm_root = root_path.replace('\\', "/");
    let norm_root = norm_root.trim_end_matches('/');
    let rel_node = if norm_root.is_empty() {
        norm_node.as_str()
    } else {
        norm_node
            .strip_prefix(&format!("{norm_root}/"))
            .unwrap_or(&norm_node)
    };
    let norm_fd_rel = fd.file_path.replace('\\', "/");
    let norm_fd_abs = fd.abs_path.replace('\\', "/");
    rel_node == norm_fd_rel || norm_node == norm_fd_abs
}

fn is_pr_target(input: &str) -> bool {
    let t = input.trim();
    t.contains("/pull/")
        || t.contains("/-/merge_requests/")
        || t.starts_with('#')
        || (!t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
}

fn file_diff_fingerprint(fd: &FileDiff) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    feed(fd.file_path.as_bytes());
    let status_b = match fd.status {
        DiffStatus::Added => b"A",
        DiffStatus::Modified => b"M",
        DiffStatus::Deleted => b"D",
    };
    feed(status_b);
    feed(&fd.additions.to_le_bytes());
    feed(&fd.deletions.to_le_bytes());
    for h in &fd.hunks {
        feed(&h.old_start.to_le_bytes());
        feed(&h.new_start.to_le_bytes());
        for ln in &h.lines {
            if ln.kind != "context" {
                feed(ln.kind.as_bytes());
                feed(ln.text.as_bytes());
            }
        }
    }
    format!("{hash:016x}")
}

fn viewed_storage_key(root_path: &str, diff: &GitDiffSummary) -> String {
    if let Some(pr) = &diff.pr {
        format!("vcode.viewed::{root_path}::pr::{}::{}", pr.provider, pr.number)
    } else {
        format!("vcode.viewed::{root_path}::git::{}", diff.base_ref)
    }
}

fn load_viewed_map(key: &str) -> HashMap<String, String> {
    tauri_bridge::local_storage_get(key)
        .and_then(|raw| serde_json::from_str::<HashMap<String, String>>(&raw).ok())
        .unwrap_or_default()
}

fn save_viewed_map(key: &str, map: &HashMap<String, String>) {
    if let Ok(json) = serde_json::to_string(map) {
        tauri_bridge::local_storage_set(key, &json);
    }
}

const RECENT_PROJECTS_KEY: &str = "vcode.recentProjects";
const EDITOR_COMMAND_KEY: &str = "vcode.editorCommand";
const OPEN_IN_TERMINAL_KEY: &str = "vcode.openInTerminal";
const DIFF_SPLIT_MODE_KEY: &str = "vcode.diffSplitMode";

fn load_recent_projects() -> Vec<RecentProject> {
    tauri_bridge::local_storage_get(RECENT_PROJECTS_KEY)
        .and_then(|raw| serde_json::from_str::<Vec<RecentProject>>(&raw).ok())
        .unwrap_or_default()
}

fn save_recent_projects(list: &[RecentProject]) {
    if let Ok(json) = serde_json::to_string(list) {
        tauri_bridge::local_storage_set(RECENT_PROJECTS_KEY, &json);
    }
}

fn push_recent_project(path: &str, name: &str, pr_ref: Option<&str>) -> Vec<RecentProject> {
    let mut list = load_recent_projects();
    let existing = list.iter().find(|p| p.path == path).cloned();
    let effective_name = if !name.contains(" #") {
        existing
            .as_ref()
            .filter(|p| p.name.contains(" #"))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| name.to_string())
    } else {
        name.to_string()
    };
    let effective_pr_ref = pr_ref
        .map(str::to_string)
        .or_else(|| existing.and_then(|p| p.pr_ref));
    list.retain(|p| p.path != path);
    list.insert(
        0,
        RecentProject {
            path: path.to_string(),
            name: effective_name,
            pr_ref: effective_pr_ref,
        },
    );
    list.truncate(5);
    save_recent_projects(&list);
    list
}

fn load_editor_command() -> String {
    tauri_bridge::local_storage_get(EDITOR_COMMAND_KEY).unwrap_or_default()
}

fn save_editor_command(cmd: &str) {
    tauri_bridge::local_storage_set(EDITOR_COMMAND_KEY, cmd);
}

#[derive(Clone, PartialEq, Eq)]
struct TreeEntry {
    id: String,
    label: String,
    kind_class: &'static str,
    is_group: bool,
    depth: usize,
    children: Vec<TreeEntry>,
}

fn build_tree(
    graph: &ProjectGraph,
    query: &str,
) -> Vec<TreeEntry> {
    let q = query.trim().to_ascii_lowercase();
    let mut child_groups: HashMap<Option<String>, Vec<&GroupNode>> = HashMap::new();
    for g in &graph.groups {
        child_groups.entry(g.parent_id.clone()).or_default().push(g);
    }
    let mut child_fns: HashMap<String, Vec<&FunctionNode>> = HashMap::new();
    for f in &graph.functions {
        child_fns.entry(f.parent_id.clone()).or_default().push(f);
    }

    fn build_group_entry(
        g: &GroupNode,
        depth: usize,
        q: &str,
        child_groups: &HashMap<Option<String>, Vec<&GroupNode>>,
        child_fns: &HashMap<String, Vec<&FunctionNode>>,
    ) -> Option<TreeEntry> {
        let mut children = Vec::new();
        if let Some(sub_gs) = child_groups.get(&Some(g.id.clone())) {
            for sub in sub_gs {
                if let Some(entry) = build_group_entry(sub, depth + 1, q, child_groups, child_fns) {
                    children.push(entry);
                }
            }
        }
        if let Some(sub_fs) = child_fns.get(&g.id) {
            for f in sub_fs {
                if q.is_empty()
                    || f.name.to_ascii_lowercase().contains(q)
                    || f.qualified_name.to_ascii_lowercase().contains(q)
                {
                    children.push(TreeEntry {
                        id: f.id.clone(),
                        label: f.name.clone(),
                        kind_class: "fn",
                        is_group: false,
                        depth: depth + 1,
                        children: Vec::new(),
                    });
                }
            }
        }

        let self_matches = q.is_empty()
            || g.name.to_ascii_lowercase().contains(q)
            || g.qualified_name.to_ascii_lowercase().contains(q);
        if !self_matches && children.is_empty() {
            return None;
        }

        let kind_class = match g.kind {
            GroupKind::Directory => "directory",
            GroupKind::Module => "module",
            GroupKind::Class => "class",
        };
        Some(TreeEntry {
            id: g.id.clone(),
            label: g.name.clone(),
            kind_class,
            is_group: true,
            depth,
            children,
        })
    }

    let mut roots = Vec::new();
    if let Some(root_gs) = child_groups.get(&None) {
        for g in root_gs {
            if let Some(entry) = build_group_entry(g, 0, &q, &child_groups, &child_fns) {
                roots.push(entry);
            }
        }
    }
    roots
}

fn flatten_tree<'a>(entries: &'a [TreeEntry], collapsed: &HashSet<String>, out: &mut Vec<&'a TreeEntry>) {
    for e in entries {
        out.push(e);
        if !collapsed.contains(&e.id) {
            flatten_tree(&e.children, collapsed, out);
        }
    }
}

#[component]
pub fn App() -> impl IntoView {
    let initial_recent = load_recent_projects();
    let has_initial_recent = !initial_recent.is_empty() && tauri_bridge::is_tauri_available();

    let graph = RwSignal::new(if has_initial_recent {
        ProjectGraph::default()
    } else {
        sample_project()
    });
    let loading_label = RwSignal::new(if has_initial_recent {
        Some(format!("Analyzing {}...", initial_recent[0].name))
    } else {
        None
    });
    let analyze_progress = RwSignal::new(Option::<AnalyzeProgress>::None);
    let error_banner = RwSignal::new(Option::<String>::None);
    let recent_projects = RwSignal::new(initial_recent);

    let viewport = RwSignal::new(Viewport::default());
    let vp_anim_gen = RwSignal::new(0_u64);
    let selected_id = RwSignal::new(Option::<String>::None);
    let hovered_id = RwSignal::new(Option::<String>::None);
    let sidebar_open = RwSignal::new(true);
    let search_query = RwSignal::new(String::new());
    let collapsed_ids = RwSignal::new(HashSet::<String>::new());

    let editor_command = RwSignal::new(load_editor_command());
    let open_in_terminal = RwSignal::new(
        tauri_bridge::local_storage_get(OPEN_IN_TERMINAL_KEY).as_deref() == Some("true"),
    );
    let settings_open = RwSignal::new(false);
    let snap_menu_open = RwSignal::new(false);
    let open_menu_open = RwSignal::new(false);
    let open_url_modal = RwSignal::new(false);
    let open_url_input = RwSignal::new(String::new());
    let open_url_error = RwSignal::new(Option::<String>::None);
    let settings_input = RwSignal::new(load_editor_command());
    let open_in_terminal_input = RwSignal::new(open_in_terminal.get_untracked());
    let pending_editor_file = RwSignal::new(Option::<String>::None);

    // Code Review & PR state
    let review_changed_only = RwSignal::new(false);
    let diff_summary = RwSignal::new(Option::<GitDiffSummary>::None);
    let git_base_ref = RwSignal::new("HEAD".to_string());
    let active_pr_ref = RwSignal::new(Option::<String>::None);
    let git_token = RwSignal::new(
        tauri_bridge::local_storage_get("vcode.gitToken").unwrap_or_default(),
    );
    let token_input = RwSignal::new(git_token.get_untracked());
    let review_busy = RwSignal::new(false);
    let review_error = RwSignal::new(Option::<String>::None);
    let diff_modal_file = RwSignal::new(Option::<String>::None);
    let diff_gutter_markers = RwSignal::new(Vec::<DiffGutterMarker>::new());
    let diff_split_mode = RwSignal::new(
        tauri_bridge::local_storage_get(DIFF_SPLIT_MODE_KEY).as_deref() == Some("true"),
    );
    let viewed_files = RwSignal::new(HashMap::<String, String>::new());

    let layout = Memo::new(move |_| Arc::new(graph.with(|g| compute_layout(g))));

    let review_overlay = Memo::new(move |_| {
        let diff_opt = diff_summary.get();
        let is_pr_loaded = diff_opt
            .as_ref()
            .and_then(|d| d.pr.as_ref())
            .is_some();

        let mut changed_ids = HashSet::new();
        let mut diff_stats: HashMap<String, (usize, usize)> = HashMap::new();

        graph.with(|g| {
            if let Some(diff) = &diff_opt {
                for fd in &diff.files {
                    for grp in &g.groups {
                        if grp.kind == GroupKind::Module {
                            if let Some(fp) = &grp.file_path {
                                if paths_match(fp, &g.root_path, fd) {
                                    changed_ids.insert(grp.id.clone());
                                    diff_stats
                                        .insert(grp.id.clone(), (fd.additions, fd.deletions));
                                }
                            }
                        }
                    }
                    for fnode in &g.functions {
                        if !paths_match(&fnode.file_path, &g.root_path, fd) {
                            continue;
                        }
                        let whole_file = matches!(fd.status, DiffStatus::Added | DiffStatus::Deleted);
                        let hunk_hits = fd.hunks.iter().any(|h| {
                            h.lines.iter().any(|ln| {
                                if ln.kind == "context" {
                                    return false;
                                }
                                let lno = ln.new_line.or(ln.old_line).unwrap_or(0);
                                lno >= fnode.start_line && lno <= fnode.end_line
                            })
                        });
                        if whole_file || hunk_hits {
                            changed_ids.insert(fnode.id.clone());
                        }
                    }
                }
            }
        });

        ReviewOverlay {
            active: true,
            changed_only: is_pr_loaded || review_changed_only.get(),
            changed_ids,
            diff_stats,
        }
    });

    let refresh_local_review = move || {
        active_pr_ref.set(None);
        let proj = graph.get_untracked().root_path;
        if proj.is_empty() || proj.starts_with("sample://") || !tauri_bridge::is_tauri_available() {
            return;
        }
        let base = git_base_ref.get_untracked();
        let base = if is_pr_target(&base) {
            "HEAD".to_string()
        } else {
            base
        };
        review_busy.set(true);
        review_error.set(None);
        spawn_local(async move {
            let base_opt = if base.trim().is_empty() {
                None
            } else {
                Some(base.as_str())
            };
            match tauri_bridge::get_git_diff(&proj, base_opt).await {
                Ok(d) => diff_summary.set(Some(d)),
                Err(e) => review_error.set(Some(e)),
            }
            review_busy.set(false);
        });
    };

    let canvas_viewport_size = || -> (f64, f64) {
        web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| {
                d.query_selector(".canvas-viewport")
                    .ok()
                    .flatten()
                    .or_else(|| d.query_selector(".canvas-stage").ok().flatten())
            })
            .map(|el| {
                let r = el.get_bounding_client_rect();
                (r.width(), r.height())
            })
            .unwrap_or((1000.0, 720.0))
    };

    let fit_to_view = move || {
        let l = layout.get_untracked();
        if l.bounds.width <= 0.0 || l.bounds.height <= 0.0 {
            return;
        }
        let (vw, vh) = canvas_viewport_size();
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let scale = ((vw - 64.0) / l.bounds.width)
            .min((vh - 64.0) / l.bounds.height)
            .clamp(0.001, 1.0);
        let x = (vw - l.bounds.width * scale) / 2.0;
        let y = (vh - l.bounds.height * scale) / 2.0;
        animate_viewport(viewport, vp_anim_gen, Viewport { x, y, scale }, 200.0);
    };

    let refresh_pr_review = move |pr_val: String| {
        let proj = graph.get_untracked().root_path;
        let tok = git_token.get_untracked();
        let is_full_url = pr_val.contains("://");
        if (!is_full_url && (proj.is_empty() || proj.starts_with("sample://")))
            || pr_val.trim().is_empty()
            || !tauri_bridge::is_tauri_available()
            || review_busy.get_untracked()
        {
            return;
        }
        let is_initial_load = active_pr_ref.get_untracked().as_deref() != Some(pr_val.as_str());
        if is_initial_load {
            let label = recent_projects
                .get_untracked()
                .into_iter()
                .find(|rp| rp.pr_ref.as_deref() == Some(pr_val.as_str()))
                .map(|rp| format!("Loading {}...", rp.name))
                .unwrap_or_else(|| "Loading PR...".to_string());
            loading_label.set(Some(label));
            analyze_progress.set(None);
        }
        review_busy.set(true);
        review_error.set(None);
        spawn_local(async move {
            let tok_opt = if tok.trim().is_empty() {
                None
            } else {
                Some(tok.as_str())
            };
            match tauri_bridge::fetch_pr_review(&proj, &pr_val, tok_opt).await {
                Ok(pr_diff) => {
                    let prev_diff = diff_summary.get_untracked();
                    if let Some(pr) = &pr_diff.pr {
                        let pr_label = if pr.repo_name.trim().is_empty() {
                            format!("PR #{}", pr.number)
                        } else {
                            format!("{} #{}", pr.repo_name, pr.number)
                        };
                        if let Some(checkout_dir) = pr.checkout_path.clone() {
                            let switched = graph.with_untracked(|g| g.root_path != checkout_dir);
                            let sha_unchanged = !switched
                                && !pr.head_sha.is_empty()
                                && prev_diff
                                    .as_ref()
                                    .and_then(|d| d.pr.as_ref())
                                    .is_some_and(|p| p.head_sha == pr.head_sha);
                            if !sha_unchanged {
                                if switched || is_initial_load {
                                    loading_label.set(Some(format!("Analyzing {pr_label}...")));
                                }
                                if switched {
                                    selected_id.set(None);
                                    hovered_id.set(None);
                                }
                                match tauri_bridge::analyze_project(&checkout_dir).await {
                                    Ok(new_graph) => {
                                        let saved_pr_ref = if pr.url.trim().is_empty() {
                                            pr_val.clone()
                                        } else {
                                            pr.url.clone()
                                        };
                                        let recent = push_recent_project(
                                            &checkout_dir,
                                            &pr_label,
                                            Some(&saved_pr_ref),
                                        );
                                        recent_projects.set(recent);
                                        let _ = tauri_bridge::set_window_title(&format!(
                                            "VCode — {pr_label}"
                                        ))
                                        .await;
                                        if switched {
                                            if new_graph.groups.len() > 150
                                                || new_graph.functions.len() > 500
                                            {
                                                let collapsed: HashSet<String> = new_graph
                                                    .groups
                                                    .iter()
                                                    .filter(|grp| grp.id != "dir:.")
                                                    .map(|grp| grp.id.clone())
                                                    .collect();
                                                collapsed_ids.set(collapsed);
                                            } else {
                                                collapsed_ids.set(HashSet::new());
                                            }
                                        }
                                        graph.set(new_graph);
                                        loading_label.set(None);
                                        analyze_progress.set(None);
                                        if switched {
                                            fit_to_view();
                                            let _ = tauri_bridge::watch_project(&checkout_dir).await;
                                        }
                                    }
                                    Err(err) => {
                                        loading_label.set(None);
                                        analyze_progress.set(None);
                                        error_banner.set(Some(err));
                                    }
                                }
                            }
                        }
                    }
                    loading_label.set(None);
                    analyze_progress.set(None);
                    active_pr_ref.set(Some(pr_val));
                    if prev_diff.as_ref() != Some(&pr_diff) {
                        diff_summary.set(Some(pr_diff));
                    }
                    review_changed_only.set(true);
                }
                Err(e) => {
                    loading_label.set(None);
                    analyze_progress.set(None);
                    review_error.set(Some(e));
                }
            }
            review_busy.set(false);
        });
    };

    let persist_project_settings = move || {
        let g = graph.get_untracked();
        if g.root_path.is_empty() || g.root_path.starts_with("sample://") || !tauri_bridge::is_tauri_available() {
            return;
        }
        let cmd = editor_command.get_untracked();
        let settings = ProjectSettings {
            show_import_edges: false,
            show_call_edges: false,
            hidden_import_group_ids: Vec::new(),
            hidden_call_group_ids: Vec::new(),
            editor_command: if cmd.trim().is_empty() { None } else { Some(cmd) },
            open_in_terminal: open_in_terminal.get_untracked(),
        };
        let path = g.root_path.clone();
        spawn_local(async move {
            let _ = tauri_bridge::save_project_settings(&path, &settings).await;
        });
    };

    let focus_node = Callback::new(move |id: String| {
        let l = layout.get_untracked();
        let Some(rect) = l.node_rects.get(&id).copied() else {
            return;
        };
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let (vw, vh) = canvas_viewport_size();
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let scale = ((vw - 48.0) / rect.width)
            .min((vh - 48.0) / rect.height)
            .clamp(0.001, 1.15);
        let cx = rect.x + rect.width / 2.0;
        let cy = rect.y + rect.height / 2.0;
        selected_id.set(Some(id));
        animate_viewport(
            viewport,
            vp_anim_gen,
            Viewport {
                x: vw / 2.0 - cx * scale,
                y: vh / 2.0 - cy * scale,
                scale,
            },
            200.0,
        );
    });

    let open_project = move |path: String| {
        let display_name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&path)
            .to_string();
        loading_label.set(Some(format!("Analyzing {display_name}...")));
        analyze_progress.set(None);
        error_banner.set(None);
        selected_id.set(None);
        hovered_id.set(None);

        spawn_local(async move {
            match tauri_bridge::analyze_project(&path).await {
                Ok(new_graph) => {
                    let saved = tauri_bridge::load_project_settings(&path).await.ok().flatten();
                    if let Some(s) = saved {
                        if let Some(cmd) = s.editor_command {
                            if !cmd.trim().is_empty() {
                                editor_command.set(cmd.clone());
                                save_editor_command(&cmd);
                            }
                        }
                        open_in_terminal.set(s.open_in_terminal);
                        tauri_bridge::local_storage_set(
                            OPEN_IN_TERMINAL_KEY,
                            if s.open_in_terminal { "true" } else { "false" },
                        );
                    }

                    let recent = push_recent_project(&new_graph.root_path, &new_graph.root_name, None);
                    let title_name = recent
                        .first()
                        .map(|r| r.name.clone())
                        .unwrap_or_else(|| new_graph.root_name.clone());
                    recent_projects.set(recent);
                    let _ = tauri_bridge::set_window_title(&format!("VCode — {title_name}")).await;
                    if new_graph.groups.len() > 150 || new_graph.functions.len() > 500 {
                        let collapsed: HashSet<String> = new_graph
                            .groups
                            .iter()
                            .filter(|grp| grp.id != "dir:.")
                            .map(|grp| grp.id.clone())
                            .collect();
                        collapsed_ids.set(collapsed);
                    } else {
                        collapsed_ids.set(HashSet::new());
                    }
                    graph.set(new_graph);
                    loading_label.set(None);
                    analyze_progress.set(None);
                    fit_to_view();
                    active_pr_ref.set(None);
                    if is_pr_target(&git_base_ref.get_untracked()) {
                        git_base_ref.set("HEAD".to_string());
                    }
                    refresh_local_review();
                    let _ = tauri_bridge::watch_project(&path).await;
                }
                Err(err) => {
                    loading_label.set(None);
                    analyze_progress.set(None);
                    error_banner.set(Some(err));
                    if graph.get_untracked().root_path.is_empty() {
                        graph.set(sample_project());
                        fit_to_view();
                    }
                }
            }
        });
    };

    let toggle_file_viewed = move |file_path: String, fp: String, checked: bool| {
        viewed_files.update(|map| {
            if checked {
                map.insert(file_path, fp);
            } else {
                map.remove(&file_path);
            }
            if let Some(diff) = diff_summary.get_untracked() {
                let root = graph.with_untracked(|g| g.root_path.clone());
                if !root.is_empty() {
                    save_viewed_map(&viewed_storage_key(&root, &diff), map);
                }
            }
        });
    };

    // Sync viewed files with current diff and clear any file whose diff changed
    Effect::new(move |_| {
        let Some(diff) = diff_summary.get() else {
            viewed_files.set(HashMap::new());
            return;
        };
        let root = graph.with_untracked(|g| g.root_path.clone());
        if root.is_empty() {
            return;
        }
        let key = viewed_storage_key(&root, &diff);
        let stored = load_viewed_map(&key);
        let mut pruned = HashMap::new();
        for fd in &diff.files {
            let fp = file_diff_fingerprint(fd);
            if stored.get(&fd.file_path).is_some_and(|s| s == &fp) {
                pruned.insert(fd.file_path.clone(), fp);
            }
        }
        if pruned != stored {
            save_viewed_map(&key, &pruned);
        }
        viewed_files.set(pruned);
    });

    // Startup: auto-open last project + subscribe to live project-updated events
    Effect::new(move |_| {
        if tauri_bridge::is_tauri_available() {
            spawn_local(async move {
                let _ = tauri_bridge::listen_project_updated(move |updated| {
                    graph.set(updated);
                    if active_pr_ref.get_untracked().is_none()
                        && !is_pr_target(&git_base_ref.get_untracked())
                    {
                        refresh_local_review();
                    }
                })
                .await;
            });
            spawn_local(async move {
                let _ = tauri_bridge::listen_analyze_progress(move |progress| {
                    analyze_progress.set(Some(progress));
                })
                .await;
            });
            let initial_from_query = web_sys::window()
                .and_then(|w| w.location().search().ok())
                .and_then(|search| {
                    search
                        .trim_start_matches('?')
                        .split('&')
                        .find_map(|pair| pair.strip_prefix("project="))
                        .and_then(|enc| js_sys::decode_uri_component(enc).ok())
                        .and_then(|v| v.as_string())
                        .filter(|s| !s.trim().is_empty())
                });
            if let Some(proj_path) = initial_from_query {
                open_project(proj_path);
            } else {
                spawn_local(async move {
                    if let Some(target) = tauri_bridge::get_launch_target().await {
                        if is_pr_target(&target) {
                            refresh_pr_review(target);
                        } else if target.starts_with("http://")
                            || target.starts_with("https://")
                            || target.starts_with("git@")
                        {
                            loading_label.set(Some(format!("Cloning {target}...")));
                            analyze_progress.set(None);
                            let tok = git_token.get_untracked();
                            let tok_opt = if tok.trim().is_empty() { None } else { Some(tok.as_str()) };
                            match tauri_bridge::clone_github_repo(&target, tok_opt).await {
                                Ok(path) => open_project(path),
                                Err(err) => {
                                    loading_label.set(None);
                                    error_banner.set(Some(err));
                                }
                            }
                        } else {
                            open_project(target);
                        }
                    } else {
                        let recent = load_recent_projects();
                        if let Some(first) = recent.first() {
                            open_project(first.path.clone());
                        } else {
                            fit_to_view();
                        }
                    }
                });
            }
        } else {
            fit_to_view();
        }
    });

    // Auto-scroll sidebar to selected diagram
    Effect::new(move |_| {
        let Some(sid) = selected_id.get() else {
            return;
        };
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            let selector = format!("[data-node-id=\"{}\"]", sid.replace('"', "\\\""));
            if let Ok(Some(el)) = doc.query_selector(&selector) {
                el.scroll_into_view_with_bool(false);
            }
        }
    });

    // Auto-scroll diff popup to the first changed line and update scrollbar gutter markers
    Effect::new(move |_| {
        let Some(_) = diff_modal_file.get() else {
            diff_gutter_markers.set(Vec::new());
            return;
        };
        let _mode = diff_split_mode.get();
        if let Some(win) = web_sys::window() {
            let cb = Closure::once_into_js(move || {
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    if let Ok(Some(body)) = doc.query_selector(".diff-modal-body") {
                        let body_rect = body.get_bounding_client_rect();
                        let scroll_height = body.scroll_height() as f64;
                        let scroll_top = body.scroll_top() as f64;

                        let mut raw_markers = Vec::new();
                        if scroll_height > 0.0 {
                            if let Ok(nodes) = body.query_selector_all(
                                ".diff-line.is-add, .diff-line.is-del, .diff-split-row",
                            ) {
                                for idx in 0..nodes.length() {
                                    let Some(node) = nodes.item(idx) else {
                                        continue;
                                    };
                                    let Ok(el) = node.dyn_into::<web_sys::Element>() else {
                                        continue;
                                    };
                                    let cls = el.class_name();
                                    let kind = if cls.contains("diff-split-row") {
                                        let has_del =
                                            el.query_selector(".is-del").ok().flatten().is_some();
                                        let has_add =
                                            el.query_selector(".is-add").ok().flatten().is_some();
                                        match (has_del, has_add) {
                                            (true, true) => "both",
                                            (true, false) => "del",
                                            (false, true) => "add",
                                            (false, false) => continue,
                                        }
                                    } else if cls.contains("is-add") {
                                        "add"
                                    } else {
                                        "del"
                                    };
                                    let el_rect = el.get_bounding_client_rect();
                                    let abs_top = el_rect.top() - body_rect.top() + scroll_top;
                                    let top_pct = (abs_top / scroll_height * 100.0).clamp(0.0, 100.0);
                                    let height_pct =
                                        (el_rect.height() / scroll_height * 100.0).max(0.0);
                                    raw_markers.push(DiffGutterMarker {
                                        top_pct,
                                        height_pct,
                                        kind,
                                    });
                                }
                            }
                        }
                        diff_gutter_markers.set(merge_gutter_markers(raw_markers));

                        if let Ok(Some(first_diff)) =
                            body.query_selector(".is-add, .is-del")
                        {
                            let diff_rect = first_diff.get_bounding_client_rect();
                            let target_top =
                                (diff_rect.top() - body_rect.top() + scroll_top - 120.0).max(0.0);
                            body.set_scroll_top(target_top.round() as i32);
                        }
                    }
                }
            });
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.unchecked_ref(),
                0,
            );
        }
    });

    let handle_open_folder = move || {
        if !tauri_bridge::is_tauri_available() {
            error_banner.set(Some("Folder picker requires the Tauri desktop app.".to_string()));
            return;
        }
        spawn_local(async move {
            match tauri_bridge::pick_project_folder().await {
                Ok(Some(path)) => open_project(path),
                Ok(None) => {}
                Err(err) => error_banner.set(Some(err)),
            }
        });
    };

    let submit_open_url = move || {
        let val = open_url_input.get_untracked().trim().to_string();
        if val.is_empty() {
            open_url_error.set(Some("Please enter a GitHub repo or PR URL.".to_string()));
            return;
        }
        if is_pr_target(&val) {
            open_url_error.set(None);
            open_url_modal.set(false);
            refresh_pr_review(val);
            return;
        }
        if !tauri_bridge::is_tauri_available() {
            open_url_error.set(Some("Cloning repositories requires the Tauri desktop app.".to_string()));
            return;
        }
        open_url_error.set(None);
        open_url_modal.set(false);
        loading_label.set(Some(format!("Cloning {val}...")));
        analyze_progress.set(None);
        let tok = git_token.get_untracked();
        spawn_local(async move {
            let tok_opt = if tok.trim().is_empty() { None } else { Some(tok.as_str()) };
            match tauri_bridge::clone_github_repo(&val, tok_opt).await {
                Ok(path) => open_project(path),
                Err(err) => {
                    loading_label.set(None);
                    error_banner.set(Some(err));
                }
            }
        });
    };

    let handle_open_in_editor = Callback::new(move |file_path: String| {
        let cmd = editor_command.get_untracked();
        if cmd.trim().is_empty() {
            pending_editor_file.set(Some(file_path));
            settings_input.set(String::new());
            open_in_terminal_input.set(open_in_terminal.get_untracked());
            token_input.set(git_token.get_untracked());
            settings_open.set(true);
            return;
        }
        let proj = graph.get_untracked().root_path;
        let proj_opt = if proj.is_empty() || proj.starts_with("sample://") {
            None
        } else {
            Some(proj)
        };
        let in_term = open_in_terminal.get_untracked();
        spawn_local(async move {
            if let Err(err) = tauri_bridge::open_in_editor(&cmd, &file_path, proj_opt.as_deref(), in_term).await {
                error_banner.set(Some(err));
            }
        });
    });

    let flat_tree = Memo::new(move |_| {
        let q = search_query.get();
        let collapsed = collapsed_ids.get();
        let tree = graph.with(|g| build_tree(g, &q));
        let mut flat = Vec::new();
        flatten_tree(&tree, &collapsed, &mut flat);
        flat.into_iter().take(400).cloned().collect::<Vec<_>>()
    });

    view! {
        <div class="app-shell" on:click=move |_| open_menu_open.set(false)>
            <header class="toolbar">
                <div class="toolbar-left">
                    <button
                        class="btn btn-ghost"
                        title="Toggle sidebar"
                        on:click=move |_| sidebar_open.update(|v| *v = !*v)
                    >
                        "☰"
                    </button>
                    <div class="brand">
                        <span class="brand-logo">"VCode"</span>
                        <span class="brand-project">{move || graph.get().root_name}</span>
                    </div>
                    <div class="open-menu-wrap" on:click=move |ev| ev.stop_propagation()>
                        <button
                            class="btn btn-primary"
                            on:click=move |_| open_menu_open.update(|v| *v = !*v)
                        >
                            "Open Project ▾"
                        </button>
                        <Show when=move || open_menu_open.get()>
                            <div class="open-menu-popover">
                                <button
                                    class="open-menu-item"
                                    on:click=move |_| {
                                        open_menu_open.set(false);
                                        handle_open_folder();
                                    }
                                >
                                    "Local / Cloned Directory..."
                                </button>
                                <button
                                    class="open-menu-item"
                                    on:click=move |_| {
                                        open_menu_open.set(false);
                                        open_url_input.set(String::new());
                                        open_url_error.set(None);
                                        open_url_modal.set(true);
                                    }
                                >
                                    "From GitHub / PR URL..."
                                </button>
                            </div>
                        </Show>
                    </div>
                    <button
                        class="btn btn-ghost"
                        title="Open a new project window"
                        on:click=move |_| {
                            if !tauri_bridge::is_tauri_available() {
                                return;
                            }
                            spawn_local(async move {
                                let _ = tauri_bridge::open_new_window(None).await;
                            });
                        }
                    >
                        "+ Window"
                    </button>
                    <div class="recent-chips">
                        {move || {
                            recent_projects.get().into_iter().map(|rp| {
                                let p = rp.path.clone();
                                let pr_ref_opt = rp.pr_ref.clone();
                                let title = format!("{} (Ctrl/Cmd+Click to open in new window)", rp.path);
                                view! {
                                    <button
                                        class="recent-chip"
                                        title=title
                                        on:click=move |ev| {
                                            if (ev.ctrl_key() || ev.meta_key()) && tauri_bridge::is_tauri_available() {
                                                let path = p.clone();
                                                spawn_local(async move {
                                                    let _ = tauri_bridge::open_new_window(Some(&path)).await;
                                                });
                                            } else if let Some(pr_target) = pr_ref_opt.clone() {
                                                refresh_pr_review(pr_target);
                                            } else {
                                                open_project(p.clone());
                                            }
                                        }
                                    >
                                        {rp.name}
                                    </button>
                                }
                            }).collect_view()
                        }}
                    </div>
                </div>

                <div class="toolbar-right">
                    <button
                        class=move || {
                            let pr_loaded = diff_summary.get().and_then(|d| d.pr).is_some();
                            if pr_loaded || review_changed_only.get() {
                                "toggle-pill impact-pill is-active"
                            } else {
                                "toggle-pill impact-pill"
                            }
                        }
                        title="Dim unchanged code and highlight modified files/functions (always on when a PR is loaded)"
                        on:click=move |_| {
                            if diff_summary.get_untracked().and_then(|d| d.pr).is_none() {
                                review_changed_only.update(|v| *v = !*v);
                            }
                        }
                    >
                        "Changed Only"
                    </button>
                    <div class="divider" />
                    <span class=move || {
                        let lod = LodLevel::from_scale(viewport.get().scale);
                        match lod {
                            LodLevel::Macro => "lod-badge lod-badge-macro",
                            LodLevel::Meso => "lod-badge lod-badge-meso",
                            LodLevel::Micro => "lod-badge lod-badge-micro",
                        }
                    }>
                        {move || LodLevel::from_scale(viewport.get().scale).label()}
                    </span>
                    <div class="zoom-controls">
                        <button
                            class="btn btn-ghost"
                            on:click=move |_| viewport.update(|vp| vp.scale = (vp.scale / 1.18).clamp(0.08, 3.0))
                        >
                            "-"
                        </button>
                        <span class="zoom-pct">
                            {move || format!("{}%", (viewport.get().scale * 100.0).round() as i32)}
                        </span>
                        <button
                            class="btn btn-ghost"
                            on:click=move |_| viewport.update(|vp| vp.scale = (vp.scale * 1.18).clamp(0.08, 3.0))
                        >
                            "+"
                        </button>
                        <button class="btn btn-ghost" on:click=move |_| fit_to_view()>
                            "Fit"
                        </button>
                    </div>
                    <div class="snap-menu-wrap">
                        <button
                            class="btn btn-ghost"
                            title="Snap and align window"
                            on:click=move |_| snap_menu_open.update(|v| *v = !*v)
                        >
                            "Snap"
                        </button>
                        <Show when=move || snap_menu_open.get()>
                            <div class="snap-popover">
                                <div class="snap-grid">
                                    {[
                                        ("left", "◧ Left ½"),
                                        ("right", "◨ Right ½"),
                                        ("top", "⬒ Top ½"),
                                        ("bottom", "⬓ Bottom ½"),
                                        ("top-left", "◰ Top-Left ¼"),
                                        ("top-right", "◱ Top-Right ¼"),
                                        ("bottom-left", "◲ Bot-Left ¼"),
                                        ("bottom-right", "◳ Bot-Right ¼"),
                                        ("left-third", "Left ⅓"),
                                        ("center-third", "Mid ⅓"),
                                        ("right-third", "Right ⅓"),
                                        ("center", "Center"),
                                        ("maximize", "Full"),
                                    ].into_iter().map(|(preset, label)| {
                                        view! {
                                            <button
                                                class="snap-item"
                                                on:click=move |_| {
                                                    snap_menu_open.set(false);
                                                    spawn_local(async move {
                                                        let _ = tauri_bridge::snap_window(preset).await;
                                                    });
                                                }
                                            >
                                                {label}
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        </Show>
                    </div>
                    <button
                        class="btn btn-ghost"
                        title="Settings"
                        on:click=move |_| {
                            settings_input.set(editor_command.get_untracked());
                            open_in_terminal_input.set(open_in_terminal.get_untracked());
                            token_input.set(git_token.get_untracked());
                            pending_editor_file.set(None);
                            settings_open.set(true);
                        }
                    >
                        "⚙"
                    </button>
                </div>
            </header>

            {move || {
                error_banner.get().map(|err| view! {
                    <div class="error-banner">
                        <span>{err}</span>
                        <button class="btn btn-ghost" on:click=move |_| error_banner.set(None)>"✕"</button>
                    </div>
                })
            }}

            <div class="workspace">
                <aside class=move || if sidebar_open.get() { "sidebar" } else { "sidebar is-collapsed" }>
                        <div class="sidebar-stats">
                            <div class="stat-chip">
                                <span class="stat-val">{move || graph.get().stats.total_files}</span>
                                <span class="stat-lbl">"Files"</span>
                            </div>
                            <div class="stat-chip">
                                <span class="stat-val">{move || graph.get().stats.total_classes}</span>
                                <span class="stat-lbl">"Classes"</span>
                            </div>
                            <div class="stat-chip">
                                <span class="stat-val">{move || graph.get().stats.total_functions}</span>
                                <span class="stat-lbl">"Functions"</span>
                            </div>
                        </div>

                        <div class="sidebar-section-title">"Project Structure"</div>

                        <div class="sidebar-search">
                            <div class="search-box">
                                <svg class="search-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <circle cx="11" cy="11" r="8" />
                                    <path d="m21 21-4.3-4.3" />
                                </svg>
                                <input
                                    type="text"
                                    placeholder="Filter files, classes, functions..."
                                    prop:value=move || search_query.get()
                                    on:input=move |ev| search_query.set(event_target_value(&ev))
                                />
                                <Show when=move || !search_query.get().is_empty()>
                                    <button
                                        class="search-clear"
                                        title="Clear filter"
                                        on:click=move |_| search_query.set(String::new())
                                    >
                                        "×"
                                    </button>
                                </Show>
                            </div>
                        </div>

                        <div class="sidebar-tree">
                            {move || {
                                let entries = flat_tree.get();
                                if entries.is_empty() {
                                    return view! {
                                        <div class="sidebar-empty">"No matching items"</div>
                                    }.into_any();
                                }
                                entries.into_iter().take(400).map(|entry| {
                                    let id = entry.id.clone();
                                    let id_row_sel = entry.id.clone();
                                    let id_sel = entry.id.clone();
                                    let id_dbl = entry.id.clone();
                                    let id_col = entry.id.clone();
                                    let has_children = !entry.children.is_empty();
                                    let kind_class = entry.kind_class;
                                    let indent = format!("padding-left: {}px;", 6 + entry.depth * 14);
                                    view! {
                                        <div
                                            class=move || {
                                                if selected_id.get().as_deref() == Some(id_row_sel.as_str()) {
                                                    "tree-row is-selected"
                                                } else {
                                                    "tree-row"
                                                }
                                            }
                                            style=indent
                                            attr:data-node-id=id
                                        >
                                            {if has_children {
                                                let id_c = id_col.clone();
                                                view! {
                                                    <button
                                                        class="tree-caret"
                                                        on:click=move |ev| {
                                                            ev.stop_propagation();
                                                            collapsed_ids.update(|set| {
                                                                if !set.remove(&id_c) {
                                                                    set.insert(id_c.clone());
                                                                }
                                                            });
                                                        }
                                                    >
                                                        {move || if collapsed_ids.get().contains(&id_col) {
                                                            view! {
                                                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                                    <path d="m9 18 6-6-6-6" />
                                                                </svg>
                                                            }.into_any()
                                                        } else {
                                                            view! {
                                                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                                    <path d="m6 9 6 6 6-6" />
                                                                </svg>
                                                            }.into_any()
                                                        }}
                                                    </button>
                                                }.into_any()
                                            } else {
                                                view! { <span class="tree-caret-spacer" /> }.into_any()
                                            }}
                                            <button
                                                class=format!("tree-item tree-{kind_class}")
                                                on:click=move |_| selected_id.set(Some(id_sel.clone()))
                                                on:dblclick=move |_| focus_node.run(id_dbl.clone())
                                            >
                                                <span class="tree-icon">
                                                    {match kind_class {
                                                        "directory" => view! {
                                                            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                                <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
                                                            </svg>
                                                        }.into_any(),
                                                        "module" => view! {
                                                            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                                <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
                                                                <path d="M14 2v4a2 2 0 0 0 2 2h4" />
                                                            </svg>
                                                        }.into_any(),
                                                        "class" => view! {
                                                            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                                <path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z" />
                                                                <path d="m3.3 7 8.7 5 8.7-5" />
                                                                <path d="M12 22V12" />
                                                            </svg>
                                                        }.into_any(),
                                                        _ => view! {
                                                            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                                <polyline points="16 18 22 12 16 6" />
                                                                <polyline points="8 6 2 12 8 18" />
                                                            </svg>
                                                        }.into_any(),
                                                    }}
                                                </span>
                                                <span class="tree-label">{entry.label}</span>
                                            </button>
                                        </div>
                                    }
                                }).collect_view().into_any()
                            }}
                        </div>

                        {move || {
                            let errs = graph.get().parse_errors;
                            if errs.is_empty() {
                                view! { <span /> }.into_any()
                            } else {
                                view! {
                                    <div class="sidebar-errors">
                                        <div class="errors-header">
                                            <span>{format!("Parse Errors ({})", errs.len())}</span>
                                        </div>
                                        {errs.into_iter().map(|pe| view! {
                                            <div class="error-item">
                                                <div class="error-file">{pe.file_path}</div>
                                                <div class="error-msg">{pe.error}</div>
                                            </div>
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            }
                        }}
                    </aside>

                <main class="canvas-stage">
                    {move || {
                        if let Some(label) = loading_label.get() {
                            let progress = analyze_progress.get();
                            view! {
                                <div class="canvas-loading">
                                    <div class="loading-spinner" />
                                    <div>{label}</div>
                                    {progress.map(|p| {
                                        let pct = if p.total > 0 {
                                            ((p.processed * 100) / p.total).min(100)
                                        } else {
                                            0
                                        };
                                        let detail = if p.total > 0 {
                                            let eta = p.eta_ms
                                                .map(|ms| format!(" · ~{} remaining", format_duration(ms)))
                                                .unwrap_or_default();
                                            format!("{}: {} / {} ({}%){eta}", p.phase, p.processed, p.total, pct)
                                        } else {
                                            format!("{} ({} files found)", p.phase, p.processed)
                                        };
                                        view! {
                                            <div class="analyze-progress-card">
                                                {(p.total > 0).then(|| view! {
                                                    <div class="analyze-progress-track">
                                                        <div
                                                            class="analyze-progress-fill"
                                                            style=format!("width: {pct}%")
                                                        />
                                                    </div>
                                                })}
                                                <div class="analyze-progress-meta">{detail}</div>
                                            </div>
                                        }
                                    })}
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <CanvasView
                                    layout=layout
                                    viewport=viewport
                                    selected_id=selected_id
                                    hovered_id=hovered_id
                                    review_overlay=review_overlay
                                    on_open_in_editor=handle_open_in_editor
                                    on_focus_node=focus_node
                                />
                            }.into_any()
                        }
                    }}
                </main>

                <aside class="review-panel">
                    <div class="review-header">
                        <span class="review-title">"Code Review"</span>
                    </div>

                        <div class="review-controls">
                            <Show when=move || active_pr_ref.get().is_none()>
                                <div class="review-row">
                                    <input
                                        class="review-input"
                                        type="text"
                                        placeholder="Branch / ref (e.g. HEAD, main)"
                                        prop:value=move || git_base_ref.get()
                                        on:input=move |ev| git_base_ref.set(event_target_value(&ev))
                                        on:keydown=move |ev| {
                                            if ev.key() == "Enter" {
                                                let val = git_base_ref.get_untracked();
                                                if is_pr_target(&val) {
                                                    review_error.set(Some("Use Open Project ▾ → From GitHub / PR URL... to open a PR.".to_string()));
                                                } else {
                                                    refresh_local_review();
                                                }
                                            }
                                        }
                                    />
                                    <button
                                        class="btn btn-primary"
                                        disabled=move || review_busy.get()
                                        on:click=move |_| {
                                            let val = git_base_ref.get_untracked();
                                            if is_pr_target(&val) {
                                                review_error.set(Some("Use Open Project ▾ → From GitHub / PR URL... to open a PR.".to_string()));
                                            } else {
                                                refresh_local_review();
                                            }
                                        }
                                    >
                                        "Get Diff"
                                    </button>
                                </div>
                                <div class="review-help">
                                    "Accepts "<code>"HEAD"</code>" or a branch/ref (e.g. "<code>"main"</code>", "<code>"origin/main"</code>")."
                                </div>
                            </Show>
                            <Show when=move || active_pr_ref.get().is_some()>
                                <div class="review-refresh-row">
                                    <button
                                        class="btn btn-primary"
                                        disabled=move || review_busy.get()
                                        on:click=move |_| {
                                            if let Some(pr_val) = active_pr_ref.get_untracked() {
                                                refresh_pr_review(pr_val);
                                            }
                                        }
                                    >
                                        {move || if review_busy.get() { "Refreshing PR..." } else { "Refresh PR" }}
                                    </button>
                                </div>
                            </Show>
                        </div>

                        {move || {
                            review_error.get().map(|err| view! {
                                <div class="review-error">{err}</div>
                            })
                        }}

                        {move || {
                            diff_summary.get().map(|d| {
                                let pr_info = d.pr.clone();
                                let repo_label = pr_info
                                    .as_ref()
                                    .map(|p| p.repo_name.clone())
                                    .filter(|s| !s.trim().is_empty())
                                    .unwrap_or_else(|| graph.with(|g| g.root_name.clone()));
                                let branch_label = format!("{} vs {}", d.branch, d.base_ref);
                                view! {
                                    <div class="review-meta">
                                        <div class="review-meta-row">
                                            <span class="review-meta-key">"Repo"</span>
                                            <span class="review-meta-val">{repo_label}</span>
                                        </div>
                                        {pr_info.map(|p| {
                                            let pr_title = format!("{} #{}: {}", p.provider, p.number, p.title);
                                            let pr_url = p.url.clone();
                                            view! {
                                                <div class="review-meta-row">
                                                    <span class="review-meta-key">"PR"</span>
                                                    {if pr_url.is_empty() {
                                                        view! { <span class="review-pr-title">{pr_title}</span> }.into_any()
                                                    } else {
                                                        view! {
                                                            <a
                                                                class="review-pr-title review-pr-link"
                                                                href=pr_url
                                                                target="_blank"
                                                                rel="noreferrer"
                                                            >
                                                                {pr_title}
                                                            </a>
                                                        }.into_any()
                                                    }}
                                                </div>
                                            }
                                        })}
                                        <div class="review-meta-row">
                                            <span class="review-meta-key">"Branch"</span>
                                            <span class="review-meta-val">{branch_label}</span>
                                        </div>
                                        <div class="review-counts">
                                            {format!("{} changed files", d.files.len())}
                                        </div>
                                    </div>
                                }
                            })
                        }}

                        <div class="review-scroll">
                            {move || {
                                let Some(diff) = diff_summary.get() else {
                                    return view! {
                                        <>
                                            <div class="review-section-title">"Changed Files"</div>
                                            <div class="review-empty">"Click Get Diff to inspect changes."</div>
                                        </>
                                    }.into_any();
                                };
                                if diff.files.is_empty() {
                                    return view! {
                                        <>
                                            <div class="review-section-title">"Changed Files"</div>
                                            <div class="review-empty">"Working tree is clean."</div>
                                        </>
                                    }.into_any();
                                }
                                let matched_groups: Vec<Option<String>> = graph.with(|g| {
                                    diff.files.iter().map(|fd| {
                                        g.groups.iter().find(|grp| {
                                            grp.kind == GroupKind::Module
                                                && grp.file_path.as_ref().is_some_and(|fp| {
                                                    paths_match(fp, &g.root_path, fd)
                                                })
                                        }).map(|grp| grp.id.clone())
                                    }).collect()
                                });
                                let viewed_map = viewed_files.get();
                                let mut unviewed_items = Vec::new();
                                let mut viewed_items = Vec::new();
                                for (fd, matched_group) in diff.files.into_iter().zip(matched_groups) {
                                    let fp = file_diff_fingerprint(&fd);
                                    let is_viewed = viewed_map.get(&fd.file_path).is_some_and(|s| s == &fp);
                                    if is_viewed {
                                        viewed_items.push((fd, matched_group, fp, true));
                                    } else {
                                        unviewed_items.push((fd, matched_group, fp, false));
                                    }
                                }
                                let make_row = |(fd, matched_group, fp, is_viewed): (FileDiff, Option<String>, String, bool)| {
                                    let modal_key = fd.file_path.clone();
                                    let row_gid = matched_group.clone();
                                    let path_for_cb = fd.file_path.clone();
                                    view! {
                                        <div
                                            class=move || {
                                                let is_sel = row_gid.as_deref().is_some()
                                                    && selected_id.get().as_deref() == row_gid.as_deref();
                                                match (is_sel, is_viewed) {
                                                    (true, true) => "review-file-row is-selected is-viewed",
                                                    (true, false) => "review-file-row is-selected",
                                                    (false, true) => "review-file-row is-viewed",
                                                    (false, false) => "review-file-row",
                                                }
                                            }
                                            on:click=move |_| {
                                                if let Some(gid) = &matched_group {
                                                    focus_node.run(gid.clone());
                                                }
                                                diff_modal_file.set(Some(modal_key.clone()));
                                            }
                                        >
                                            <input
                                                class="review-viewed-cb"
                                                type="checkbox"
                                                title=if is_viewed { "Unmark viewed" } else { "Mark as viewed" }
                                                prop:checked=is_viewed
                                                on:click=move |ev| ev.stop_propagation()
                                                on:change=move |ev| {
                                                    toggle_file_viewed(
                                                        path_for_cb.clone(),
                                                        fp.clone(),
                                                        event_target_checked(&ev),
                                                    );
                                                }
                                            />
                                            <span class="review-file-name">{fd.file_path}</span>
                                            <span class="review-diff-stat">
                                                <span class="diff-add">{format!("+{}", fd.additions)}</span>
                                                " "
                                                <span class="diff-del">{format!("-{}", fd.deletions)}</span>
                                            </span>
                                        </div>
                                    }
                                };
                                let unviewed_len = unviewed_items.len();
                                let viewed_len = viewed_items.len();
                                view! {
                                    <>
                                        <div class="review-section-title">{format!("Changed Files ({unviewed_len})")}</div>
                                        {if unviewed_len == 0 {
                                            view! { <div class="review-empty">"All changed files viewed."</div> }.into_any()
                                        } else {
                                            unviewed_items.into_iter().map(make_row).collect_view().into_any()
                                        }}
                                        {if viewed_len > 0 {
                                            view! {
                                                <>
                                                    <div class="review-section-title">{format!("Viewed ({viewed_len})")}</div>
                                                    {viewed_items.into_iter().map(make_row).collect_view()}
                                                </>
                                            }.into_any()
                                        } else {
                                            view! { <span /> }.into_any()
                                        }}
                                    </>
                                }.into_any()
                            }}
                        </div>
                    </aside>
            </div>

            <Show when=move || diff_modal_file.get().is_some()>
                {move || {
                    let target = diff_modal_file.get()?;
                    let diff = diff_summary.get()?;
                    let fd = diff.files.into_iter().find(|f| f.file_path == target)?;
                    let status_label = match fd.status {
                        DiffStatus::Added => "Added",
                        DiffStatus::Modified => "Modified",
                        DiffStatus::Deleted => "Deleted",
                    };
                    let modal_fp = file_diff_fingerprint(&fd);
                    let modal_path_chk = fd.file_path.clone();
                    let modal_path_cb = fd.file_path.clone();
                    let modal_fp_chk = modal_fp.clone();
                    let modal_fp_lbl = modal_fp.clone();
                    let modal_path_lbl = fd.file_path.clone();
                    Some(view! {
                        <div class="modal-backdrop" on:click=move |_| diff_modal_file.set(None)>
                            <div
                                class=move || if diff_split_mode.get() { "diff-modal-card is-split" } else { "diff-modal-card" }
                                on:click=move |ev| ev.stop_propagation()
                            >
                                <div class="modal-header">
                                    <div class="diff-modal-title">
                                        <span class="diff-status-badge">{status_label}</span>
                                        <span class="diff-modal-path">{fd.file_path.clone()}</span>
                                        <span class="review-diff-stat">
                                            <span class="diff-add">{format!("+{}", fd.additions)}</span>
                                            " "
                                            <span class="diff-del">{format!("-{}", fd.deletions)}</span>
                                        </span>
                                    </div>
                                    <div class="diff-modal-actions">
                                        <div class="diff-mode-toggle">
                                            <button
                                                class=move || if !diff_split_mode.get() { "btn btn-primary diff-mode-btn" } else { "btn btn-ghost diff-mode-btn" }
                                                on:click=move |_| {
                                                    diff_split_mode.set(false);
                                                    tauri_bridge::local_storage_set(DIFF_SPLIT_MODE_KEY, "false");
                                                }
                                            >
                                                "Unified"
                                            </button>
                                            <button
                                                class=move || if diff_split_mode.get() { "btn btn-primary diff-mode-btn" } else { "btn btn-ghost diff-mode-btn" }
                                                on:click=move |_| {
                                                    diff_split_mode.set(true);
                                                    tauri_bridge::local_storage_set(DIFF_SPLIT_MODE_KEY, "true");
                                                }
                                            >
                                                "Split"
                                            </button>
                                        </div>
                                        <button class="btn btn-ghost" on:click=move |_| diff_modal_file.set(None)>"✕"</button>
                                    </div>
                                </div>
                                <div class="diff-modal-body-wrap">
                                    <div class="diff-modal-body">
                                    {if fd.hunks.is_empty() {
                                        view! { <div class="review-empty">"No diff hunks available for this file."</div> }.into_any()
                                    } else {
                                        fd.hunks.into_iter().map(|h| {
                                            let hdr = format!("@@ -{},{} +{},{} @@", h.old_start, h.old_lines, h.new_start, h.new_lines);
                                            let h_lines = h.lines;
                                            view! {
                                                <div class="diff-modal-hunk">
                                                    <div class="diff-hunk-hdr">{hdr}</div>
                                                    {move || {
                                                        if diff_split_mode.get() {
                                                            let rows = build_split_rows(&h_lines);
                                                            view! {
                                                                <div class="diff-split-table">
                                                                    {rows.into_iter().map(|(left, right)| {
                                                                        let l_cls = match left.as_ref().map(|x| x.kind.as_str()) {
                                                                            Some("del") => "diff-split-cell is-del",
                                                                            Some(_) => "diff-split-cell",
                                                                            None => "diff-split-cell is-empty",
                                                                        };
                                                                        let r_cls = match right.as_ref().map(|x| x.kind.as_str()) {
                                                                            Some("add") => "diff-split-cell is-add",
                                                                            Some(_) => "diff-split-cell",
                                                                            None => "diff-split-cell is-empty",
                                                                        };
                                                                        let l_num = left.as_ref().and_then(|x| x.old_line).map(|n| n.to_string()).unwrap_or_default();
                                                                        let l_sig = left.as_ref().map(|x| if x.kind == "del" { "-" } else { " " }).unwrap_or("");
                                                                        let l_txt = left.map(|x| x.text).unwrap_or_default();
                                                                        let r_num = right.as_ref().and_then(|x| x.new_line).map(|n| n.to_string()).unwrap_or_default();
                                                                        let r_sig = right.as_ref().map(|x| if x.kind == "add" { "+" } else { " " }).unwrap_or("");
                                                                        let r_txt = right.map(|x| x.text).unwrap_or_default();
                                                                        view! {
                                                                            <div class="diff-split-row">
                                                                                <div class=l_cls>
                                                                                    <span class="diff-ln">{l_num}</span>
                                                                                    <span class="diff-sig">{l_sig}</span>
                                                                                    <span class="diff-txt">{l_txt}</span>
                                                                                </div>
                                                                                <div class=r_cls>
                                                                                    <span class="diff-ln">{r_num}</span>
                                                                                    <span class="diff-sig">{r_sig}</span>
                                                                                    <span class="diff-txt">{r_txt}</span>
                                                                                </div>
                                                                            </div>
                                                                        }
                                                                    }).collect_view()}
                                                                </div>
                                                            }.into_any()
                                                        } else {
                                                            view! {
                                                                <div>
                                                                    {h_lines.iter().cloned().map(|ln| {
                                                                        let cls = match ln.kind.as_str() {
                                                                            "add" => "diff-line is-add",
                                                                            "del" => "diff-line is-del",
                                                                            _ => "diff-line",
                                                                        };
                                                                        let prefix = match ln.kind.as_str() {
                                                                            "add" => "+",
                                                                            "del" => "-",
                                                                            _ => " ",
                                                                        };
                                                                        let old_ln = ln.old_line.map(|n| n.to_string()).unwrap_or_default();
                                                                        let new_ln = ln.new_line.map(|n| n.to_string()).unwrap_or_default();
                                                                        view! {
                                                                            <div class=cls>
                                                                                <span class="diff-ln">{old_ln}</span>
                                                                                <span class="diff-ln">{new_ln}</span>
                                                                                <span class="diff-sig">{prefix}</span>
                                                                                <span class="diff-txt">{ln.text}</span>
                                                                            </div>
                                                                        }
                                                                    }).collect_view()}
                                                                </div>
                                                            }.into_any()
                                                        }
                                                    }}
                                                </div>
                                            }
                                        }).collect_view().into_any()
                                    }}
                                    </div>
                                    <div
                                        class="diff-scrollbar-gutter"
                                        title="Click to jump to change"
                                        on:click=move |ev| {
                                            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                                                if let (Ok(Some(gutter)), Ok(Some(body))) = (
                                                    doc.query_selector(".diff-scrollbar-gutter"),
                                                    doc.query_selector(".diff-modal-body"),
                                                ) {
                                                    let rect = gutter.get_bounding_client_rect();
                                                    let ratio = ((ev.client_y() as f64 - rect.top())
                                                        / rect.height().max(1.0))
                                                    .clamp(0.0, 1.0);
                                                    let scroll_h = body.scroll_height() as f64;
                                                    let client_h = body.client_height() as f64;
                                                    let target = (ratio * scroll_h - client_h * 0.5)
                                                        .clamp(0.0, (scroll_h - client_h).max(0.0));
                                                    body.set_scroll_top(target.round() as i32);
                                                }
                                            }
                                        }
                                    >
                                        {move || {
                                            diff_gutter_markers
                                                .get()
                                                .into_iter()
                                                .map(|m| {
                                                    let cls = match m.kind {
                                                        "add" => "diff-gutter-marker is-add",
                                                        "del" => "diff-gutter-marker is-del",
                                                        _ => "diff-gutter-marker is-both",
                                                    };
                                                    let style = format!(
                                                        "top: {:.3}%; height: {:.3}%;",
                                                        m.top_pct, m.height_pct
                                                    );
                                                    view! { <div class=cls style=style></div> }
                                                })
                                                .collect_view()
                                        }}
                                    </div>
                                </div>
                                <div class="diff-modal-footer">
                                    <button
                                        class=move || {
                                            let is_v = viewed_files
                                                .get()
                                                .get(&modal_path_chk)
                                                .is_some_and(|s| s == &modal_fp_chk);
                                            if is_v {
                                                "btn diff-viewed-cta is-viewed"
                                            } else {
                                                "btn btn-primary diff-viewed-cta"
                                            }
                                        }
                                        on:click=move |_| {
                                            let currently_viewed = viewed_files
                                                .get_untracked()
                                                .get(&modal_path_cb)
                                                .is_some_and(|s| s == &modal_fp);
                                            toggle_file_viewed(
                                                modal_path_cb.clone(),
                                                modal_fp.clone(),
                                                !currently_viewed,
                                            );
                                        }
                                    >
                                        {move || {
                                            let is_v = viewed_files
                                                .get()
                                                .get(&modal_path_lbl)
                                                .is_some_and(|s| s == &modal_fp_lbl);
                                            if is_v {
                                                "✓ Viewed"
                                            } else {
                                                "✓ Mark as Viewed"
                                            }
                                        }}
                                    </button>
                                </div>
                            </div>
                        </div>
                    })
                }}
            </Show>

            <Show when=move || settings_open.get()>
                <div class="modal-backdrop" on:click=move |_| settings_open.set(false)>
                    <div
                        class="modal-card"
                        on:click=move |ev| ev.stop_propagation()
                    >
                        <div class="modal-header">
                            <span>"Settings"</span>
                            <button class="btn btn-ghost" on:click=move |_| settings_open.set(false)>"✕"</button>
                        </div>
                        <div class="modal-body">
                            <label class="modal-label">"Editor CLI command"</label>
                            <input
                                class="modal-input"
                                type="text"
                                placeholder="e.g. code, cursor, zed, subl, nvim"
                                prop:value=move || settings_input.get()
                                on:input=move |ev| settings_input.set(event_target_value(&ev))
                            />
                            <div class="modal-presets">
                                {["code", "cursor", "zed", "subl", "nvim"].into_iter().map(|preset| {
                                    view! {
                                        <button
                                            class="preset-chip"
                                            on:click=move |_| {
                                                settings_input.set(preset.to_string());
                                                open_in_terminal_input.set(preset == "nvim");
                                            }
                                        >
                                            {preset}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
                            <label class="modal-checkbox">
                                <input
                                    type="checkbox"
                                    prop:checked=move || open_in_terminal_input.get()
                                    on:change=move |ev| open_in_terminal_input.set(event_target_checked(&ev))
                                />
                                <span>"Open in terminal (for nvim, vim, helix, etc.)"</span>
                            </label>
                            <p class="modal-help">
                                "Tip: use "<code>"{file}"</code>" placeholder if your editor needs custom flags (e.g. "<code>"code -r {file}"</code>")."
                            </p>
                            <label class="modal-label">"GitHub / GitLab Token (optional)"</label>
                            <input
                                class="modal-input"
                                type="password"
                                placeholder="Leave blank to auto-use gh/glab or SSH"
                                prop:value=move || token_input.get()
                                on:input=move |ev| token_input.set(event_target_value(&ev))
                            />
                            <p class="modal-help">
                                "Only needed for private repos when "<code>"gh"</code>"/"<code>"glab"</code>" CLI or SSH isn't configured."
                            </p>
                        </div>
                        <div class="modal-footer">
                            <button class="btn btn-ghost" on:click=move |_| settings_open.set(false)>"Cancel"</button>
                            <button
                                class="btn btn-primary"
                                on:click=move |_| {
                                    let val = settings_input.get_untracked().trim().to_string();
                                    let in_term = open_in_terminal_input.get_untracked();
                                    let tok = token_input.get_untracked().trim().to_string();
                                    editor_command.set(val.clone());
                                    save_editor_command(&val);
                                    open_in_terminal.set(in_term);
                                    tauri_bridge::local_storage_set(
                                        OPEN_IN_TERMINAL_KEY,
                                        if in_term { "true" } else { "false" },
                                    );
                                    git_token.set(tok.clone());
                                    tauri_bridge::local_storage_set("vcode.gitToken", &tok);
                                    persist_project_settings();
                                    settings_open.set(false);
                                    if !val.is_empty() {
                                        if let Some(fp) = pending_editor_file.get_untracked() {
                                            pending_editor_file.set(None);
                                            handle_open_in_editor.run(fp);
                                        }
                                    }
                                }
                            >
                                "Save"
                            </button>
                        </div>
                    </div>
                </div>
            </Show>

            <Show when=move || open_url_modal.get()>
                <div class="modal-backdrop" on:click=move |_| open_url_modal.set(false)>
                    <div class="modal-card" on:click=move |ev| ev.stop_propagation()>
                        <div class="modal-header">
                            <h3>"Open from GitHub / PR URL"</h3>
                            <button class="btn btn-ghost" on:click=move |_| open_url_modal.set(false)>"✕"</button>
                        </div>
                        <div class="modal-body">
                            <label class="modal-label">"GitHub repository URL, owner/repo, or PR / MR URL"</label>
                            <input
                                class="modal-input"
                                type="text"
                                placeholder="https://github.com/owner/repo or .../pull/123"
                                prop:value=move || open_url_input.get()
                                on:input=move |ev| open_url_input.set(event_target_value(&ev))
                                on:keydown=move |ev| {
                                    if ev.key() == "Enter" {
                                        submit_open_url();
                                    } else if ev.key() == "Escape" {
                                        open_url_modal.set(false);
                                    }
                                }
                            />
                            {move || {
                                open_url_error.get().map(|err| view! {
                                    <div class="review-error">{err}</div>
                                })
                            }}
                        </div>
                        <div class="modal-footer">
                            <button class="btn btn-ghost" on:click=move |_| open_url_modal.set(false)>"Cancel"</button>
                            <button class="btn btn-primary" on:click=move |_| submit_open_url()>"Open"</button>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{file_diff_fingerprint, merge_gutter_markers, paths_match, DiffGutterMarker};
    use crate::types::{DiffHunk, DiffLine, DiffStatus, FileDiff};

    #[test]
    fn matches_exact_relative_paths_without_basename_collisions() {
        let fd = FileDiff {
            file_path: "utils.py".to_string(),
            abs_path: "/repo/utils.py".to_string(),
            status: DiffStatus::Modified,
            additions: 1,
            deletions: 0,
            hunks: vec![],
        };
        assert!(paths_match("utils.py", "/repo", &fd));
        assert!(paths_match("/repo/utils.py", "/repo", &fd));
        assert!(!paths_match("src/utils.py", "/repo", &fd));
        assert!(!paths_match("/repo/src/utils.py", "/repo", &fd));
    }

    #[test]
    fn diff_fingerprint_changes_when_new_changes_are_added() {
        let base = FileDiff {
            file_path: "src/app.rs".to_string(),
            abs_path: "/repo/src/app.rs".to_string(),
            status: DiffStatus::Modified,
            additions: 1,
            deletions: 0,
            hunks: vec![DiffHunk {
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 2,
                lines: vec![DiffLine {
                    kind: "add".to_string(),
                    old_line: None,
                    new_line: Some(2),
                    text: "let x = 1;".to_string(),
                }],
            }],
        };
        let mut updated = base.clone();
        updated.additions = 2;
        updated.hunks[0].lines.push(DiffLine {
            kind: "add".to_string(),
            old_line: None,
            new_line: Some(3),
            text: "let y = 2;".to_string(),
        });
        assert_eq!(file_diff_fingerprint(&base), file_diff_fingerprint(&base));
        assert_ne!(file_diff_fingerprint(&base), file_diff_fingerprint(&updated));
    }

    #[test]
    fn merges_adjacent_gutter_markers_by_kind() {
        let raw = vec![
            DiffGutterMarker { top_pct: 10.0, height_pct: 1.0, kind: "add" },
            DiffGutterMarker { top_pct: 11.1, height_pct: 1.5, kind: "add" },
            DiffGutterMarker { top_pct: 12.6, height_pct: 1.0, kind: "del" },
            DiffGutterMarker { top_pct: 40.0, height_pct: 2.0, kind: "del" },
        ];
        let merged = merge_gutter_markers(raw);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].kind, "add");
        assert!((merged[0].top_pct - 10.0).abs() < 1e-6);
        assert!((merged[0].height_pct - 2.6).abs() < 1e-6);
        assert_eq!(merged[1].kind, "del");
        assert_eq!(merged[2].kind, "del");
    }

    #[test]
    fn lerps_viewport_with_ease_out_cubic() {
        let a = crate::types::Viewport { x: 0.0, y: 0.0, scale: 1.0 };
        let b = crate::types::Viewport { x: 100.0, y: 200.0, scale: 2.0 };
        let mid = super::lerp_viewport(a, b, 0.5);
        // ease_out_cubic(0.5) = 0.875
        assert!((mid.x - 87.5).abs() < 1e-6);
        assert!((mid.y - 175.0).abs() < 1e-6);
        assert!((mid.scale - 1.875).abs() < 1e-6);
        let end = super::lerp_viewport(a, b, 1.0);
        assert_eq!(end, b);
    }
}
