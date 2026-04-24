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

## Limitations

- On macOS and Linux the app spawns Python in its own process group so Stop kills the whole tree; on Windows it uses `taskkill /T /F`. If a training crash leaves orphan processes, use Task Manager / `ps` to clean up.
- No persistent settings yet — the chosen folder resets on relaunch. Re-detection usually finds it again immediately.
- This is a thin launcher. For dataset management, advanced job editing, sample previews, etc., use the existing Next.js `ui/` app at the project root.
