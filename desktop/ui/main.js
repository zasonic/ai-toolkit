const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open } = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);

const els = {
  statusRoot: $("status-root"),
  statusPython: $("status-python"),
  statusGpu: $("status-gpu"),
  setupHint: $("setup-hint"),
  pickFolder: $("btn-pick-folder"),
  recheck: $("btn-recheck"),
  configSelect: $("config-select"),
  start: $("btn-start"),
  stop: $("btn-stop"),
  clearLog: $("btn-clear-log"),
  runStatus: $("run-status"),
  log: $("log"),
};

let state = {
  repoRoot: null,
  running: false,
  selectedConfig: null,
};

function setStatus(dd, text, cls) {
  dd.textContent = text;
  dd.classList.remove("ok", "err", "warn");
  if (cls) dd.classList.add(cls);
}

function appendLog(stream, line) {
  const el = document.createElement("span");
  el.className = `line ${stream}`;
  el.textContent = line + "\n";
  els.log.appendChild(el);
  // Cap log to ~5000 lines to avoid DOM bloat
  while (els.log.childElementCount > 5000) {
    els.log.removeChild(els.log.firstChild);
  }
  els.log.scrollTop = els.log.scrollHeight;
}

function updateButtons() {
  const canStart =
    !!state.repoRoot && !!state.selectedConfig && !state.running;
  els.start.disabled = !canStart;
  els.stop.disabled = !state.running;
  els.runStatus.textContent = state.running
    ? "Running training job. You can stop it at any time."
    : "Idle.";
}

async function refreshConfigs() {
  els.configSelect.innerHTML = "";
  if (!state.repoRoot) return;
  try {
    const configs = await invoke("list_configs");
    if (configs.length === 0) {
      const opt = document.createElement("option");
      opt.disabled = true;
      opt.textContent = "(no config files found)";
      els.configSelect.appendChild(opt);
      return;
    }
    const groups = {};
    for (const c of configs) {
      (groups[c.group] ||= []).push(c);
    }
    for (const [group, items] of Object.entries(groups)) {
      const og = document.createElement("optgroup");
      og.label = group;
      for (const c of items) {
        const opt = document.createElement("option");
        opt.value = c.path;
        opt.textContent = c.name;
        og.appendChild(opt);
      }
      els.configSelect.appendChild(og);
    }
  } catch (e) {
    appendLog("system", `Failed to list configs: ${e}`);
  }
}

async function refreshStatus() {
  const env = await invoke("check_environment");

  if (env.repo_root) {
    state.repoRoot = env.repo_root;
    setStatus(els.statusRoot, env.repo_root, "ok");
  } else {
    state.repoRoot = null;
    setStatus(els.statusRoot, "not set — click 'Choose ai-toolkit folder'", "err");
  }

  if (env.python) {
    setStatus(els.statusPython, env.python, "ok");
  } else {
    setStatus(els.statusPython, "python not found on PATH (or venv missing)", "err");
  }

  if (env.gpu) {
    setStatus(els.statusGpu, env.gpu, "ok");
  } else {
    setStatus(els.statusGpu, "no NVIDIA GPU detected (nvidia-smi not found)", "warn");
  }

  const missing = [];
  if (!env.repo_root) missing.push("the ai-toolkit folder");
  if (!env.python) missing.push("Python");
  if (!env.gpu) missing.push("an NVIDIA GPU");
  els.setupHint.textContent = missing.length
    ? `Still missing: ${missing.join(", ")}.`
    : "Looks good. You can pick a recipe below.";

  await refreshConfigs();
  state.running = await invoke("is_running");
  updateButtons();
}

async function pickFolder() {
  const selected = await open({ directory: true, multiple: false });
  if (!selected) return;
  try {
    const root = await invoke("set_repo_root", { path: selected });
    appendLog("system", `ai-toolkit folder set to ${root}`);
    await refreshStatus();
  } catch (e) {
    appendLog("system", `Could not use that folder: ${e}`);
  }
}

async function start() {
  if (!state.selectedConfig) return;
  try {
    state.running = true;
    updateButtons();
    await invoke("start_training", { configPath: state.selectedConfig });
  } catch (e) {
    state.running = false;
    appendLog("system", `Could not start: ${e}`);
    updateButtons();
  }
}

async function stop() {
  try {
    await invoke("stop_training");
  } catch (e) {
    appendLog("system", `Could not stop: ${e}`);
  }
}

function wireEvents() {
  els.pickFolder.addEventListener("click", pickFolder);
  els.recheck.addEventListener("click", refreshStatus);
  els.start.addEventListener("click", start);
  els.stop.addEventListener("click", stop);
  els.clearLog.addEventListener("click", () => (els.log.innerHTML = ""));
  els.configSelect.addEventListener("change", () => {
    state.selectedConfig = els.configSelect.value || null;
    updateButtons();
  });

  listen("training-log", (e) => {
    const { stream, line } = e.payload;
    appendLog(stream, line);
  });
  listen("training-exit", () => {
    state.running = false;
    updateButtons();
  });
}

async function init() {
  wireEvents();
  // Try auto-detecting the repo first
  try {
    const detected = await invoke("detect_repo_root");
    if (detected) appendLog("system", `Detected ai-toolkit folder at ${detected}`);
  } catch {}
  await refreshStatus();
}

init();
