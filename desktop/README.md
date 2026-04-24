# AI Toolkit Desktop

A friendly Tauri + Rust desktop app for launching `ai-toolkit` training jobs without using the terminal. Built for people who would rather click "Start" than type `python run.py config/my_lora.yaml`.

## What it does

- Auto-detects (or lets you browse for) your `ai-toolkit` folder
- Lists the training configs in `config/` and `config/examples/`
- One big button to start a job, one to stop it
- Streams the training logs into a built-in window

It is **only a launcher** — it does not install Python, PyTorch, or the rest of the training stack. Those still have to be set up the usual way.

## Realistic expectations for non-technical users

Training diffusion models needs:

- An NVIDIA GPU with enough VRAM for the model being trained (e.g. FLUX LoRA wants 24 GB)
- Python with CUDA-enabled PyTorch installed
- A dataset of images and captions laid out in the format the config expects

This app makes **running** a training job push-button, but a technical helper still has to do the one-time setup and prepare the configs and dataset.

## One-time setup (the helper does this)

1. Clone and set up the ai-toolkit repo as usual (install Python deps, create `venv`, etc.). See the project root `README.md`.
2. Install the Rust toolchain and Tauri prerequisites:
   - Rust: <https://rustup.rs>
   - System deps: <https://v2.tauri.app/start/prerequisites/>
3. Install the Tauri CLI:
   ```bash
   cd desktop
   npm install
   ```
4. Generate real application icons (optional but recommended):
   ```bash
   npx tauri icon path/to/logo.png
   ```
5. Run it in dev mode:
   ```bash
   npm run dev
   ```
6. Build a release installer (`.msi`/`.dmg`/`.AppImage`/`.deb`):
   ```bash
   npm run build
   ```
   The installer is placed in `desktop/src-tauri/target/release/bundle/`.

Hand the installer to the user; after that, they just double-click the app.

## How it finds things

- **ai-toolkit folder:** on launch the app walks up from its install directory looking for a folder that contains `run.py` and `toolkit/`. If it can't find one, click "Choose ai-toolkit folder" and pick it.
- **Python:** prefers a `venv/` inside the ai-toolkit folder (`venv/bin/python` or `venv/Scripts/python.exe` on Windows), and falls back to `python` on `PATH`.
- **GPU:** runs `nvidia-smi` to display the GPU name; absence shows a warning but doesn't block you (configs that don't need CUDA still work).

## Layout

```
desktop/
├── package.json          # just pulls in the Tauri CLI
├── src-tauri/            # Rust backend + Tauri config
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── icons/
│   └── src/main.rs
└── ui/                   # static HTML/CSS/JS frontend (no build step)
    ├── index.html
    ├── styles.css
    └── main.js
```

## Packaging a double-click portable bundle for non-technical users

If the end user cannot install Python, Rust, or npm — they can only unzip a folder and double-click — build a portable bundle on your own machine that contains everything pre-installed. Your parents (or whoever) then unzip it once and run it with no further setup.

### What you build (once per OS)

```
AI-Toolkit-Portable/
├── AI-Toolkit(.exe)        <- the Tauri launcher, pre-compiled
├── portable.flag           <- makes the launcher hide update/web-ui buttons
├── run.py, toolkit/, ...   <- the ai-toolkit source
├── config/                 <- put curated recipes here before zipping
├── output/, datasets/      <- created empty for the user
└── python-portable/        <- standalone Python + every dependency pre-installed
    ├── bin/python3 (or python.exe)
    └── lib/...
```

