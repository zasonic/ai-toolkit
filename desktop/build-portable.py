#!/usr/bin/env python3
"""Package a double-click portable AI Toolkit zip for non-technical users.

Run this on the SAME OS/architecture the zip is for (Python packages are
platform-specific). Output: a single .zip that the end user unzips and
double-clicks. No install, no terminal, no network required after unzip.

Prereqs (one-time on the build machine):
  1) Build the Tauri launcher:
         cd desktop && npm install && npm run build
     This produces a binary under desktop/src-tauri/target/release/.
  2) Download the "install_only" tarball of python-build-standalone for your
     target platform (https://github.com/astral-sh/python-build-standalone/releases).
     Pick a recent CPython 3.11 or 3.12 build.

Example (Linux x64):
  python desktop/build-portable.py \
      --python-tarball ~/Downloads/cpython-3.12.7+20241016-x86_64-unknown-linux-gnu-install_only.tar.gz \
      --tauri-bin desktop/src-tauri/target/release/ai-toolkit-desktop \
      --output /tmp/AI-Toolkit-Portable-Linux.zip

Example (Windows x64):
  python desktop\\build-portable.py ^
      --python-tarball C:\\Users\\you\\Downloads\\cpython-3.12.7+...-x86_64-pc-windows-msvc-install_only.tar.gz ^
      --tauri-bin desktop\\src-tauri\\target\\release\\ai-toolkit-desktop.exe ^
      --output AI-Toolkit-Portable-Windows.zip

Resulting zip is typically 8-15 GB (PyTorch + CUDA wheels). Upload to a file
host (Dropbox, Google Drive, Mega, etc.) and share the link with your parents.
"""

from __future__ import annotations

import argparse
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

SRC_FILES = [
    "run.py",
    "requirements.txt",
    "requirements_base.txt",
    "info.py",
    "version.py",
    "README.md",
    "LICENSE",
    "FAQ.md",
]
SRC_DIRS = [
    "toolkit",
    "extensions",
    "extensions_built_in",
    "jobs",
    "scripts",
    "testing",
    "config",
    "assets",
]
IGNORED_NAMES = {
    ".git",
    ".github",
    "node_modules",
    "__pycache__",
    ".pytest_cache",
    ".venv",
    "venv",
    "output",
    "datasets",
    "dist",
    "build",
    ".next",
    "target",
    ".DS_Store",
}


def ignore_patterns(_dir: str, names: list[str]) -> list[str]:
    return [n for n in names if n in IGNORED_NAMES or n.endswith(".pyc")]


def log(msg: str) -> None:
    print(f"[build-portable] {msg}", flush=True)


def find_portable_python(py_dir: Path) -> Path:
    candidates = [
        py_dir / "bin" / "python3",
        py_dir / "bin" / "python",
        py_dir / "python.exe",
        py_dir / "install" / "bin" / "python3",
    ]
    for c in candidates:
        if c.is_file():
            return c
    raise FileNotFoundError(
        f"Could not find the python interpreter under {py_dir}. "
        f"Layout: {[p.name for p in py_dir.iterdir()]}"
    )


def flatten_python_tarball_root(py_dir: Path) -> None:
    """python-build-standalone's install_only tarball extracts to python/ by default.
    Flatten one level so our launcher finds bin/ or python.exe directly under python-portable/.
    """
    entries = [p for p in py_dir.iterdir() if not p.name.startswith(".")]
    if len(entries) == 1 and entries[0].is_dir() and entries[0].name.lower() == "python":
        inner = entries[0]
        for child in list(inner.iterdir()):
            shutil.move(str(child), str(py_dir / child.name))
        inner.rmdir()


