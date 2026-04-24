// Prevent additional console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashSet,
    ffi::OsStr,
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

const WEB_UI_URL: &str = "http://localhost:8675";

struct AppState {
    training: Mutex<Option<Child>>,
    utility: Mutex<Option<Child>>,
    web_ui: Mutex<Option<Child>>,
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
    node: Option<String>,
    git: Option<String>,
    repo_root: Option<String>,
    is_portable: bool,
}

#[derive(Serialize, Clone)]
struct LogLine {
    source: String,
    stream: String,
    line: String,
}

#[derive(Serialize, Clone)]
struct JobEnd {
    source: String,
    code: Option<i32>,
}

#[derive(Copy, Clone)]
enum Slot {
    Training,
    Utility,
    WebUi,
}

impl Slot {
    fn source(self) -> &'static str {
        match self {
            Slot::Training => "training",
            Slot::Utility => "utility",
            Slot::WebUi => "web-ui",
        }
    }
}

fn slot_guard<'a>(
    state: &'a State<'_, AppState>,
    slot: Slot,
) -> std::sync::MutexGuard<'a, Option<Child>> {
    match slot {
        Slot::Training => state.training.lock().unwrap(),
        Slot::Utility => state.utility.lock().unwrap(),
        Slot::WebUi => state.web_ui.lock().unwrap(),
    }
}

fn emit_log(app: &AppHandle, source: &str, stream: &str, line: impl Into<String>) {
    let _ = app.emit(
        "app-log",
        LogLine {
            source: source.into(),
            stream: stream.into(),
            line: line.into(),
        },
    );
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
    // Check common portable / venv layouts first so a zipped, pre-bundled
    // distribution can run without any system Python installed.
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        vec![
            root.join("venv").join("Scripts").join("python.exe"),
            root.join("python-portable").join("python.exe"),
            root.join("python-portable").join("Scripts").join("python.exe"),
            root.join("python").join("python.exe"),
        ]
    } else {
        vec![
            root.join("venv").join("bin").join("python"),
            root.join("python-portable").join("bin").join("python3"),
            root.join("python-portable").join("bin").join("python"),
            root.join("python-portable").join("install").join("bin").join("python3"),
            root.join("python").join("bin").join("python3"),
        ]
    };
    for c in candidates {
        if c.is_file() {
            return c.to_string_lossy().into_owned();
        }
    }
    "python".to_string()
}

fn first_line_of(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let s = if s.is_empty() {
        String::from_utf8_lossy(&out.stderr).trim().to_string()
    } else {
        s
    };
    s.lines().next().map(|l| l.to_string())
}

#[tauri::command]
fn check_environment(state: State<'_, AppState>) -> EnvStatus {
    let repo_root = state.repo_root.lock().unwrap().clone();

    let python_exe = repo_root
        .as_ref()
        .map(|r| python_cmd(r))
        .unwrap_or_else(|| "python".to_string());

    let python = first_line_of(&python_exe, &["--version"]);
    let gpu = first_line_of("nvidia-smi", &["--query-gpu=name", "--format=csv,noheader"]);
    let node = first_line_of("node", &["--version"]);
    let git = first_line_of("git", &["--version"]);

    let portable = repo_root
        .as_ref()
        .map(|r| r.join("portable.flag").is_file() || r.join("python-portable").is_dir())
        .unwrap_or(false);

    EnvStatus {
        python,
        gpu,
        node,
        git,
        repo_root: repo_root.map(|p| p.to_string_lossy().into_owned()),
        is_portable: portable,
    }
}