The user unzips that folder anywhere and double-clicks `AI-Toolkit`. The launcher auto-detects the portable layout, uses the bundled Python, and hides the "Update / Install Python packages / Launch Web UI" buttons (which don't apply in a pre-bundled setup).

### Steps on the build machine (same OS as the target)

> **Windows note:** the build must run on a Windows PC (cross-compiling Tauri to Windows is impractical). Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the "Desktop development with C++" workload, [Rust](https://rustup.rs/), and [Node.js LTS](https://nodejs.org/) before starting. While the build runs (especially the `pip install` step), temporarily add `desktop\` and your system temp folder to Windows Defender exclusions — real-time scanning makes the PyTorch install 5–10x slower otherwise.

> **Windows WebView2:** the launcher bundles a fixed WebView2 runtime so the zip works on any Windows 10/11 even without Edge. Download the [WebView2 Fixed Version Runtime](https://developer.microsoft.com/microsoft-edge/webview2/?form=MA13LH#download-section) (the "Fixed Version" CAB, x64), extract it so its contents live at `desktop/src-tauri/webview2-runtime/Microsoft.WebView2.FixedVersionRuntime.<version>.x64/`. The `tauri.conf.json` `bundle.windows.webviewInstallMode` already points there.

1. Build the Tauri launcher:
   ```bash
   cd desktop
   npm install
   npm run build
   ```
   The binary ends up at `desktop/src-tauri/target/release/ai-toolkit-desktop` (or `.exe`).
2. Download a standalone Python from [python-build-standalone](https://github.com/astral-sh/python-build-standalone/releases). Pick a recent **CPython 3.11 or 3.12** `install_only` tarball for your platform:
   - Linux: `cpython-3.12.*-x86_64-unknown-linux-gnu-install_only.tar.gz`
   - macOS Apple Silicon: `cpython-3.12.*-aarch64-apple-darwin-install_only.tar.gz`
   - macOS Intel: `cpython-3.12.*-x86_64-apple-darwin-install_only.tar.gz`
   - Windows: `cpython-3.12.*-x86_64-pc-windows-msvc-install_only.tar.gz`
3. Run the packager:
   ```bash
   python desktop/build-portable.py \
       --python-tarball path/to/cpython-3.12.*-install_only.tar.gz \
       --tauri-bin desktop/src-tauri/target/release/ai-toolkit-desktop \
       --output AI-Toolkit-Portable-Linux.zip
   ```
   This copies the ai-toolkit source into a staging folder, extracts the standalone Python into `python-portable/`, runs `pip install -r requirements.txt` into that Python, copies the launcher, writes `portable.flag`, and zips it up.

   The step takes a while (~5-15 min) and produces an **8-15 GB zip** — PyTorch's bundled CUDA wheels are the bulk of the size. That's unavoidable for an offline AI-training bundle.
4. Drop the zip in a cloud share (Dropbox / Google Drive / Mega) and send the link.

### What the end user does

1. Download the zip.
2. Right-click → "Extract All" / "Unzip" to a **short path** like `C:\AIToolkit\` (NOT inside `Documents\Downloads\…`). Some Python packages on Windows break when nested paths exceed 260 characters.
3. Open the resulting folder and double-click the launcher.
4. **Windows SmartScreen** will say "Windows protected your PC" the first time — click **More info → Run anyway**. macOS may show a similar warning — right-click → **Open** instead of double-clicking, then confirm.
5. If Windows Defender quarantines the launcher, open Windows Security → Virus & threat protection → Protection history, find the entry, and click **Allow on device**.
6. In the GUI: pick a training recipe, click **Start training**, watch the log.

No terminal, no installer, no package manager, no GitHub. The bundled `README-FOR-USERS.txt` walks them through the same steps.

### Things to curate before packaging

- Delete config files from `config/examples/` the user won't need, or pre-stage a few ready-to-run recipes at the top level of `config/` that point to `datasets/my-images/` (a folder the packager creates empty for them).
- If the recipe downloads base models from HuggingFace the first time it runs, the user needs an internet connection for that one initial download. To go fully offline, run the recipe yourself on the build machine so HuggingFace caches the weights, then copy `~/.cache/huggingface/` into the portable bundle (this is large — another 20+ GB for FLUX).

## Limitations

- On macOS and Linux the app spawns Python in its own process group so Stop kills the whole tree; on Windows it uses `taskkill /T /F`. If a training crash leaves orphan processes, use Task Manager / `ps` to clean up.
- No persistent settings yet — the chosen folder resets on relaunch. Re-detection usually finds it again immediately.
- This is a thin launcher. For dataset management, advanced job editing, sample previews, etc., use the existing Next.js `ui/` app at the project root.