def extract_tarball(tarball: Path, dest: Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    with tarfile.open(tarball, "r:*") as tf:
        # Python 3.12+ requires an extraction filter or warns loudly.
        # 'data' blocks absolute paths, path traversal, device nodes, etc.
        try:
            tf.extractall(dest, filter="data")
        except TypeError:
            tf.extractall(dest)


def copy_source(repo_root: Path, staging: Path) -> None:
    for name in SRC_FILES:
        src = repo_root / name
        if src.is_file():
            shutil.copy2(src, staging / name)
    for dname in SRC_DIRS:
        src = repo_root / dname
        if src.is_dir():
            shutil.copytree(src, staging / dname, ignore=ignore_patterns)


def install_requirements(
    py_exe: Path,
    staging: Path,
    torch_versions: tuple[str, str, str],
    torch_index_url: str,
) -> None:
    # 1. Bootstrap pip + build tools.
    subprocess.check_call(
        [str(py_exe), "-m", "pip", "install", "--upgrade", "pip", "wheel", "setuptools"]
    )
    # 2. Install CUDA-enabled torch FIRST from PyTorch's wheel index. ai-toolkit's
    #    requirements.txt does not pin torch; if we skip this step, transitive
    #    deps pull a CPU-only torch and the bundle silently cannot use the GPU.
    torch, torchvision, torchaudio = torch_versions
    subprocess.check_call(
        [
            str(py_exe),
            "-m",
            "pip",
            "install",
            "--no-cache-dir",
            f"torch=={torch}",
            f"torchvision=={torchvision}",
            f"torchaudio=={torchaudio}",
            "--index-url",
            torch_index_url,
        ]
    )
    # 3. Install everything else.
    subprocess.check_call(
        [
            str(py_exe),
            "-m",
            "pip",
            "install",
            "--no-cache-dir",
            "-r",
            str(staging / "requirements.txt"),
        ]
    )


def zip_dir(source: Path, output: Path) -> None:
    total = sum(f.stat().st_size for f in source.rglob("*") if f.is_file())
    log(f"Zipping {total / 1e9:.2f} GB of files (stored, no recompression) ...")
    # Compressing PyTorch wheels that are already zlib-compressed is a waste.
    # ZIP_STORED is fast and close to optimal for this payload.
    with zipfile.ZipFile(output, "w", zipfile.ZIP_STORED, allowZip64=True) as zf:
        for f in source.rglob("*"):
            if f.is_file():
                zf.write(f, f.relative_to(source.parent))


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--python-tarball", required=True, type=Path)
    parser.add_argument("--tauri-bin", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--launcher-name",
        default=None,
        help="Filename for the launcher in the zip. Default: 'AI-Toolkit' (+'.exe' on Windows).",
    )
    # Defaults match ai-toolkit/README.md; bump them when ai-toolkit pins newer torch.
    parser.add_argument("--torch-version", default="2.9.1")
    parser.add_argument("--torchvision-version", default="0.24.1")
    parser.add_argument("--torchaudio-version", default="2.9.1")
    parser.add_argument(
        "--torch-index-url",
        default="https://download.pytorch.org/whl/cu128",
        help="PyTorch wheel index. Defaults to CUDA 12.8 builds. "
             "Use https://download.pytorch.org/whl/cu121 for older drivers, "
             "or https://download.pytorch.org/whl/cpu for CPU-only.",
    )
    args = parser.parse_args()

    if not args.python_tarball.is_file():
        print(f"Python tarball not found: {args.python_tarball}", file=sys.stderr)
        return 2
    if not args.tauri_bin.is_file():
        print(f"Tauri binary not found: {args.tauri_bin}", file=sys.stderr)
        return 2

    # ai-toolkit's requirements_base.txt installs diffusers via `git+https://...`,
    # so the build machine needs git on PATH. Fail early with a clear message.
    if shutil.which("git") is None:
        print(
            "git was not found on PATH. The build needs git because\n"
            "requirements_base.txt installs diffusers from a git URL.\n"
            "Install Git for Windows (https://git-scm.com/download/win) and re-run.",
            file=sys.stderr,
        )
        return 2

    repo_root = Path(__file__).resolve().parent.parent
    if not (repo_root / "run.py").is_file():
        print(f"Could not find run.py under {repo_root}", file=sys.stderr)
        return 2

    is_windows = platform.system().lower().startswith("win")
    launcher_name = args.launcher_name or ("AI-Toolkit.exe" if is_windows else "AI-Toolkit")

    with tempfile.TemporaryDirectory(prefix="ai-toolkit-portable-") as td:
        staging = Path(td) / "AI-Toolkit-Portable"
        staging.mkdir()

        log(f"[1/6] Copying ai-toolkit source into {staging} ...")
        copy_source(repo_root, staging)

        log("[2/6] Extracting portable Python ...")
        py_dir = staging / "python-portable"
        extract_tarball(args.python_tarball, py_dir)
        flatten_python_tarball_root(py_dir)
        py_exe = find_portable_python(py_dir)
        log(f"       python: {py_exe}")

        log(
            f"[3/6] Installing Python packages (CUDA torch first from "
            f"{args.torch_index_url}, then requirements.txt; this takes a while) ..."
        )
        install_requirements(
            py_exe,
            staging,
            (args.torch_version, args.torchvision_version, args.torchaudio_version),
            args.torch_index_url,
        )

        log(f"[4/6] Copying Tauri launcher as {launcher_name} ...")
        target_launcher = staging / launcher_name
        shutil.copy2(args.tauri_bin, target_launcher)
        if not is_windows:
            target_launcher.chmod(0o755)

        log("[5/6] Writing portable.flag and user README ...")
        (staging / "portable.flag").write_text("1\n")
        (staging / "output").mkdir(exist_ok=True)
        (staging / "datasets").mkdir(exist_ok=True)
        (staging / "README-FOR-USERS.txt").write_text(
            "AI Toolkit (portable)\n"
            "=====================\n\n"
            "BEFORE YOU START\n"
            "----------------\n"
            "Unzip this folder somewhere with a SHORT path, like:\n"
            "    C:\\AIToolkit\\\n"
            "    D:\\AIToolkit\\\n"
            "Do NOT leave it inside Downloads or Documents. Some Python packages\n"
            "break when the path is too long (Windows 260-character limit).\n\n"
            "TO RUN\n"
            "------\n"
            f"1. Double-click \"{launcher_name}\".\n"
            "2. The first time only, Windows may show a blue\n"
            "   \"Windows protected your PC\" box (SmartScreen). This is normal\n"
            "   for any app that is not signed. Click \"More info\", then\n"
            "   \"Run anyway\". Windows will remember and not ask again.\n"
            "3. If Windows Defender or another antivirus quarantines the\n"
            "   launcher, open Windows Security -> Virus & threat protection\n"
            "   -> Protection history, find the entry, and choose\n"
            "   \"Allow on device\".\n"
            "4. In the app, pick a training recipe from the list and press\n"
            "   the blue Start training button.\n\n"
            "WHERE THINGS LIVE\n"
            "-----------------\n"
            "  - Trained models are saved in   output\\\n"
            "  - Put your training images in   datasets\\\n"
            "  - Training recipes (configs) in config\\\n"
            "Use the buttons in the app to open these folders -\n"
            "you do not need File Explorer.\n\n"
            "TROUBLESHOOTING\n"
            "---------------\n"
            "  - Slow unzip? Use 7-Zip (free, https://www.7-zip.org/)\n"
            "    instead of the built-in Windows unzipper.\n"
            "  - Launcher does not open? Make sure you have an NVIDIA GPU\n"
            "    and that the path is short (see above).\n"
            "  - Anything else? Take a screenshot and send it to whoever\n"
            "    set this up for you.\n\n"
            "This bundle is self-contained. No installation required.\n"
        )

        log(f"[6/6] Creating zip at {args.output} ...")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        zip_dir(staging, args.output)

    size_gb = args.output.stat().st_size / 1e9
    log(f"Done: {args.output} ({size_gb:.2f} GB)")
    log("Share the zip with your users. They unzip it and double-click the launcher.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