fn try_wait_slot(state: &State<'_, AppState>, slot: Slot) -> bool {
    let mut guard = slot_guard(state, slot);
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
fn is_running(state: State<'_, AppState>) -> bool {
    try_wait_slot(&state, Slot::Training)
}

#[tauri::command]
fn is_utility_running(state: State<'_, AppState>) -> bool {
    try_wait_slot(&state, Slot::Utility)
}

#[tauri::command]
fn is_web_ui_running(state: State<'_, AppState>) -> bool {
    try_wait_slot(&state, Slot::WebUi)
}

fn configure_group(cmd: &mut Command) {
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
}

fn kill_child(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    unsafe {
        // SIGKILL to the whole process group so PyTorch dataloader workers
        // (and any other children of the python process) also die.
        let _ = libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .output();
    }
    let _ = child.kill();
}

fn spawn_and_stream(
    app: AppHandle,
    slot: Slot,
    mut cmd: Command,
    friendly_desc: &str,
) -> Result<u32, String> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONUNBUFFERED", "1");
    configure_group(&mut cmd);

    // Hold the slot lock from the "already running?" check through storing
    // the new Child so two fast clicks cannot both pass the check and race.
    let state: State<AppState> = app.state();
    let mut guard = slot_guard(&state, slot);
    if guard.is_some() {
        return Err(format!("{friendly_desc} is already running."));
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch {}: {e}", friendly_desc.to_lowercase()))?;
    let stdout = child.stdout.take().ok_or_else(|| "no stdout".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "no stderr".to_string())?;

    let source = slot.source();

    let app_out = app.clone();
    let src_out = source.to_string();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            emit_log(&app_out, &src_out, "stdout", line);
        }
    });
    let app_err = app.clone();
    let src_err = source.to_string();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            emit_log(&app_err, &src_err, "stderr", line);
        }
    });

    let pid = child.id();
    *guard = Some(child);
    drop(guard);

    let app_wait = app.clone();
    let src_wait = source.to_string();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(400));
        let state: State<AppState> = app_wait.state();
        let mut guard = slot_guard(&state, slot);
        let Some(child) = guard.as_mut() else { break };
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code();
                let msg = match code {
                    Some(c) => format!("[{src_wait}] process exited with code {c}."),
                    None => format!("[{src_wait}] process exited (terminated by signal)."),
                };
                *guard = None;
                drop(guard);
                emit_log(&app_wait, &src_wait, "system", msg);
                let _ = app_wait.emit(
                    "job-end",
                    JobEnd {
                        source: src_wait.clone(),
                        code,
                    },
                );
                break;
            }
            Ok(None) => continue,
            Err(_) => break,
        }
    });

    Ok(pid)
}

fn stop_slot(app: &AppHandle, slot: Slot) -> bool {
    let state: State<AppState> = app.state();
    let mut guard = slot_guard(&state, slot);
    let Some(child) = guard.as_mut() else {
        return false;
    };
    kill_child(child);
    true
}

#[tauri::command]
fn start_training(
    app: AppHandle,
    state: State<'_, AppState>,
    config_path: String,
) -> Result<u32, String> {
    let root = state
        .repo_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "ai-toolkit folder has not been set.".to_string())?;

    if config_path.trim().is_empty() {
        return Err("No config file was selected.".into());
    }
    let cfg = Path::new(&config_path);
    let cfg_abs = if cfg.is_absolute() { cfg.to_path_buf() } else { root.join(cfg) };
    if !cfg_abs.is_file() {
        return Err(format!("Config file not found: {}", cfg_abs.display()));
    }

    let py = python_cmd(&root);

    let mut cmd = Command::new(&py);
    cmd.arg("run.py").arg(&config_path).current_dir(&root);

    emit_log(
        &app,
        "training",
        "system",
        format!("Starting: {py} run.py {config_path}"),
    );
    spawn_and_stream(app, Slot::Training, cmd, "Training")
}

#[tauri::command]
fn stop_training(app: AppHandle) -> Result<(), String> {
    if stop_slot(&app, Slot::Training) {
        emit_log(&app, "training", "system", "Stop requested.");
        Ok(())
    } else {
        Err("No training job is running.".into())
    }
}

#[tauri::command]
fn update_repo(app: AppHandle, state: State<'_, AppState>) -> Result<u32, String> {
    let root = state
        .repo_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "ai-toolkit folder has not been set.".to_string())?;

    let mut cmd = Command::new("git");
    cmd.args(["pull", "--ff-only"]).current_dir(&root);
    emit_log(&app, "utility", "system", "Running: git pull --ff-only");
    spawn_and_stream(app, Slot::Utility, cmd, "Utility task")
}

