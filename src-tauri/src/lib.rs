mod analyzer;

use notify::{Event, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    pub show_import_edges: bool,
    pub show_call_edges: bool,
    pub hidden_import_group_ids: Vec<String>,
    pub hidden_call_group_ids: Vec<String>,
    pub editor_command: String,
}

struct ActiveWatcher {
    _watcher: notify::RecommendedWatcher,
    stop_tx: mpsc::Sender<()>,
}

struct WatcherState {
    active: Mutex<Option<ActiveWatcher>>,
}

impl Default for WatcherState {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }
}

fn config_root_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(appdata).join("vcode"));
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return Ok(PathBuf::from(profile).join("AppData").join("Roaming").join("vcode"));
        }
        return Err("Unable to determine APPDATA directory".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("vcode"));
        }
        return Err("Unable to determine HOME directory".to_string());
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Ok(PathBuf::from(xdg).join("vcode"));
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home).join(".config").join("vcode"));
        }
        Err("Unable to determine HOME or XDG_CONFIG_HOME directory".to_string())
    }
}

fn fnv1a_32(input: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in input.as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn project_settings_dir(base_dir: &Path, project_path: &str) -> PathBuf {
    let p = Path::new(project_path);
    let raw_name = p
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("project");
    let sanitized: String = raw_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let hash = fnv1a_32(project_path);
    base_dir.join(format!("{sanitized}-{hash:08x}"))
}

fn project_settings_file(project_path: &str) -> Result<PathBuf, String> {
    let base = config_root_dir()?;
    Ok(project_settings_dir(&base, project_path).join("settings.json"))
}

#[tauri::command]
async fn analyze_project(project_path: String) -> Result<analyzer::ProjectGraph, String> {
    let path = PathBuf::from(&project_path);
    if !path.is_dir() {
        return Err(format!("Selected path is not a directory: {project_path}"));
    }
    tauri::async_runtime::spawn_blocking(move || analyzer::analyze_project(&path))
        .await
        .map_err(|e| format!("Analyzer task failed: {e}"))?
}

#[tauri::command]
async fn analyze_python_project(project_path: String) -> Result<analyzer::ProjectGraph, String> {
    analyze_project(project_path).await
}

#[tauri::command]
fn watch_project(
    app: AppHandle,
    state: State<'_, WatcherState>,
    project_path: String,
) -> Result<(), String> {
    let root = PathBuf::from(&project_path);
    if !root.is_dir() {
        return Err(format!("Not a directory: {project_path}"));
    }

    let mut guard = state
        .active
        .lock()
        .map_err(|_| "Watcher mutex poisoned".to_string())?;
    if let Some(prev) = guard.take() {
        let _ = prev.stop_tx.send(());
    }

    let (dirty_tx, dirty_rx) = mpsc::channel::<()>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let dirty_flag = Arc::new(dirty_tx);

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            if event.kind.is_access() {
                return;
            }
            let relevant = event
                .paths
                .iter()
                .any(|p| analyzer::is_watched_path(p) || p.is_dir());
            if relevant {
                let _ = dirty_flag.send(());
            }
        }
    })
    .map_err(|e| format!("Failed to initialize file watcher: {e}"))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch {}: {e}", root.display()))?;

    std::thread::spawn(move || {
        let debounce = Duration::from_millis(300);
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            match dirty_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(()) => {
                    // Coalesce rapid file events (e.g. nvim swap/write sequences)
                    std::thread::sleep(debounce);
                    while dirty_rx.try_recv().is_ok() {}
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    if let Ok(graph) = analyzer::analyze_project(&root) {
                        let _ = app.emit("project-updated", graph);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    *guard = Some(ActiveWatcher {
        _watcher: watcher,
        stop_tx,
    });

    Ok(())
}

#[tauri::command]
fn unwatch_project(state: State<'_, WatcherState>) -> Result<(), String> {
    let mut guard = state
        .active
        .lock()
        .map_err(|_| "Watcher mutex poisoned".to_string())?;
    if let Some(prev) = guard.take() {
        let _ = prev.stop_tx.send(());
    }
    Ok(())
}

#[tauri::command]
fn load_project_settings(project_path: String) -> Result<Option<ProjectSettings>, String> {
    let file = project_settings_file(&project_path)?;
    if !file.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&file)
        .map_err(|e| format!("Failed to read settings {}: {e}", file.display()))?;
    let parsed: ProjectSettings = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse settings {}: {e}", file.display()))?;
    Ok(Some(parsed))
}

#[tauri::command]
fn save_project_settings(project_path: String, settings: ProjectSettings) -> Result<(), String> {
    let file = project_settings_file(&project_path)?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create settings dir {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    fs::write(&file, json)
        .map_err(|e| format!("Failed to write settings {}: {e}", file.display()))
}

fn build_editor_invocation(editor_command: &str, file_path: &str) -> Result<(String, Vec<String>), String> {
    let parts: Vec<&str> = editor_command.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Editor command is empty".to_string());
    }

    let program = parts[0].to_string();
    let mut has_placeholder = false;
    let mut args: Vec<String> = parts[1..]
        .iter()
        .map(|arg| {
            if arg.contains("{file}") {
                has_placeholder = true;
                arg.replace("{file}", file_path)
            } else {
                (*arg).to_string()
            }
        })
        .collect();

    if !has_placeholder {
        args.push(file_path.to_string());
    }

    Ok((program, args))
}

#[tauri::command]
fn open_in_editor(
    editor_command: String,
    file_path: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let (program, args) = build_editor_invocation(&editor_command, &file_path)?;
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    if let Some(dir) = project_path {
        let p = Path::new(&dir);
        if p.is_dir() {
            cmd.current_dir(p);
        }
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch '{program}': {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(WatcherState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            analyze_project,
            analyze_python_project,
            watch_project,
            unwatch_project,
            load_project_settings,
            save_project_settings,
            open_in_editor
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{build_editor_invocation, project_settings_dir, ProjectSettings};
    use std::fs;

    #[test]
    fn appends_file_when_no_placeholder() {
        let (prog, args) = build_editor_invocation("code -r", "/tmp/a.py").unwrap();
        assert_eq!(prog, "code");
        assert_eq!(args, vec!["-r", "/tmp/a.py"]);
    }

    #[test]
    fn replaces_file_placeholder() {
        let (prog, args) = build_editor_invocation("code --goto {file}:1", "/tmp/a.py").unwrap();
        assert_eq!(prog, "code");
        assert_eq!(args, vec!["--goto", "/tmp/a.py:1"]);
    }

    #[test]
    fn stores_and_loads_project_settings_per_project_dir() {
        let base = std::env::temp_dir().join(format!("vcode_cfg_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = project_settings_dir(&base, "/home/user/projects/exoindex");
        assert!(dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("exoindex-"));

        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        let settings = ProjectSettings {
            show_import_edges: true,
            show_call_edges: false,
            hidden_import_group_ids: vec!["dir:tests".to_string()],
            hidden_call_group_ids: vec!["dir:tests".to_string()],
            editor_command: "code".to_string(),
        };
        fs::write(&file, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        let loaded: ProjectSettings =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(loaded, settings);
        let _ = fs::remove_dir_all(&base);
    }
}
