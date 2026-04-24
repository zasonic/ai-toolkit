const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// tauri-plugin-dialog's JS bindings live in the @tauri-apps/plugin-dialog
// npm package and are NOT auto-injected by withGlobalTauri. Call the plugin
// command directly through invoke() so we don't need a JS bundler.
async function pickDirectory() {
  return invoke("plugin:dialog|open", {
    options: { directory: true, multiple: false },
  });
}

const $ = (id) => document.getElementById(id);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

const els = {
  portableBanner: $("portable-banner"),
  portableRow: $("portable-row"),
  cardMaintenance: $("card-maintenance"),
  cardWebUi: $("card-web-ui"),

  statusRoot: $("status-root"),
  statusPython: $("status-python"),
  statusGpu: $("status-gpu"),
  statusNode: $("status-node"),
  statusGit: $("status-git"),
  setupHint: $("setup-hint"),
  pickFolder: $("btn-pick-folder"),
  recheck: $("btn-recheck"),
  recheckPortable: $("btn-recheck-portable"),

  configSelect: $("config-select"),
  start: $("btn-start"),
  stop: $("btn-stop"),
  runStatus: $("run-status"),

  updateRepo: $("btn-update-repo"),
  installDeps: $("btn-install-deps"),
  stopUtility: $("btn-stop-utility"),
  utilityStatus: $("utility-status"),

  launchWeb: $("btn-launch-web"),
  openWeb: $("btn-open-web"),
  stopWeb: $("btn-stop-web"),
  webStatus: $("web-status"),

  clearLog: $("btn-clear-log"),
  log: $("log"),
};

let state = {
  repoRoot: null,
  selectedConfig: null,
  training: false,
  utility: false,
  webUi: false,
  webUiUrl: "http://localhost:8675",
  portable: false,
};

const filters = {
  training: true,
  utility: true,
  "web-ui": true,
  system: true,
};

function setStatus(dd, text, cls) {
  dd.textContent = text;
  dd.classList.remove("ok", "err", "warn");
  if (cls) dd.classList.add(cls);
}

function appendLog(source, stream, line) {
  const el = document.createElement("span");
  el.className = `line source-${source} stream-${stream}`;
  el.dataset.source = source;
  el.textContent = `[${source}] ${line}\n`;
  if (!filters[source]) el.style.display = "none";
  els.log.appendChild(el);
  while (els.log.childElementCount > 5000) {
    els.log.removeChild(els.log.firstChild);
  }
  els.log.scrollTop = els.log.scrollHeight;
}

function applyFilter(source) {
  for (const node of els.log.querySelectorAll(`.source-${source}`)) {
    node.style.display = filters[source] ? "" : "none";
  }
  els.log.scrollTop = els.log.scrollHeight;
}

function updateButtons() {
  els.start.disabled = !state.repoRoot || !state.selectedConfig || state.training;
  els.stop.disabled = !state.training;
  els.runStatus.textContent = state.training
    ? "Running training job. You can stop it at any time."
    : "Idle.";

  els.updateRepo.disabled = !state.repoRoot || state.utility;
  els.installDeps.disabled = !state.repoRoot || state.utility;
  els.stopUtility.disabled = !state.utility;
  els.utilityStatus.textContent = state.utility
    ? "Maintenance task running. See log below."
    : "No maintenance task running.";

  els.launchWeb.disabled = !state.repoRoot || state.webUi;
  els.openWeb.disabled = !state.webUi;
  els.stopWeb.disabled = !state.webUi;
  els.webStatus.textContent = state.webUi
    ? `Web UI process started at ${state.webUiUrl}. When the log shows 'ready', click "Open in browser".`
    : "Web UI is not running.";
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
    appendLog("system", "stderr", `Failed to list configs: ${e}`);
  }
}

function applyPortableMode(isPortable) {
  state.portable = isPortable;
  els.portableBanner.hidden = !isPortable;
  els.cardMaintenance.hidden = isPortable;
  els.cardWebUi.hidden = isPortable;
  // Swap the Setup action row: portable users shouldn't reassign the folder.
  els.pickFolder.parentElement.hidden = isPortable;
  els.portableRow.hidden = !isPortable;
}

async function refreshStatus() {
  const env = await invoke("check_environment");

  applyPortableMode(!!env.is_portable);

  state.repoRoot = env.repo_root || null;
  if (env.repo_root) {
    setStatus(els.statusRoot, env.repo_root, "ok");
  } else {
    setStatus(els.statusRoot, "not set - click 'Choose ai-toolkit folder'", "err");
  }

  if (env.python) {
    const m = env.python.match(/(\d+)\.(\d+)/);
    if (m && (parseInt(m[1], 10) < 3 || (parseInt(m[1], 10) === 3 && parseInt(m[2], 10) < 10))) {
      setStatus(els.statusPython, `${env.python} - too old, ai-toolkit needs Python 3.10 or newer`, "err");
    } else {
      setStatus(els.statusPython, env.python, "ok");
    }
  } else {
    setStatus(els.statusPython, "python not found on PATH (or venv missing)", "err");
  }

  if (env.gpu) setStatus(els.statusGpu, env.gpu, "ok");
  else setStatus(els.statusGpu, "no NVIDIA GPU detected (nvidia-smi not found)", "warn");

  // In portable mode we don't show the Node/git rows' warnings since those
  // buttons are hidden anyway.
  if (env.node) setStatus(els.statusNode, env.node, "ok");
  else setStatus(els.statusNode, state.portable ? "not needed in portable mode" : "Node.js not found - Web UI launch will fail", state.portable ? "ok" : "warn");

  if (env.git) setStatus(els.statusGit, env.git, "ok");
  else setStatus(els.statusGit, state.portable ? "not needed in portable mode" : "git not found - Update button will fail", state.portable ? "ok" : "warn");

  const blockers = [];
  if (!env.repo_root) blockers.push("the ai-toolkit folder");
  if (!env.python) blockers.push("Python");
  if (!env.gpu) blockers.push("an NVIDIA GPU");
  els.setupHint.textContent = blockers.length
    ? `Still missing: ${blockers.join(", ")}.`
    : state.portable
      ? "You are all set. Pick a recipe and press Start."
      : "Looks good. Pick a recipe above or use the shortcuts below.";

  await refreshConfigs();

  state.training = await invoke("is_running");
  state.utility = await invoke("is_utility_running");
  state.webUi = await invoke("is_web_ui_running");
  updateButtons();
}

