use std::path::Path;
use std::process::Command;

const PYTHON_PARSER_SCRIPT: &str = include_str!("../resources/parse_python.py");

fn run_python_parser(project_path: &str) -> Result<String, String> {
    let candidates = if cfg!(target_os = "windows") {
        vec!["python", "python3"]
    } else {
        vec!["python3", "python"]
    };

    let mut last_err = String::from("No Python interpreter found");
    for bin in candidates {
        match Command::new(bin)
            .arg("-c")
            .arg(PYTHON_PARSER_SCRIPT)
            .arg(project_path)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    return String::from_utf8(output.stdout)
                        .map_err(|e| format!("Invalid UTF-8 from parser: {e}"));
                }
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                return Err(format!(
                    "Python parser failed ({bin}): {stderr} {stdout}"
                ));
            }
            Err(e) => {
                last_err = format!("Failed to launch {bin}: {e}");
            }
        }
    }
    Err(last_err)
}

#[tauri::command]
fn analyze_python_project(project_path: String) -> Result<serde_json::Value, String> {
    let path = Path::new(&project_path);
    if !path.is_dir() {
        return Err(format!("Selected path is not a directory: {project_path}"));
    }

    let raw_json = run_python_parser(&project_path)?;
    serde_json::from_str(&raw_json)
        .map_err(|e| format!("Failed to parse analyzer JSON output: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![analyze_python_project])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
