// Prevent additional console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    process: Mutex<Option<Child>>,
    repo_root: Mutex<Option<PathBuf>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ConfigEntry {
    name: String,
    path: String,
    group: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct EnvStatus {
    python: Option<String>,
    gpu: Option<String>,
    repo_root: Option<String>,
}

#[derive(Serialize, Clone)]
struct LogLine {
    stream: String,
    line: String,
}

fn find_repo_root_from(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start.to_path_buf());
    while let Some(dir) = current {
        if dir.join("run.py").is_file() && dir.join("toolkit").is_dir() {
            return Some(dir);
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    None
}

#[tauri::command]
fn detect_repo_root(state: State<'_, AppState>) -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.to_path_buf());
        }
    }

    for c in candidates {
        if let Some(root) = find_repo_root_from(&c) {
            *state.repo_root.lock().unwrap() = Some(root.clone());
            return Some(root.to_string_lossy().into_owned());
        }
    }
    None
}

#[tauri::command]
fn set_repo_root(path: String, state: State<'_, AppState>) -> Result<String, String> {
    let p = PathBuf::from(&path);
    if !p.join("run.py").is_file() || !p.join("toolkit").is_dir() {
        return Err(
            "That folder does not look like an ai-toolkit repo (missing run.py or toolkit/).".into(),
        );
    }
    *state.repo_root.lock().unwrap() = Some(p.clone());
    Ok(p.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_repo_root(state: State<'_, AppState>) -> Option<String> {
    state
        .repo_root
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}

fn scan_configs_in(dir: &Path, group: &str, out: &mut Vec<ConfigEntry>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    let exts: HashSet<&str> = ["yaml", "yml", "json"].into_iter().collect();
    for entry in iter.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !exts.contains(ext) {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        out.push(ConfigEntry {
            name,
            path: p.to_string_lossy().into_owned(),
            group: group.to_string(),
        });
    }
}

#[tauri::command]
fn list_configs(state: State<'_, AppState>) -> Result<Vec<ConfigEntry>, String> {
    let root = state
        .repo_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "ai-toolkit folder has not been set.".to_string())?;
    let mut out = Vec::new();
    scan_configs_in(&root.join("config"), "My configs", &mut out);
    scan_configs_in(&root.join("config").join("examples"), "Examples", &mut out);
    out.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn python_cmd(root: &Path) -> String {
    if cfg!(windows) {
        let venv = root.join("venv").join("Scripts").join("python.exe");
        if venv.is_file() {
            return venv.to_string_lossy().into_owned();
        }
    } else {
        let venv = root.join("venv").join("bin").join("python");
        if venv.is_file() {
            return venv.to_string_lossy().into_owned();
        }
    }
    "python".to_string()
}

#[tauri::command]
fn check_environment(state: State<'_, AppState>) -> EnvStatus {
    let repo_root = state.repo_root.lock().unwrap().clone();

    let exe = repo_root
        .as_ref()
        .map(|r| python_cmd(r))
        .unwrap_or_else(|| "python".to_string());

    let python = Command::new(&exe).arg("--version").output().ok().and_then(|o| {
        if !o.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !stdout.is_empty() {
            Some(stdout)
        } else {
            Some(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    });

    let gpu = Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            }
        });

    EnvStatus {
        python,
        gpu,
        repo_root: repo_root.map(|p| p.to_string_lossy().into_owned()),
    }
}

#[tauri::command]
fn is_running(state: State<'_, AppState>) -> bool {
    let mut guard = state.process.lock().unwrap();
    let Some(child) = guard.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(Some(_)) => {
            *guard = None;
            false
        }
        Ok(None) => true,
        Err(_) => false,
    }
}

#[tauri::command]
fn start_training(
    app: AppHandle,
    state: State<'_, AppState>,
    config_path: String,
) -> Result<u32, String> {
    {
        let guard = state.process.lock().unwrap();
        if guard.is_some() {
            return Err("A training job is already running.".into());
        }
    }
    let root = state
        .repo_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "ai-toolkit folder has not been set.".to_string())?;
    let py = python_cmd(&root);

    let mut cmd = Command::new(&py);
    cmd.arg("run.py")
        .arg(&config_path)
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONUNBUFFERED", "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch python: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "no stdout".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "no stderr".to_string())?;

    let app_out = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = app_out.emit(
                "training-log",
                LogLine {
                    stream: "stdout".into(),
                    line,
                },
            );
        }
    });
    let app_err = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = app_err.emit(
                "training-log",
                LogLine {
                    stream: "stderr".into(),
                    line,
                },
            );
        }
    });

    let pid = child.id();
    *state.process.lock().unwrap() = Some(child);

    let app_wait = app.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(400));
        let state: State<AppState> = app_wait.state();
        let mut guard = state.process.lock().unwrap();
        let Some(child) = guard.as_mut() else { break };
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code();
                let msg = match code {
                    Some(c) => format!("Process exited with code {c}."),
                    None => "Process exited (terminated by signal).".into(),
                };
                *guard = None;
                drop(guard);
                let _ = app_wait.emit(
                    "training-log",
                    LogLine {
                        stream: "system".into(),
                        line: msg,
                    },
                );
                let _ = app_wait.emit("training-exit", code);
                break;
            }
            Ok(None) => continue,
            Err(_) => break,
        }
    });

    let _ = app.emit(
        "training-log",
        LogLine {
            stream: "system".into(),
            line: format!("Started training (pid {pid})."),
        },
    );
    Ok(pid)
}

#[tauri::command]
fn stop_training(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.process.lock().unwrap();
    let Some(child) = guard.as_mut() else {
        return Err("No training job is running.".into());
    };
    let pid = child.id();

    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(pid as i32), libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .output();
    }

    let _ = child.kill();

    let _ = app.emit(
        "training-log",
        LogLine {
            stream: "system".into(),
            line: "Stop requested.".into(),
        },
    );
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            process: Mutex::new(None),
            repo_root: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            detect_repo_root,
            set_repo_root,
            get_repo_root,
            list_configs,
            check_environment,
            is_running,
            start_training,
            stop_training,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
