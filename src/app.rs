use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::canvas::CanvasView;
use crate::layout::compute_layout;
use crate::sample::sample_project;
use crate::tauri_bridge;
use crate::types::{
    EdgeVisibility, FunctionNode, GroupKind, GroupNode, LodLevel, ProjectGraph, ProjectSettings,
    RecentProject, Viewport,
};

const RECENT_PROJECTS_KEY: &str = "vcode.recentProjects";
const EDITOR_COMMAND_KEY: &str = "vcode.editorCommand";

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
    kind_tag: &'static str,
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
                        kind_tag: "fn",
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

        let kind_tag = match g.kind {
            GroupKind::Directory => "dir",
            GroupKind::Module => "file",
            GroupKind::Class => "class",
        };
        Some(TreeEntry {
            id: g.id.clone(),
            label: g.name.clone(),
            kind_tag,
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
    let error_banner = RwSignal::new(Option::<String>::None);
    let recent_projects = RwSignal::new(initial_recent);

    let viewport = RwSignal::new(Viewport::default());
    let selected_id = RwSignal::new(Option::<String>::None);
    let hovered_id = RwSignal::new(Option::<String>::None);
    let sidebar_open = RwSignal::new(true);
    let search_query = RwSignal::new(String::new());
    let collapsed_ids = RwSignal::new(HashSet::<String>::new());

    let show_imports = RwSignal::new(true);
    let show_calls = RwSignal::new(true);
    let hidden_import_groups = RwSignal::new(HashSet::<String>::new());
    let hidden_call_groups = RwSignal::new(HashSet::<String>::new());

    let editor_command = RwSignal::new(load_editor_command());
    let settings_open = RwSignal::new(false);
    let settings_input = RwSignal::new(load_editor_command());
    let pending_editor_file = RwSignal::new(Option::<String>::None);

    let layout = Memo::new(move |_| compute_layout(&graph.get()));
    let edge_vis = Memo::new(move |_| EdgeVisibility {
        show_imports: show_imports.get(),
        show_calls: show_calls.get(),
        hidden_import_group_ids: hidden_import_groups.get(),
        hidden_call_group_ids: hidden_call_groups.get(),
    });

    let persist_project_settings = move || {
        let g = graph.get_untracked();
        if g.root_path.is_empty() || g.root_path.starts_with("sample://") || !tauri_bridge::is_tauri_available() {
            return;
        }
        let cmd = editor_command.get_untracked();
        let settings = ProjectSettings {
            show_import_edges: show_imports.get_untracked(),
            show_call_edges: show_calls.get_untracked(),
            hidden_import_group_ids: hidden_import_groups.get_untracked().into_iter().collect(),
            hidden_call_group_ids: hidden_call_groups.get_untracked().into_iter().collect(),
            editor_command: if cmd.trim().is_empty() { None } else { Some(cmd) },
        };
        let path = g.root_path.clone();
        spawn_local(async move {
            let _ = tauri_bridge::save_project_settings(&path, &settings).await;
        });
    };

    let fit_to_view = move || {
        let l = layout.get_untracked();
        if l.bounds.width <= 0.0 || l.bounds.height <= 0.0 {
            return;
        }
        let (vw, vh) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(".canvas-viewport").ok().flatten())
            .map(|el| {
                let r = el.get_bounding_client_rect();
                (r.width(), r.height())
            })
            .unwrap_or((1000.0, 720.0));
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let scale = ((vw - 64.0) / l.bounds.width)
            .min((vh - 64.0) / l.bounds.height)
            .clamp(0.1, 1.0);
        let x = (vw - l.bounds.width * scale) / 2.0;
        let y = (vh - l.bounds.height * scale) / 2.0;
        viewport.set(Viewport { x, y, scale });
    };

    let focus_node = Callback::new(move |id: String| {
        let l = layout.get_untracked();
        let Some(rect) = l.node_rects.get(&id).copied() else {
            return;
        };
        let (vw, vh) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(".canvas-viewport").ok().flatten())
            .map(|el| {
                let r = el.get_bounding_client_rect();
                (r.width(), r.height())
            })
            .unwrap_or((1000.0, 720.0));
        let scale = ((vw * 0.72) / rect.width)
            .min((vh * 0.72) / rect.height)
            .clamp(0.3, 1.15);
        let cx = rect.x + rect.width / 2.0;
        let cy = rect.y + rect.height / 2.0;
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
        error_banner.set(None);
        selected_id.set(None);
        hovered_id.set(None);

        spawn_local(async move {
            match tauri_bridge::analyze_project(&path).await {
                Ok(new_graph) => {
                    let saved = tauri_bridge::load_project_settings(&path).await.ok().flatten();
                    if let Some(s) = saved {
                        show_imports.set(s.show_import_edges);
                        show_calls.set(s.show_call_edges);
                        hidden_import_groups.set(s.hidden_import_group_ids.into_iter().collect());
                        hidden_call_groups.set(s.hidden_call_group_ids.into_iter().collect());
                        if let Some(cmd) = s.editor_command {
                            if !cmd.trim().is_empty() {
                                editor_command.set(cmd.clone());
                                save_editor_command(&cmd);
                            }
                        }
                    } else {
                        show_imports.set(true);
                        show_calls.set(true);
                        hidden_import_groups.set(HashSet::new());
                        hidden_call_groups.set(HashSet::new());
                    }

                    let recent = push_recent_project(&new_graph.root_path, &new_graph.root_name);
                    recent_projects.set(recent);
                    graph.set(new_graph);
                    loading_label.set(None);
                    fit_to_view();
                    let _ = tauri_bridge::watch_project(&path).await;
                }
                Err(err) => {
                    loading_label.set(None);
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

    let toggle_group_import = Callback::new(move |group_id: String| {
        hidden_import_groups.update(|set| {
            if !set.remove(&group_id) {
                set.insert(group_id);
            }
        });
        persist_project_settings();
    });

    let toggle_group_call = Callback::new(move |group_id: String| {
        hidden_call_groups.update(|set| {
            if !set.remove(&group_id) {
                set.insert(group_id);
            }
        });
        persist_project_settings();
    });

    let flat_tree = Memo::new(move |_| {
        let g = graph.get();
        let q = search_query.get();
        let collapsed = collapsed_ids.get();
        let tree = build_tree(&g, &q);
        let mut flat = Vec::new();
        flatten_tree(&tree, &collapsed, &mut flat);
        flat.into_iter().cloned().collect::<Vec<_>>()
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
                        class=move || if show_imports.get() { "toggle-pill import-pill is-active" } else { "toggle-pill import-pill" }
                        on:click=move |_| {
                            show_imports.update(|v| *v = !*v);
                            persist_project_settings();
                        }
                    >
                        "Imports"
                    </button>
                    <button
                        class=move || if show_calls.get() { "toggle-pill call-pill is-active" } else { "toggle-pill call-pill" }
                        on:click=move |_| {
                            show_calls.update(|v| *v = !*v);
                            persist_project_settings();
                        }
                    >
                        "Calls"
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
                        <div class="sidebar-header">
                            <span class="sidebar-title">"Project Structure"</span>
                            <span class="sidebar-stats">
                                {move || {
                                    let s = graph.get().stats;
                                    format!("{} files · {} cls · {} fns", s.total_files, s.total_classes, s.total_functions)
                                }}
                            </span>
                        </div>
                        <div class="sidebar-search">
                            <input
                                type="text"
                                placeholder="Filter files, classes, functions..."
                                prop:value=move || search_query.get()
                                on:input=move |ev| search_query.set(event_target_value(&ev))
                            />
                        </div>
                        <div class="tree-list">
                            {move || {
                                flat_tree.get().into_iter().map(|entry| {
                                    let id = entry.id.clone();
                                    let id_sel = entry.id.clone();
                                    let id_dbl = entry.id.clone();
                                    let id_col = entry.id.clone();
                                    let id_imp = entry.id.clone();
                                    let id_call = entry.id.clone();
                                    let id_imp_cls = entry.id.clone();
                                    let id_call_cls = entry.id.clone();
                                    let has_children = !entry.children.is_empty();
                                    let is_group = entry.is_group;
                                    let indent = format!("padding-left: {}px;", 10 + entry.depth * 14);
                                    view! {
                                        <div
                                            class=move || if selected_id.get().as_deref() == Some(id.as_str()) {
                                                "tree-row is-selected"
                                            } else {
                                                "tree-row"
                                            }
                                            style=indent
                                            attr:data-node-id=entry.id.clone()
                                            on:click=move |_| selected_id.set(Some(id_sel.clone()))
                                            on:dblclick=move |_| focus_node.run(id_dbl.clone())
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
                                                        {move || if collapsed_ids.get().contains(&id_col) { "▸" } else { "▾" }}
                                                    </button>
                                                }.into_any()
                                            } else {
                                                view! { <span class="tree-caret-spacer" /> }.into_any()
                                            }}
                                            <span class=format!("tree-tag tree-tag-{}", entry.kind_tag)>{entry.kind_tag}</span>
                                            <span class="tree-label">{entry.label}</span>
                                            {if is_group {
                                                view! {
                                                    <span class="tree-actions">
                                                        <button
                                                            class=move || if hidden_import_groups.get().contains(&id_imp_cls) {
                                                                "tree-edge-btn is-hidden"
                                                            } else {
                                                                "tree-edge-btn is-import"
                                                            }
                                                            title="Toggle import graph"
                                                            on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                toggle_group_import.run(id_imp.clone());
                                                            }
                                                        >
                                                            "I"
                                                        </button>
                                                        <button
                                                            class=move || if hidden_call_groups.get().contains(&id_call_cls) {
                                                                "tree-edge-btn is-hidden"
                                                            } else {
                                                                "tree-edge-btn is-call"
                                                            }
                                                            title="Toggle call graph"
                                                            on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                toggle_group_call.run(id_call.clone());
                                                            }
                                                        >
                                                            "C"
                                                        </button>
                                                    </span>
                                                }.into_any()
                                            } else {
                                                view! { <span /> }.into_any()
                                            }}
                                        </div>
                                    }
                                }).collect_view()
                            }}
                        </div>

                        {move || {
                            let errs = graph.get().parse_errors;
                            if errs.is_empty() {
                                view! { <span /> }.into_any()
                            } else {
                                view! {
                                    <div class="parse-errors">
                                        <div class="parse-errors-title">{format!("Parse Errors ({})", errs.len())}</div>
                                        {errs.into_iter().map(|pe| view! {
                                            <div class="parse-error-item">
                                                <strong>{pe.file_path}</strong>": "{pe.error}
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
                            view! { <div class="canvas-loading">{label}</div> }.into_any()
                        } else {
                            view! {
                                <CanvasView
                                    layout=layout
                                    viewport=viewport
                                    selected_id=selected_id
                                    hovered_id=hovered_id
                                    edge_vis=edge_vis
                                    on_toggle_group_import=toggle_group_import
                                    on_toggle_group_call=toggle_group_call
                                    on_open_in_editor=handle_open_in_editor
                                    on_focus_node=focus_node
                                />
                            }.into_any()
                        }
                    }}
                </main>
            </div>

            <Show when=move || settings_open.get()>
                <div class="modal-backdrop" on:click=move |_| settings_open.set(false)>
                    <div
                        class="modal-card"
                        on:click=move |ev| ev.stop_propagation()
                    >
                        <div class="modal-header">
                            <span>"Editor Settings"</span>
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
                        </div>
                        <div class="modal-footer">
                            <button class="btn btn-ghost" on:click=move |_| settings_open.set(false)>"Cancel"</button>
                            <button
                                class="btn btn-primary"
                                on:click=move |_| {
                                    let val = settings_input.get_untracked().trim().to_string();
                                    editor_command.set(val.clone());
                                    save_editor_command(&val);
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
