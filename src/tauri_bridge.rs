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

pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
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
    let result = JsFuture::from(promise).await.map_err(js_err_msg)?;
    serde_wasm_bindgen::from_value(result)
        .map_err(|e| format!("Failed to deserialize response for '{cmd}': {e}"))
}

pub async fn listen_project_updated<F>(mut on_update: F) -> Result<js_sys::Function, String>
where
    F: FnMut(ProjectGraph) + 'static,
{
    let event_ns = tauri_namespace("event")?;
    let listen_fn = Reflect::get(&event_ns, &JsValue::from_str("listen"))
        .map_err(|_| "Tauri event.listen not found".to_string())?;
    let func: Function = listen_fn
        .dyn_into()
        .map_err(|_| "Tauri event.listen is not a function".to_string())?;

    let cb = Closure::<dyn FnMut(JsValue)>::new(move |evt: JsValue| {
        if let Ok(payload) = Reflect::get(&evt, &JsValue::from_str("payload")) {
            if let Ok(graph) = serde_wasm_bindgen::from_value::<ProjectGraph>(payload) {
                on_update(graph);
            }
        }
    });

    let promise_val = func
        .call2(
            &event_ns,
            &JsValue::from_str("project-updated"),
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
}

pub async fn pick_project_folder() -> Result<Option<String>, String> {
    invoke("pick_project_folder", &EmptyArgs {}).await
}

pub async fn analyze_project(project_path: &str) -> Result<ProjectGraph, String> {
    invoke("analyze_project", &ProjectPathArgs { project_path }).await
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
) -> Result<(), String> {
    invoke(
        "open_in_editor",
        &OpenInEditorArgs {
            editor_command,
            file_path,
            project_path,
        },
    )
    .await
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
struct AddCommentArgs<'a> {
    project_path: &'a str,
    file_path: &'a str,
    line: usize,
    node_id: Option<&'a str>,
    author: &'a str,
    body: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommentIdArgs<'a> {
    project_path: &'a str,
    comment_id: &'a str,
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

pub async fn load_review_comments(
    project_path: &str,
) -> Result<Vec<crate::types::ReviewComment>, String> {
    invoke("load_review_comments", &ProjectPathArgs { project_path }).await
}

pub async fn add_review_comment(
    project_path: &str,
    file_path: &str,
    line: usize,
    node_id: Option<&str>,
    author: &str,
    body: &str,
) -> Result<Vec<crate::types::ReviewComment>, String> {
    invoke(
        "add_review_comment",
        &AddCommentArgs {
            project_path,
            file_path,
            line,
            node_id,
            author,
            body,
        },
    )
    .await
}

pub async fn toggle_resolve_comment(
    project_path: &str,
    comment_id: &str,
) -> Result<Vec<crate::types::ReviewComment>, String> {
    invoke(
        "toggle_resolve_comment",
        &CommentIdArgs {
            project_path,
            comment_id,
        },
    )
    .await
}

pub async fn delete_review_comment(
    project_path: &str,
    comment_id: &str,
) -> Result<Vec<crate::types::ReviewComment>, String> {
    invoke(
        "delete_review_comment",
        &CommentIdArgs {
            project_path,
            comment_id,
        },
    )
    .await
}

pub async fn fetch_pr_review(
    project_path: &str,
    pr_url_or_number: &str,
    token: Option<&str>,
) -> Result<(crate::types::GitDiffSummary, Vec<crate::types::ReviewComment>), String> {
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