async function pickFolder() {
  let selected;
  try {
    selected = await pickDirectory();
  } catch (e) {
    appendLog("system", "stderr", `Could not open folder picker: ${e}`);
    return;
  }
  if (!selected) return;
  try {
    const root = await invoke("set_repo_root", { path: selected });
    appendLog("system", "system", `ai-toolkit folder set to ${root}`);
    await refreshStatus();
  } catch (e) {
    appendLog("system", "stderr", `Could not use that folder: ${e}`);
  }
}

async function startTraining() {
  if (!state.selectedConfig) return;
  try {
    state.training = true;
    updateButtons();
    await invoke("start_training", { configPath: state.selectedConfig });
  } catch (e) {
    state.training = false;
    appendLog("training", "stderr", `Could not start: ${e}`);
    updateButtons();
  }
}

async function stopTraining() {
  try { await invoke("stop_training"); }
  catch (e) { appendLog("training", "stderr", `Could not stop: ${e}`); }
}

async function updateRepo() {
  try {
    state.utility = true;
    updateButtons();
    await invoke("update_repo");
  } catch (e) {
    state.utility = false;
    appendLog("utility", "stderr", `Could not update: ${e}`);
    updateButtons();
  }
}

async function installDeps() {
  try {
    state.utility = true;
    updateButtons();
    await invoke("install_deps");
  } catch (e) {
    state.utility = false;
    appendLog("utility", "stderr", `Could not install: ${e}`);
    updateButtons();
  }
}

async function stopUtility() {
  try { await invoke("stop_utility"); }
  catch (e) { appendLog("utility", "stderr", `Could not stop: ${e}`); }
}

async function launchWebUi() {
  try {
    state.webUi = true;
    updateButtons();
    const url = await invoke("launch_web_ui");
    state.webUiUrl = url;
    updateButtons();
  } catch (e) {
    state.webUi = false;
    appendLog("web-ui", "stderr", `Could not launch Web UI: ${e}`);
    updateButtons();
  }
}

async function openWebInBrowser() {
  try { await invoke("open_url", { url: state.webUiUrl }); }
  catch (e) { appendLog("system", "stderr", `Could not open browser: ${e}`); }
}

async function stopWebUi() {
  try { await invoke("stop_web_ui"); }
  catch (e) { appendLog("web-ui", "stderr", `Could not stop Web UI: ${e}`); }
}

async function openShortcut(relPath) {
  try { await invoke("open_path", { relPath }); }
  catch (e) { appendLog("system", "stderr", `Could not open folder: ${e}`); }
}

function wireEvents() {
  els.pickFolder.addEventListener("click", pickFolder);
  els.recheck.addEventListener("click", refreshStatus);
  els.recheckPortable.addEventListener("click", refreshStatus);
  els.start.addEventListener("click", startTraining);
  els.stop.addEventListener("click", stopTraining);
  els.configSelect.addEventListener("change", () => {
    state.selectedConfig = els.configSelect.value || null;
    updateButtons();
  });

  els.updateRepo.addEventListener("click", updateRepo);
  els.installDeps.addEventListener("click", installDeps);
  els.stopUtility.addEventListener("click", stopUtility);

  els.launchWeb.addEventListener("click", launchWebUi);
  els.openWeb.addEventListener("click", openWebInBrowser);
  els.stopWeb.addEventListener("click", stopWebUi);

  for (const btn of $$(".shortcut")) {
    btn.addEventListener("click", () => openShortcut(btn.dataset.path));
  }

  els.clearLog.addEventListener("click", () => (els.log.innerHTML = ""));

  for (const source of ["training", "utility", "web-ui", "system"]) {
    const cb = $(`filter-${source}`);
    if (!cb) continue;
    cb.addEventListener("change", () => {
      filters[source] = cb.checked;
      applyFilter(source);
    });
  }

  listen("app-log", (e) => {
    const { source, stream, line } = e.payload;
    appendLog(source, stream, line);
  });
  listen("job-end", (e) => {
    const { source } = e.payload;
    if (source === "training") state.training = false;
    else if (source === "utility") state.utility = false;
    else if (source === "web-ui") state.webUi = false;
    updateButtons();
  });
}

async function init() {
  wireEvents();
  try {
    const detected = await invoke("detect_repo_root");
    if (detected) appendLog("system", "system", `Detected ai-toolkit folder at ${detected}`);
  } catch {}
  await refreshStatus();
}

init();