#[tauri::command]
fn install_deps(app: AppHandle, state: State<'_, AppState>) -> Result<u32, String> {
    let root = state
        .repo_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "ai-toolkit folder has not been set.".to_string())?;
    let py = python_cmd(&root);

    let mut cmd = Command::new(&py);
    cmd.args(["-m", "pip", "install", "-U", "-r", "requirements.txt"])
        .current_dir(&root);
    emit_log(
        &app,
        "utility",
        "system",
        format!("Running: {} -m pip install -U -r requirements.txt", py),
    );
    spawn_and_stream(app, Slot::Utility, cmd, "Utility task")
}

#[tauri::command]
fn stop_utility(app: AppHandle) -> Result<(), String> {
    if stop_slot(&app, Slot::Utility) {
        emit_log(&app, "utility", "system", "Stop requested.");
        Ok(())
    } else {
        Err("No utility task is running.".into())
    }
}

fn npm_command(root: &Path, script: &str) -> Command {
    let ui_dir = root.join("ui");
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "npm", "run", script]).current_dir(ui_dir);
        cmd
    } else {
        let mut cmd = Command::new("npm");
        cmd.args(["run", script]).current_dir(ui_dir);
        cmd
    }
}

#[tauri::command]
fn launch_web_ui(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let root = state
        .repo_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "ai-toolkit folder has not been set.".to_string())?;

    if !root.join("ui").join("package.json").is_file() {
        return Err("The ai-toolkit ui/ folder is missing package.json.".into());
    }

    let node_modules = root.join("ui").join("node_modules");
    let next_build = root.join("ui").join(".next");
    let script = if node_modules.is_dir() && next_build.is_dir() {
        "start"
    } else {
        "build_and_start"
    };

    let cmd = npm_command(&root, script);
    emit_log(
        &app,
        "web-ui",
        "system",
        format!("Running: npm run {script} (in ui/)"),
    );
    emit_log(
        &app,
        "web-ui",
        "system",
        format!(
            "First-time launch can take several minutes. When you see 'ready' in the log, click 'Open in browser' to visit {WEB_UI_URL}."
        ),
    );
    spawn_and_stream(app.clone(), Slot::WebUi, cmd, "Web UI")?;
    Ok(WEB_UI_URL.to_string())
}

#[tauri::command]
fn stop_web_ui(app: AppHandle) -> Result<(), String> {
    if stop_slot(&app, Slot::WebUi) {
        emit_log(&app, "web-ui", "system", "Stop requested.");
        Ok(())
    } else {
        Err("The Web UI is not running.".into())
    }
}

fn os_open<S: AsRef<OsStr>>(target: S) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(target).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(target)
            .spawn()?;
    }
    Ok(())
}

#[tauri::command]
fn open_path(state: State<'_, AppState>, rel_path: String) -> Result<(), String> {
    let root = state
        .repo_root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "ai-toolkit folder has not been set.".to_string())?;

    let rel = Path::new(&rel_path);
    if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("Only paths inside the ai-toolkit folder can be opened.".into());
    }

    let target = root.join(rel);
    if !target.exists() {
        if target == root.join("output") || target == root.join("datasets") {
            fs::create_dir_all(&target).map_err(|e| format!("Could not create {}: {e}", target.display()))?;
        } else {
            return Err(format!("Path does not exist: {}", target.display()));
        }
    }
    os_open(&target).map_err(|e| format!("Could not open: {e}"))
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Only http:// and https:// URLs can be opened.".into());
    }
    os_open(&url).map_err(|e| format!("Could not open URL: {e}"))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            training: Mutex::new(None),
            utility: Mutex::new(None),
            web_ui: Mutex::new(None),
            repo_root: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            detect_repo_root,
            set_repo_root,
            get_repo_root,
            list_configs,
            check_environment,
            is_running,
            is_utility_running,
            is_web_ui_running,
            start_training,
            stop_training,
            update_repo,
            install_deps,
            stop_utility,
            launch_web_ui,
            stop_web_ui,
            open_path,
            open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
