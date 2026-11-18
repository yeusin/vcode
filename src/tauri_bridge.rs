use js_sys::{Function, Promise, Reflect};
use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::types::{ProjectGraph, ProjectSettings};

fn js_err_msg(err: JsValue) -> String {
    if let Some(s) = err.as_string() {
        return s;
    }
    if let Ok(msg) = Reflect::get(&err, &JsValue::from_str("message")) {
        if let Some(s) = msg.as_string() {
            return s;
        }
    }
    format!("{err:?}")
}

fn tauri_namespace(name: &str) -> Result<JsValue, String> {
    let win = web_sys::window().ok_or_else(|| "No window object".to_string())?;
    let tauri = Reflect::get(&win, &JsValue::from_str("__TAURI__"))
        .map_err(|_| "Tauri runtime not found".to_string())?;
    if tauri.is_undefined() || tauri.is_null() {
        return Err("Tauri runtime is not available".to_string());
    }
    let ns = Reflect::get(&tauri, &JsValue::from_str(name))
        .map_err(|_| format!("Tauri namespace '{name}' not found"))?;
    if ns.is_undefined() || ns.is_null() {
        return Err(format!("Tauri namespace '{name}' is unavailable"));
    }
    Ok(ns)
}

pub fn is_tauri_available() -> bool {
    tauri_namespace("core").is_ok()
}

async fn invoke_raw<A: Serialize>(cmd: &str, args: &A) -> Result<JsValue, String> {
    let core = tauri_namespace("core")?;
    let invoke_fn = Reflect::get(&core, &JsValue::from_str("invoke"))
        .map_err(|_| "Tauri invoke not found".to_string())?;
    let func: Function = invoke_fn
        .dyn_into()
        .map_err(|_| "Tauri invoke is not a function".to_string())?;
    let js_args =
        serde_wasm_bindgen::to_value(args).map_err(|e| format!("Failed to serialize args: {e}"))?;
    let promise_val = func
        .call2(&core, &JsValue::from_str(cmd), &js_args)
        .map_err(js_err_msg)?;
    let promise: Promise = promise_val
        .dyn_into()
        .map_err(|_| "Tauri invoke did not return a Promise".to_string())?;
    JsFuture::from(promise).await.map_err(js_err_msg)
}

pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let result = invoke_raw(cmd, args).await?;
    serde_wasm_bindgen::from_value(result)
        .map_err(|e| format!("Failed to deserialize response for '{cmd}': {e}"))
}

async fn listen_event<T, F>(event_name: &str, mut on_event: F) -> Result<js_sys::Function, String>
where
    T: serde::de::DeserializeOwned + 'static,
    F: FnMut(T) + 'static,
{
    let event_ns = tauri_namespace("event")?;
    let listen_fn = Reflect::get(&event_ns, &JsValue::from_str("listen"))
        .map_err(|_| "Tauri event.listen not found".to_string())?;
    let func: Function = listen_fn
        .dyn_into()
        .map_err(|_| "Tauri event.listen is not a function".to_string())?;

    let cb = Closure::<dyn FnMut(JsValue)>::new(move |evt: JsValue| {
        if let Ok(payload) = Reflect::get(&evt, &JsValue::from_str("payload")) {
            if let Ok(val) = serde_wasm_bindgen::from_value::<T>(payload) {
                on_event(val);
            }
        }
    });

    let promise_val = func
        .call2(
            &event_ns,
            &JsValue::from_str(event_name),
            cb.as_ref().unchecked_ref(),
        )
        .map_err(js_err_msg)?;
    cb.forget();

    let promise: Promise = promise_val
        .dyn_into()
        .map_err(|_| "Tauri event.listen did not return a Promise".to_string())?;
    let unlisten_val = JsFuture::from(promise).await.map_err(js_err_msg)?;
    unlisten_val
        .dyn_into::<Function>()
        .map_err(|_| "Unlisten handle is not a function".to_string())
}

pub async fn listen_project_updated<F>(on_update: F) -> Result<js_sys::Function, String>
where
    F: FnMut(ProjectGraph) + 'static,
{
    listen_event("project-updated", on_update).await
}

pub async fn listen_analyze_progress<F>(on_progress: F) -> Result<js_sys::Function, String>
where
    F: FnMut(crate::types::AnalyzeProgress) + 'static,
{
    listen_event("analyze-progress", on_progress).await
}

