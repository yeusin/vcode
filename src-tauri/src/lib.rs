mod analyzer;
mod review;

use notify::{Event, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    pub show_import_edges: bool,
    pub show_call_edges: bool,
    pub hidden_import_group_ids: Vec<String>,
    pub hidden_call_group_ids: Vec<String>,
    pub editor_command: String,
    #[serde(default)]
    pub open_in_terminal: bool,
}

struct ActiveWatcher {
    _watcher: notify::RecommendedWatcher,
    stop_tx: mpsc::Sender<()>,
}

struct WatcherState {
    active: Mutex<HashMap<String, ActiveWatcher>>,
}

impl Default for WatcherState {
    fn default() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
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
async fn analyze_project(
    window: WebviewWindow,
    project_path: String,
) -> Result<tauri::ipc::Response, String> {
    let path = PathBuf::from(&project_path);
    if !path.is_dir() {
        return Err(format!("Selected path is not a directory: {project_path}"));
    }
    let graph = tauri::async_runtime::spawn_blocking(move || {
        analyzer::analyze_project_with_progress(&path, |progress| {
            let _ = window.emit("analyze-progress", &progress);
        })
    })
    .await
    .map_err(|e| format!("Analyzer task failed: {e}"))??;
    let bytes = serde_json::to_vec(&graph)
        .map_err(|e| format!("Failed to serialize project graph: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn analyze_python_project(
    window: WebviewWindow,
    project_path: String,
) -> Result<tauri::ipc::Response, String> {
    analyze_project(window, project_path).await
}

#[tauri::command]
async fn watch_project(
    window: WebviewWindow,
    state: State<'_, WatcherState>,
    project_path: String,
) -> Result<(), String> {
    let root = PathBuf::from(&project_path);
    if !root.is_dir() {
        return Err(format!("Not a directory: {project_path}"));
    }
    let win_label = window.label().to_string();

    {
        let mut guard = state
            .active
            .lock()
            .map_err(|_| "Watcher mutex poisoned".to_string())?;
        if let Some(prev) = guard.remove(&win_label) {
            let _ = prev.stop_tx.send(());
        }
    }

    let (dirty_tx, dirty_rx) = mpsc::channel::<()>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let dirty_flag = Arc::new(dirty_tx);

    let root_for_watch = root.clone();
    let watcher = tauri::async_runtime::spawn_blocking(move || -> Result<notify::RecommendedWatcher, String> {
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
            .watch(&root_for_watch, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch {}: {e}", root_for_watch.display()))?;
        Ok(watcher)
    })
    .await
    .map_err(|e| format!("Watcher setup task failed: {e}"))??;

    let win_for_thread = window.clone();
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
                        let _ = win_for_thread.emit("project-updated", graph);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    let mut guard = state
        .active
        .lock()
        .map_err(|_| "Watcher mutex poisoned".to_string())?;
    guard.insert(
        win_label,
        ActiveWatcher {
            _watcher: watcher,
            stop_tx,
        },
    );

    Ok(())
}

#[tauri::command]
fn unwatch_project(window: WebviewWindow, state: State<'_, WatcherState>) -> Result<(), String> {
    let mut guard = state
        .active
        .lock()
        .map_err(|_| "Watcher mutex poisoned".to_string())?;
    if let Some(prev) = guard.remove(window.label()) {
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

fn resolve_editor_binary(program: &str) -> String {
    if program.contains('/') || program.contains('\\') {
        return program.to_string();
    }
    if let Some(home) = std::env::var_os("HOME") {
        let hp = PathBuf::from(home);
        for sub in [".local/bin", "bin", ".cargo/bin"] {
            let candidate = hp.join(sub).join(program);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
        "/home/linuxbrew/.linuxbrew/bin",
    ] {
        let candidate = Path::new(dir).join(program);
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    program.to_string()
}

#[cfg(any(target_os = "macos", test))]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(any(target_os = "macos", test))]
fn build_terminal_shell_command(
    program: &str,
    args: &[String],
    project_path: Option<&str>,
) -> String {
    let mut cmd_parts = vec![shell_quote(program)];
    for arg in args {
        cmd_parts.push(shell_quote(arg));
    }
    let exec_line = cmd_parts.join(" ");
    if let Some(dir) = project_path {
        let p = Path::new(dir);
        if p.is_dir() {
            return format!("cd {} && {}", shell_quote(dir), exec_line);
        }
    }
    exec_line
}

#[tauri::command]
fn open_in_editor(
    editor_command: String,
    file_path: String,
    project_path: Option<String>,
    open_in_terminal: Option<bool>,
) -> Result<(), String> {
    let (raw_program, args) = build_editor_invocation(&editor_command, &file_path)?;
    let program = resolve_editor_binary(&raw_program);

    if open_in_terminal.unwrap_or(false) {
        #[cfg(target_os = "macos")]
        {
            let shell_cmd = build_terminal_shell_command(&program, &args, project_path.as_deref());
            let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!("tell application \"Terminal\" to do script \"{escaped}\"");
            return Command::new("osascript")
                .args(["-e", &script, "-e", "tell application \"Terminal\" to activate"])
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to open Terminal for '{program}': {e}"));
        }

        #[cfg(target_os = "linux")]
        {
            let mut terms: Vec<String> = Vec::new();
            if let Some(t) = std::env::var_os("TERMINAL") {
                let s = t.to_string_lossy().trim().to_string();
                if !s.is_empty() {
                    terms.push(s);
                }
            }
            for t in [
                "x-terminal-emulator",
                "gnome-terminal",
                "konsole",
                "xfce4-terminal",
                "kitty",
                "alacritty",
                "ghostty",
                "wezterm",
                "xterm",
            ] {
                terms.push(t.to_string());
            }
            for term in terms {
                let mut cmd = Command::new(&term);
                if let Some(dir) = &project_path {
                    let p = Path::new(dir);
                    if p.is_dir() {
                        cmd.current_dir(p);
                    }
                }
                if term.ends_with("gnome-terminal") || term.ends_with("wezterm") {
                    if term.ends_with("wezterm") {
                        cmd.arg("start");
                    }
                    cmd.arg("--");
                } else {
                    cmd.arg("-e");
                }
                cmd.arg(&program).args(&args);
                if cmd.spawn().is_ok() {
                    return Ok(());
                }
            }
            return Err(format!("No terminal emulator found to launch '{program}'"));
        }

        #[cfg(target_os = "windows")]
        {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", "start", ""]);
            cmd.arg(&program).args(&args);
            if let Some(dir) = project_path {
                let p = Path::new(&dir);
                if p.is_dir() {
                    cmd.current_dir(p);
                }
            }
            return cmd
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to open terminal for '{program}': {e}"));
        }
    }

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

#[tauri::command]
async fn pick_project_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .set_title("Select Project Folder")
            .blocking_pick_folder();
        Ok(picked
            .and_then(|p| p.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| format!("Failed to open folder dialog: {e}"))?
}

static NEXT_WINDOW_ID: AtomicUsize = AtomicUsize::new(1);

fn percent_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn compute_snap_rect(
    ax: i32,
    ay: i32,
    aw: u32,
    ah: u32,
    preset: &str,
) -> Result<(i32, i32, u32, u32), String> {
    let w = aw as i32;
    let h = ah as i32;
    let half_w = (w / 2).max(1) as u32;
    let half_h = (h / 2).max(1) as u32;
    let third_w = (w / 3).max(1) as u32;

    match preset {
        "left" => Ok((ax, ay, half_w, ah)),
        "right" => Ok((ax + w / 2, ay, aw - (w / 2) as u32, ah)),
        "top" => Ok((ax, ay, aw, half_h)),
        "bottom" => Ok((ax, ay + h / 2, aw, ah - (h / 2) as u32)),
        "top-left" => Ok((ax, ay, half_w, half_h)),
        "top-right" => Ok((ax + w / 2, ay, aw - (w / 2) as u32, half_h)),
        "bottom-left" => Ok((ax, ay + h / 2, half_w, ah - (h / 2) as u32)),
        "bottom-right" => Ok((
            ax + w / 2,
            ay + h / 2,
            aw - (w / 2) as u32,
            ah - (h / 2) as u32,
        )),
        "left-third" => Ok((ax, ay, third_w, ah)),
        "center-third" => Ok((ax + w / 3, ay, third_w, ah)),
        "right-third" => Ok((ax + (w * 2) / 3, ay, aw - ((w * 2) / 3) as u32, ah)),
        "center" => {
            let cw = ((aw as f64) * 0.75).round() as u32;
            let ch = ((ah as f64) * 0.80).round() as u32;
            let cx = ax + ((aw - cw) / 2) as i32;
            let cy = ay + ((ah - ch) / 2) as i32;
            Ok((cx, cy, cw, ch))
        }
        "maximize" => Ok((ax, ay, aw, ah)),
        _ => Err(format!("Unknown snap preset: {preset}")),
    }
}

#[tauri::command]
fn open_new_window(app: AppHandle, project_path: Option<String>) -> Result<(), String> {
    let id = NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    let label = format!("vcode-{id}");
    let (url, title) = match project_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => {
            let name = Path::new(p)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("VCode");
            (
                WebviewUrl::App(format!("index.html?project={}", percent_encode_query(p)).into()),
                format!("VCode — {name}"),
            )
        }
        None => (
            WebviewUrl::App("index.html".into()),
            "VCode — Code Visualizer".to_string(),
        ),
    };

    WebviewWindowBuilder::new(&app, label, url)
        .title(title)
        .inner_size(1400.0, 900.0)
        .min_inner_size(420.0, 320.0)
        .build()
        .map(|_| ())
        .map_err(|e| format!("Failed to open new window: {e}"))
}

#[tauri::command]
fn snap_window(window: WebviewWindow, preset: String) -> Result<(), String> {
    let monitor = window
        .current_monitor()
        .map_err(|e| format!("Failed to query monitor: {e}"))?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or_else(|| "No monitor detected".to_string())?;
    let wa = monitor.work_area();
    let (x, y, w, h) = compute_snap_rect(wa.position.x, wa.position.y, wa.size.width, wa.size.height, &preset)?;

    let _ = window.unmaximize();
    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)))
        .map_err(|e| format!("Failed to position window: {e}"))?;
    window
        .set_size(tauri::Size::Physical(tauri::PhysicalSize::new(w, h)))
        .map_err(|e| format!("Failed to resize window: {e}"))?;
    Ok(())
}

#[tauri::command]
fn set_window_title(window: WebviewWindow, title: String) -> Result<(), String> {
    window
        .set_title(&title)
        .map_err(|e| format!("Failed to set window title: {e}"))
}

#[tauri::command]
fn get_git_diff(
    project_path: String,
    base_ref: Option<String>,
) -> Result<review::GitDiffSummary, String> {
    review::get_git_diff(&project_path, base_ref)
}

#[tauri::command]
async fn fetch_pr_review(
    project_path: String,
    pr_url_or_number: String,
    token: Option<String>,
) -> Result<review::GitDiffSummary, String> {
    review::fetch_pr_review(&project_path, &pr_url_or_number, token).await
}

#[tauri::command]
async fn list_repo_prs(
    project_path: String,
    token: Option<String>,
) -> Result<Vec<review::PrListItem>, String> {
    review::list_repo_prs(&project_path, token).await
}

#[tauri::command]
async fn list_involved_prs_all_repos(
    token: Option<String>,
) -> Result<Vec<review::PrListItem>, String> {
    review::list_involved_prs_all_repos(token).await
}

#[tauri::command]
async fn clone_github_repo(repo_url: String, token: Option<String>) -> Result<String, String> {
    review::clone_github_repo(&repo_url, token).await
}

fn resolve_cli_target_arg(args: &[String], cwd: &Path) -> Option<String> {
    let raw = args
        .iter()
        .skip(1)
        .map(|s| s.trim())
        .find(|s| !s.is_empty() && !s.starts_with('-'))?;
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("git@") {
        return Some(raw.to_string());
    }
    let p = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        cwd.join(raw)
    };
    let dir = if p.is_file() {
        p.parent().map(Path::to_path_buf).unwrap_or(p)
    } else {
        p
    };
    if !dir.is_dir() {
        return None;
    }
    let resolved = fs::canonicalize(&dir).unwrap_or(dir);
    Some(resolved.to_string_lossy().into_owned())
}

fn parse_semver_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let clean = v.trim().trim_start_matches('v');
    let core = clean.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[tauri::command]
async fn check_latest_release() -> Result<Option<(String, String)>, String> {
    let current = parse_semver_tuple(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0));
    if current == (0, 0, 0) {
        return Ok(None);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get("https://api.github.com/repos/yeusin/vcode/releases/latest")
        .header("User-Agent", "vcode-app")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let tag = json.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    let url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/yeusin/vcode/releases/latest");
    if let Some(latest) = parse_semver_tuple(tag) {
        if latest > current {
            return Ok(Some((tag.to_string(), url.to_string())));
        }
    }
    Ok(None)
}

#[tauri::command]
fn get_launch_target() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir().ok()?;
    resolve_cli_target_arg(&args, &cwd)
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
            open_in_editor,
            pick_project_folder,
            open_new_window,
            snap_window,
            set_window_title,
            get_git_diff,
            fetch_pr_review,
            list_repo_prs,
            list_involved_prs_all_repos,
            clone_github_repo,
            get_launch_target,
            check_latest_release
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.try_state::<WatcherState>() {
                    if let Ok(mut guard) = state.active.lock() {
                        if let Some(prev) = guard.remove(window.label()) {
                            let _ = prev.stop_tx.send(());
                        }
                    }
                }
            }
        })
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
            open_in_terminal: true,
        };
        fs::write(&file, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        let loaded: ProjectSettings =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(loaded, settings);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn formats_terminal_shell_command() {
        let cmd = super::build_terminal_shell_command(
            "/opt/homebrew/bin/nvim",
            &["/tmp/a b/main.rs".to_string()],
            None,
        );
        assert_eq!(cmd, "'/opt/homebrew/bin/nvim' '/tmp/a b/main.rs'");
    }

    #[test]
    fn computes_snap_rects_for_halves_and_quarters() {
        assert_eq!(
            super::compute_snap_rect(0, 24, 1920, 1056, "left").unwrap(),
            (0, 24, 960, 1056)
        );
        assert_eq!(
            super::compute_snap_rect(0, 24, 1920, 1056, "right").unwrap(),
            (960, 24, 960, 1056)
        );
        assert_eq!(
            super::compute_snap_rect(0, 24, 1920, 1056, "bottom-right").unwrap(),
            (960, 552, 960, 528)
        );
        assert_eq!(
            super::compute_snap_rect(0, 24, 1920, 1056, "left-third").unwrap(),
            (0, 24, 640, 1056)
        );
    }

    #[test]
    fn resolves_cli_target_paths_and_urls() {
        let cwd = std::env::current_dir().unwrap();
        let dot = super::resolve_cli_target_arg(
            &["vcode".to_string(), "-psn_0_1".to_string(), ".".to_string()],
            &cwd,
        )
        .unwrap();
        assert_eq!(std::path::PathBuf::from(dot), fs::canonicalize(&cwd).unwrap());

        let pr_url = "https://github.com/yeusin/vcode/pull/12";
        assert_eq!(
            super::resolve_cli_target_arg(&["vcode".to_string(), pr_url.to_string()], &cwd),
            Some(pr_url.to_string())
        );

        assert_eq!(
            super::resolve_cli_target_arg(&["vcode".to_string()], &cwd),
            None
        );
    }

    #[test]
    fn parses_semver_tags_for_update_check() {
        assert_eq!(super::parse_semver_tuple("v0.3.5"), Some((0, 3, 5)));
        assert_eq!(super::parse_semver_tuple("0.4.0-beta.1"), Some((0, 4, 0)));
        assert!(super::parse_semver_tuple("v0.3.6") > super::parse_semver_tuple("v0.3.5"));
    }
}
