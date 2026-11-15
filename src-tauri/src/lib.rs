mod analyzer;

use std::path::Path;
use std::process::Command;

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
fn open_in_editor(editor_command: String, file_path: String) -> Result<(), String> {
    let (program, args) = build_editor_invocation(&editor_command, &file_path)?;
    Command::new(&program)
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch '{program}': {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            analyze_project,
            analyze_python_project,
            open_in_editor
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::build_editor_invocation;

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
}
