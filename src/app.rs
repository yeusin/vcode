use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::{closure::Closure, JsCast};

use crate::canvas::{CanvasView, ReviewOverlay};
use crate::layout::compute_layout;
use crate::sample::sample_project;
use crate::tauri_bridge;
use crate::types::{
    AnalyzeProgress, DiffStatus, FunctionNode, GitDiffSummary, GroupKind, GroupNode, LodLevel,
    ProjectGraph, ProjectSettings, RecentProject, Viewport,
};

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

const RECENT_PROJECTS_KEY: &str = "vcode.recentProjects";
const EDITOR_COMMAND_KEY: &str = "vcode.editorCommand";
const PR_REFRESH_PERIODS: &[(&str, u64)] = &[
    ("30s", 30_000),
    ("1m", 60_000),
    ("3m", 180_000),
    ("5m", 300_000),
    ("15m", 900_000),
    ("30m", 1_800_000),
    ("1h", 3_600_000),
];

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

fn push_recent_project(path: &str, name: &str) -> Vec<RecentProject> {
    let mut list = load_recent_projects();
    list.retain(|p| p.path != path);
    list.insert(
        0,
        RecentProject {
            path: path.to_string(),
            name: name.to_string(),
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
    let selected_id = RwSignal::new(Option::<String>::None);
    let hovered_id = RwSignal::new(Option::<String>::None);
    let sidebar_open = RwSignal::new(true);
    let search_query = RwSignal::new(String::new());
    let collapsed_ids = RwSignal::new(HashSet::<String>::new());

    let editor_command = RwSignal::new(load_editor_command());
    let settings_open = RwSignal::new(false);
    let settings_input = RwSignal::new(load_editor_command());
    let pending_editor_file = RwSignal::new(Option::<String>::None);

    // Code Review & PR state
    let review_changed_only = RwSignal::new(false);
    let diff_summary = RwSignal::new(Option::<GitDiffSummary>::None);
    let git_base_ref = RwSignal::new("HEAD".to_string());
    let pr_input = RwSignal::new(String::new());
    let active_pr_ref = RwSignal::new(Option::<String>::None);
    let pr_refresh_ms = RwSignal::new(Option::<u64>::None);
    let last_pr_refresh_ms = RwSignal::new(0.0_f64);
    let git_token = RwSignal::new(
        tauri_bridge::local_storage_get("vcode.gitToken").unwrap_or_default(),
    );
    let token_input = RwSignal::new(git_token.get_untracked());
    let review_busy = RwSignal::new(false);
    let review_error = RwSignal::new(Option::<String>::None);

    let layout = Memo::new(move |_| Arc::new(graph.with(|g| compute_layout(g))));

    let review_overlay = Memo::new(move |_| {
        let diff_opt = diff_summary.get();
        let is_pr_loaded = diff_opt
            .as_ref()
            .and_then(|d| d.pr.as_ref())
            .is_some();

        let mut changed_ids = HashSet::new();
        let mut diff_stats: HashMap<String, (usize, usize)> = HashMap::new();
        let mut blast_radius_ids = HashSet::new();

        graph.with(|g| {
            if let Some(diff) = &diff_opt {
                for fd in &diff.files {
                    for grp in &g.groups {
                        if grp.kind == GroupKind::Module {
                            if let Some(fp) = &grp.file_path {
                                if fp == &fd.abs_path
                                    || fp.ends_with(&format!("/{}", fd.file_path))
                                    || fp == &fd.file_path
                                {
                                    changed_ids.insert(grp.id.clone());
                                    diff_stats
                                        .insert(grp.id.clone(), (fd.additions, fd.deletions));
                                }
                            }
                        }
                    }
                    for fnode in &g.functions {
                        let matches_file = fnode.file_path == fd.abs_path
                            || fnode.file_path.ends_with(&format!("/{}", fd.file_path))
                            || fnode.file_path == fd.file_path;
                        if !matches_file {
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

            for e in &g.edges {
                if changed_ids.contains(&e.source_id) && !changed_ids.contains(&e.target_id) {
                    blast_radius_ids.insert(e.target_id.clone());
                }
                if changed_ids.contains(&e.target_id) && !changed_ids.contains(&e.source_id) {
                    blast_radius_ids.insert(e.source_id.clone());
                }
            }
        });

        ReviewOverlay {
            active: true,
            changed_only: is_pr_loaded || review_changed_only.get(),
            changed_ids,
            blast_radius_ids,
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

    let refresh_pr_review = move |pr_val: String| {
        let proj = graph.get_untracked().root_path;
        let tok = git_token.get_untracked();
        if proj.is_empty()
            || proj.starts_with("sample://")
            || pr_val.trim().is_empty()
            || !tauri_bridge::is_tauri_available()
            || review_busy.get_untracked()
        {
            return;
        }
        review_busy.set(true);
        review_error.set(None);
        last_pr_refresh_ms.set(js_sys::Date::now());
        spawn_local(async move {
            let tok_opt = if tok.trim().is_empty() {
                None
            } else {
                Some(tok.as_str())
            };
            match tauri_bridge::fetch_pr_review(&proj, &pr_val, tok_opt).await {
                Ok(pr_diff) => {
                    active_pr_ref.set(Some(pr_val));
                    diff_summary.set(Some(pr_diff));
                    review_changed_only.set(true);
                }
                Err(e) => review_error.set(Some(e)),
            }
            last_pr_refresh_ms.set(js_sys::Date::now());
            review_busy.set(false);
        });
    };

    if let Some(win) = web_sys::window() {
        let cb = Closure::<dyn FnMut()>::new(move || {
            if let (Some(pr_val), Some(period_ms)) =
                (active_pr_ref.get_untracked(), pr_refresh_ms.get_untracked())
            {
                if js_sys::Date::now() - last_pr_refresh_ms.get_untracked() >= period_ms as f64 {
                    refresh_pr_review(pr_val);
                }
            }
        });
        let _ = win.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            1_000,
        );
        cb.forget();
    }

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
        };
        let path = g.root_path.clone();
        spawn_local(async move {
            let _ = tauri_bridge::save_project_settings(&path, &settings).await;
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
        viewport.set(Viewport { x, y, scale });
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
        viewport.set(Viewport {
            x: vw / 2.0 - cx * scale,
            y: vh / 2.0 - cy * scale,
            scale,
        });
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
                    }

                    let recent = push_recent_project(&new_graph.root_path, &new_graph.root_name);
                    recent_projects.set(recent);
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

    // Startup: auto-open last project + subscribe to live project-updated events
    Effect::new(move |_| {
        if tauri_bridge::is_tauri_available() {
            spawn_local(async move {
                let _ = tauri_bridge::listen_project_updated(move |updated| {
                    graph.set(updated);
                })
                .await;
            });
            spawn_local(async move {
                let _ = tauri_bridge::listen_analyze_progress(move |progress| {
                    analyze_progress.set(Some(progress));
                })
                .await;
            });
            let recent = load_recent_projects();
            if let Some(first) = recent.first() {
                open_project(first.path.clone());
            } else {
                fit_to_view();
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

    let handle_open_folder = move |_| {
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

    let handle_open_in_editor = Callback::new(move |file_path: String| {
        let cmd = editor_command.get_untracked();
        if cmd.trim().is_empty() {
            pending_editor_file.set(Some(file_path));
            settings_input.set(String::new());
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
        spawn_local(async move {
            if let Err(err) = tauri_bridge::open_in_editor(&cmd, &file_path, proj_opt.as_deref()).await {
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
        <div class="app-shell">
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
                    <button class="btn btn-primary" on:click=handle_open_folder>
                        "Open Project"
                    </button>
                    <div class="recent-chips">
                        {move || {
                            recent_projects.get().into_iter().map(|rp| {
                                let p = rp.path.clone();
                                let title = rp.path.clone();
                                view! {
                                    <button
                                        class="recent-chip"
                                        title=title
                                        on:click=move |_| open_project(p.clone())
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
                        title="Dim unchanged code and highlight modified + blast-radius nodes (always on when a PR is loaded)"
                        on:click=move |_| {
                            if diff_summary.get_untracked().and_then(|d| d.pr).is_none() {
                                review_changed_only.update(|v| *v = !*v);
                            }
                        }
                    >
                        "Changed + Impact"
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
                    <button
                        class="btn btn-ghost"
                        title="Settings"
                        on:click=move |_| {
                            settings_input.set(editor_command.get_untracked());
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
                <Show when=move || sidebar_open.get()>
                    <aside class="sidebar">
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
                </Show>

                <main class="canvas-stage">
                    {move || {
                        if let Some(label) = loading_label.get() {
                            let progress = analyze_progress.get();
                            view! {
                                <div class="canvas-loading">
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
                            <div class="review-row">
                                <input
                                    class="review-input"
                                    type="text"
                                    placeholder="Base ref (HEAD, main...)"
                                    prop:value=move || git_base_ref.get()
                                    on:input=move |ev| git_base_ref.set(event_target_value(&ev))
                                />
                                <button
                                    class="btn btn-ghost"
                                    disabled=move || review_busy.get()
                                    on:click=move |_| refresh_local_review()
                                >
                                    "Git Diff"
                                </button>
                            </div>
                            <div class="review-row">
                                <input
                                    class="review-input"
                                    type="text"
                                    placeholder="GitHub/GitLab PR URL or #num"
                                    prop:value=move || pr_input.get()
                                    on:input=move |ev| pr_input.set(event_target_value(&ev))
                                />
                                <button
                                    class="btn btn-primary"
                                    disabled=move || review_busy.get()
                                    on:click=move |_| refresh_pr_review(pr_input.get_untracked())
                                >
                                    "Load PR"
                                </button>
                            </div>
                            <div class="review-refresh-row">
                                <span class="review-lbl">"Auto"</span>
                                {PR_REFRESH_PERIODS.iter().map(|&(label, ms)| {
                                    view! {
                                        <button
                                            class=move || {
                                                if pr_refresh_ms.get() == Some(ms) {
                                                    "btn btn-primary review-period-btn"
                                                } else {
                                                    "btn btn-ghost review-period-btn"
                                                }
                                            }
                                            on:click=move |_| {
                                                if pr_refresh_ms.get_untracked() == Some(ms) {
                                                    pr_refresh_ms.set(None);
                                                } else {
                                                    last_pr_refresh_ms.set(js_sys::Date::now());
                                                    pr_refresh_ms.set(Some(ms));
                                                }
                                            }
                                        >
                                            {label}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
                        </div>

                        {move || {
                            review_error.get().map(|err| view! {
                                <div class="review-error">{err}</div>
                            })
                        }}

                        {move || {
                            diff_summary.get().map(|d| {
                                let pr_badge = d.pr.as_ref().map(|p| format!("{} #{}: {}", p.provider, p.number, p.title));
                                view! {
                                    <div class="review-meta">
                                        <div>{format!("Branch: {}  vs  {}", d.branch, d.base_ref)}</div>
                                        {pr_badge.map(|t| view! { <div class="review-pr-title">{t}</div> })}
                                        <div class="review-counts">
                                            {move || {
                                                let auto_label = if d.pr.is_some() {
                                                    pr_refresh_ms.get().and_then(|ms| {
                                                        PR_REFRESH_PERIODS.iter().find(|&&(_, p)| p == ms).map(|&(lbl, _)| format!(" · Auto-refresh {lbl}"))
                                                    }).unwrap_or_default()
                                                } else {
                                                    String::new()
                                                };
                                                format!(
                                                    "{} changed files · {} impacted nodes{}",
                                                    d.files.len(),
                                                    review_overlay.get().blast_radius_ids.len(),
                                                    auto_label
                                                )
                                            }}
                                        </div>
                                    </div>
                                }
                            })
                        }}

                        <div class="review-scroll">
                            <div class="review-section-title">"Changed Files"</div>
                            {move || {
                                let Some(diff) = diff_summary.get() else {
                                    return view! { <div class="review-empty">"Click Git Diff or Load PR to inspect changes."</div> }.into_any();
                                };
                                if diff.files.is_empty() {
                                    return view! { <div class="review-empty">"Working tree is clean."</div> }.into_any();
                                }
                                let g = graph.get();
                                diff.files.into_iter().map(|fd| {
                                    let rel = fd.file_path.clone();
                                    let abs = fd.abs_path.clone();
                                    let matched_group = g.groups.iter().find(|grp| {
                                        grp.kind == GroupKind::Module
                                            && grp.file_path.as_ref().is_some_and(|fp| {
                                                fp == &abs || fp.ends_with(&format!("/{rel}")) || fp == &rel
                                            })
                                    }).map(|grp| grp.id.clone());
                                    view! {
                                        <div
                                            class="review-file-row"
                                            on:click=move |_| {
                                                if let Some(gid) = &matched_group {
                                                    selected_id.set(Some(gid.clone()));
                                                    focus_node.run(gid.clone());
                                                }
                                            }
                                        >
                                            <span class="review-file-name">{fd.file_path}</span>
                                            <span class="review-diff-stat">
                                                <span class="diff-add">{format!("+{}", fd.additions)}</span>
                                                " "
                                                <span class="diff-del">{format!("-{}", fd.deletions)}</span>
                                            </span>
                                        </div>
                                    }
                                }).collect_view().into_any()
                            }}

                            <div class="review-section-title">"Diff Hunks"</div>
                            {move || {
                                let Some(diff) = diff_summary.get() else {
                                    return view! { <span /> }.into_any();
                                };
                                let sel = selected_id.get();
                                let g = graph.get();
                                let target_file = sel.as_deref().and_then(|sid| {
                                    g.functions
                                        .iter()
                                        .find(|f| f.id == sid)
                                        .map(|f| f.file_path.clone())
                                        .or_else(|| {
                                            g.groups
                                                .iter()
                                                .find(|grp| grp.id == sid)
                                                .and_then(|grp| grp.file_path.clone())
                                        })
                                });
                                let active_fd = target_file
                                    .and_then(|tf| {
                                        diff.files.iter().find(|fd| {
                                            fd.abs_path == tf
                                                || tf.ends_with(&format!("/{}", fd.file_path))
                                                || fd.file_path == tf
                                        }).cloned()
                                    })
                                    .or_else(|| diff.files.first().cloned());

                                let Some(fd) = active_fd else {
                                    return view! { <div class="review-empty">"No diff hunks."</div> }.into_any();
                                };

                                view! {
                                    <div class="diff-hunk-box">
                                        <div class="diff-file-hdr">{fd.file_path}</div>
                                        {fd.hunks.into_iter().map(|h| view! {
                                            <div class="diff-hunk">
                                                <div class="diff-hunk-hdr">{format!("@@ -{},{} +{},{} @@", h.old_start, h.old_lines, h.new_start, h.new_lines)}</div>
                                                {h.lines.into_iter().map(|ln| {
                                                    let lno = ln.new_line.or(ln.old_line).unwrap_or(1);
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
                                                    view! {
                                                        <div class=cls>
                                                            <span class="diff-ln">{lno}</span>
                                                            <span class="diff-sig">{prefix}</span>
                                                            <span class="diff-txt">{ln.text}</span>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            }}
                        </div>
                    </aside>
            </div>

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
                                placeholder="e.g. code, cursor, zed, subl"
                                prop:value=move || settings_input.get()
                                on:input=move |ev| settings_input.set(event_target_value(&ev))
                            />
                            <div class="modal-presets">
                                {["code", "cursor", "zed", "subl"].into_iter().map(|preset| {
                                    view! {
                                        <button
                                            class="preset-chip"
                                            on:click=move |_| settings_input.set(preset.to_string())
                                        >
                                            {preset}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
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
                                    let tok = token_input.get_untracked().trim().to_string();
                                    editor_command.set(val.clone());
                                    save_editor_command(&val);
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
        </div>
    }
}
