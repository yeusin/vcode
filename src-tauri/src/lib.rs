mod analyzer;

use std::path::Path;

#[tauri::command]
fn analyze_project(project_path: String) -> Result<analyzer::ProjectGraph, String> {
    let path = Path::new(&project_path);
    if !path.is_dir() {
        return Err(format!("Selected path is not a directory: {project_path}"));
    }
    analyzer::analyze_project(path)
}

#[tauri::command]
fn analyze_python_project(project_path: String) -> Result<analyzer::ProjectGraph, String> {
    analyze_project(project_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            analyze_project,
            analyze_python_project
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