#[derive(Serialize)]
struct EmptyArgs {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectPathArgs<'a> {
    project_path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsArgs<'a> {
    project_path: &'a str,
    settings: &'a ProjectSettings,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenInEditorArgs<'a> {
    editor_command: &'a str,
    file_path: &'a str,
    project_path: Option<&'a str>,
    open_in_terminal: bool,
}

pub async fn pick_project_folder() -> Result<Option<String>, String> {
    invoke("pick_project_folder", &EmptyArgs {}).await
}

pub async fn analyze_project(project_path: &str) -> Result<ProjectGraph, String> {
    let raw = invoke_raw("analyze_project", &ProjectPathArgs { project_path }).await?;
    let bytes = js_sys::Uint8Array::new(&raw).to_vec();
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("Failed to parse project graph JSON: {e}"))
}

pub async fn watch_project(project_path: &str) -> Result<(), String> {
    invoke("watch_project", &ProjectPathArgs { project_path }).await
}

pub async fn load_project_settings(project_path: &str) -> Result<Option<ProjectSettings>, String> {
    invoke("load_project_settings", &ProjectPathArgs { project_path }).await
}

pub async fn save_project_settings(
    project_path: &str,
    settings: &ProjectSettings,
) -> Result<(), String> {
    invoke(
        "save_project_settings",
        &SaveSettingsArgs {
            project_path,
            settings,
        },
    )
    .await
}

pub async fn open_in_editor(
    editor_command: &str,
    file_path: &str,
    project_path: Option<&str>,
    open_in_terminal: bool,
) -> Result<(), String> {
    invoke(
        "open_in_editor",
        &OpenInEditorArgs {
            editor_command,
            file_path,
            project_path,
            open_in_terminal,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenWindowArgs<'a> {
    project_path: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapWindowArgs<'a> {
    preset: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WindowTitleArgs<'a> {
    title: &'a str,
}

pub async fn open_new_window(project_path: Option<&str>) -> Result<(), String> {
    invoke("open_new_window", &OpenWindowArgs { project_path }).await
}

pub async fn snap_window(preset: &str) -> Result<(), String> {
    invoke("snap_window", &SnapWindowArgs { preset }).await
}

pub async fn set_window_title(title: &str) -> Result<(), String> {
    invoke("set_window_title", &WindowTitleArgs { title }).await
}

pub fn local_storage_get(key: &str) -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
}

pub fn local_storage_set(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, value);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitDiffArgs<'a> {
    project_path: &'a str,
    base_ref: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchPrArgs<'a> {
    project_path: &'a str,
    pr_url_or_number: &'a str,
    token: Option<&'a str>,
}

pub async fn get_git_diff(
    project_path: &str,
    base_ref: Option<&str>,
) -> Result<crate::types::GitDiffSummary, String> {
    invoke(
        "get_git_diff",
        &GitDiffArgs {
            project_path,
            base_ref,
        },
    )
    .await
}

