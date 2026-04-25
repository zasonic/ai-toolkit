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
        help=(
            "Filename for the launcher in the zip. Defaults to a name that "
            "screams at the end user: 'START HERE - Windows.exe' on Windows, "
            "'START HERE - Mac' on macOS, 'START HERE - Linux' elsewhere."
        ),
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
    is_mac = platform.system().lower() == "darwin"
    if args.launcher_name:
        launcher_name = args.launcher_name
    elif is_windows:
        launcher_name = "START HERE - Windows.exe"
    elif is_mac:
        launcher_name = "START HERE - Mac"
    else:
        launcher_name = "START HERE - Linux"

    with tempfile.TemporaryDirectory(prefix="ai-toolkit-portable-") as td:
        staging = Path(td) / "AI-Toolkit-Portable"
        staging.mkdir()
        # Bury the source, the bundled Python, output/, datasets/ and the
        # portable.flag inside _internal/. The end user only sees the launcher
        # and READ ME FIRST.txt at the top of the folder; the launcher walks
        # into _internal/ to find run.py.
        internal = staging / "_internal"
        internal.mkdir()

        log(f"[1/6] Copying ai-toolkit source into {internal} ...")
        copy_source(repo_root, internal)

        log("[2/6] Extracting portable Python ...")
        py_dir = internal / "python-portable"
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
            internal,
            (args.torch_version, args.torchvision_version, args.torchaudio_version),
            args.torch_index_url,
        )

        log(f"[4/6] Copying Tauri launcher as {launcher_name} ...")
        target_launcher = staging / launcher_name
        shutil.copy2(args.tauri_bin, target_launcher)
        if not is_windows:
            target_launcher.chmod(0o755)

        log("[5/6] Writing portable.flag and 'READ ME FIRST.txt' ...")
        (internal / "portable.flag").write_text("1\n")
        (internal / "output").mkdir(exist_ok=True)
        (internal / "datasets").mkdir(exist_ok=True)
        readme_lines = [
            "AI TOOLKIT  ---  read this once, then never again",
            "================================================",
            "",
            "STEP 1.  Make sure this folder is somewhere with a SHORT path,",
            "         for example:    C:\\AIToolkit\\",
            "         NOT inside Downloads or Documents (paths get too long).",
            "",
            f"STEP 2.  Double-click  \"{launcher_name}\".",
            "         The app window opens. That is the whole setup.",
            "",
            "STEP 3.  Inside the window, pick a recipe from the list and",
            "         click the big blue  \"Start training\"  button.",
            "",
            "",
            "FIRST-TIME WARNINGS  (only the first time you launch)",
            "-----------------------------------------------------",
            "  * Blue \"Windows protected your PC\" box?",
            "      Click  \"More info\"  ->  \"Run anyway\".",
            "  * Defender quarantined the app?",
            "      Open  Windows Security  ->  Virus & threat protection",
            "      ->  Protection history,  find it,  click  \"Allow on device\".",
            "",
            "WHERE THINGS LIVE",
            "-----------------",
            "  Trained models      ->  output\\",
            "  Your training images ->  datasets\\",
            "  Training recipes     ->  config\\  (use the buttons in the app)",
            "",
            "TROUBLE?",
            "--------",
            "  * Slow to unzip?  Use 7-Zip (free, https://www.7-zip.org/).",
            "  * App will not open?  Check you have an NVIDIA GPU and that this",
            "    folder lives at a short path like C:\\AIToolkit\\.",
            "  * Anything else?  Take a screenshot and send it to whoever",
            "    set this up for you.",
            "",
            "Everything is bundled. No install. No internet needed (after",
            "the first model download, if your recipe needs one).",
            "",
        ]
        (staging / "READ ME FIRST.txt").write_text("\r\n".join(readme_lines))

        log(f"[6/6] Creating zip at {args.output} ...")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        zip_dir(staging, args.output)

    size_gb = args.output.stat().st_size / 1e9
    log(f"Done: {args.output} ({size_gb:.2f} GB)")
    log("Share the zip with your users. They unzip it and double-click the launcher.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