pub async fn fetch_pr_review(
    project_path: &str,
    pr_url_or_number: &str,
    token: Option<&str>,
) -> Result<crate::types::GitDiffSummary, String> {
    invoke(
        "fetch_pr_review",
        &FetchPrArgs {
            project_path,
            pr_url_or_number,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListPrsArgs<'a> {
    project_path: &'a str,
    token: Option<&'a str>,
}

pub async fn list_repo_prs(
    project_path: &str,
    token: Option<&str>,
) -> Result<Vec<crate::types::PrListItem>, String> {
    invoke(
        "list_repo_prs",
        &ListPrsArgs {
            project_path,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenArgs<'a> {
    token: Option<&'a str>,
}

pub async fn list_involved_prs_all_repos(
    token: Option<&str>,
) -> Result<Vec<crate::types::PrListItem>, String> {
    invoke("list_involved_prs_all_repos", &TokenArgs { token }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloneRepoArgs<'a> {
    repo_url: &'a str,
    token: Option<&'a str>,
}

pub async fn clone_github_repo(repo_url: &str, token: Option<&str>) -> Result<String, String> {
    invoke("clone_github_repo", &CloneRepoArgs { repo_url, token }).await
}

pub async fn get_launch_target() -> Option<String> {
    invoke::<_, Option<String>>("get_launch_target", &EmptyArgs {})
        .await
        .ok()
        .flatten()
}

pub async fn check_latest_release() -> Option<(String, String)> {
    invoke::<_, Option<(String, String)>>("check_latest_release", &EmptyArgs {})
        .await
        .ok()
        .flatten()
}

pub async fn fetch_pr_details(
    project_path: &str,
    pr_url_or_number: &str,
    token: Option<&str>,
) -> Result<crate::types::PrDetails, String> {
    invoke(
        "fetch_pr_details",
        &FetchPrArgs {
            project_path,
            pr_url_or_number,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostPrCommentArgs<'a> {
    project_path: &'a str,
    pr_url_or_number: &'a str,
    body: &'a str,
    token: Option<&'a str>,
}

pub async fn post_pr_comment(
    project_path: &str,
    pr_url_or_number: &str,
    body: &str,
    token: Option<&str>,
) -> Result<(), String> {
    invoke(
        "post_pr_comment",
        &PostPrCommentArgs {
            project_path,
            pr_url_or_number,
            body,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddPrLineCommentArgs<'a> {
    project_path: &'a str,
    pr_url_or_number: &'a str,
    path: &'a str,
    line: usize,
    side: &'a str,
    body: &'a str,
    in_reply_to: Option<u64>,
    commit_id: &'a str,
    token: Option<&'a str>,
}

#[allow(clippy::too_many_arguments)]
pub async fn add_pr_line_comment(
    project_path: &str,
    pr_url_or_number: &str,
    path: &str,
    line: usize,
    side: &str,
    body: &str,
    in_reply_to: Option<u64>,
    commit_id: &str,
    token: Option<&str>,
) -> Result<(), String> {
    invoke(
        "add_pr_line_comment",
        &AddPrLineCommentArgs {
            project_path,
            pr_url_or_number,
            path,
            line,
            side,
            body,
            in_reply_to,
            commit_id,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolvePrThreadArgs<'a> {
    project_path: &'a str,
    pr_url_or_number: &'a str,
    thread_id: &'a str,
    resolve: bool,
    token: Option<&'a str>,
}

pub async fn resolve_pr_thread(
    project_path: &str,
    pr_url_or_number: &str,
    thread_id: &str,
    resolve: bool,
    token: Option<&str>,
) -> Result<(), String> {
    invoke(
        "resolve_pr_thread",
        &ResolvePrThreadArgs {
            project_path,
            pr_url_or_number,
            thread_id,
            resolve,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmitPrReviewArgs<'a> {
    project_path: &'a str,
    pr_url_or_number: &'a str,
    event: &'a str,
    body: &'a str,
    pending_comments: &'a [crate::types::PendingReviewComment],
    token: Option<&'a str>,
}

pub async fn submit_pr_review(
    project_path: &str,
    pr_url_or_number: &str,
    event: &str,
    body: &str,
    pending_comments: &[crate::types::PendingReviewComment],
    token: Option<&str>,
) -> Result<(), String> {
    invoke(
        "submit_pr_review",
        &SubmitPrReviewArgs {
            project_path,
            pr_url_or_number,
            event,
            body,
            pending_comments,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagePrArgs<'a> {
    project_path: &'a str,
    pr_url_or_number: &'a str,
    action: &'a str,
    value: &'a str,
    token: Option<&'a str>,
}

pub async fn manage_pr(
    project_path: &str,
    pr_url_or_number: &str,
    action: &str,
    value: &str,
    token: Option<&str>,
) -> Result<(), String> {
    invoke(
        "manage_pr",
        &ManagePrArgs {
            project_path,
            pr_url_or_number,
            action,
            value,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindBranchPrArgs<'a> {
    project_path: &'a str,
    token: Option<&'a str>,
}

pub async fn find_branch_pr(
    project_path: &str,
    token: Option<&str>,
) -> Result<Option<String>, String> {
    invoke(
        "find_branch_pr",
        &FindBranchPrArgs {
            project_path,
            token,
        },
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenExternalUrlArgs<'a> {
    url: &'a str,
}

pub async fn open_external_url(url: &str) -> Result<(), String> {
    invoke("open_external_url", &OpenExternalUrlArgs { url }).await
}

pub async fn fetch_pr_suggestions(
    project_path: &str,
    pr_url_or_number: &str,
    token: Option<&str>,
) -> Result<crate::types::PrSuggestions, String> {
    invoke(
        "fetch_pr_suggestions",
        &FetchPrArgs {
            project_path,
            pr_url_or_number,
            token,
        },
    )
    .await
}
