"""Build/verify the frozen research resources; never includes StockDB or user accounts."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
MANIFESTS = ("research/research-center-runner/registry.json", "research/ashare-fundamental-2026-10-01/replay-sources.json")
PROGRAMS = (*MANIFESTS, "research/research-center-runner/model_runner.py", "research/research-center-runner/refresh_market.py", "research/research-center-runner/recent_market_fallback.py", "research/research-center-runner/follow_context.py", "research/ashare-fundamental-2026-10-01/replay_adapter.py", "research/nested-technical-v1/search.py")
PYTHON = "3.12.14"


def digest(file):
    with Path(file).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def safe_name(name):
    path = PurePosixPath(name)
    if not name.startswith("research/") or "\\" in name or ":" in name or any(part in ("", ".", "..") for part in name.split("/")) or path.is_absolute():
        raise ValueError("unsafe research path: " + name)
    return name


def expected_files(root):
    files = {}
    for manifest in MANIFESTS:
        for row in json.loads((root/manifest).read_text(encoding="utf8"))["files"].values():
            name = safe_name(row["path"])
            if name in files and files[name] != row["sha256"]:
                raise ValueError("conflicting frozen fingerprints: " + name)
            files[name] = row["sha256"]
    files.update(json.loads((ROOT/"research-assets.json").read_text(encoding="utf8"))["additional_files"])
    for name in PROGRAMS:
        files[name] = digest(root/name)
    return files


def verify(root, files):
    for name, sha in files.items():
        file = root/safe_name(name)
        if file.is_symlink() or not file.is_file() or not file.resolve().is_relative_to(root.resolve()) or digest(file) != sha:
            raise ValueError("missing/modified frozen resource: " + name)


def pack(source, output):
    files = expected_files(source)
    verify(source, files)
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=1) as archive:
        for name in sorted(files):
            archive.write(source/name, name)
    result = {"tag": "research-assets-2026-09-30", "asset": output.name, "sha256": digest(output), "files": len(files), "bytes": output.stat().st_size}
    print(json.dumps(result))
    return result


def extract(archive_path, output, files):
    with zipfile.ZipFile(archive_path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or set(names) != set(files):
            raise ValueError("research archive has missing, duplicate or unexpected entries")
        for info in archive.infolist():
            safe_name(info.filename)
            if info.flag_bits & 1 or (info.external_attr >> 16) & 0o170000 == 0o120000 or info.file_size > 1024**3:
                raise ValueError("unsafe research archive entry")
            destination = output/info.filename
            if not destination.resolve().is_relative_to(output.resolve()):
                raise ValueError("research extraction leaves staging directory")
            destination.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(info) as source, destination.open("xb") as target:
                shutil.copyfileobj(source, target)
    verify(output, files)


def runtime_python(root, arch=None):
    arch = arch or ("aarch64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64")
    return root/("python-"+arch)/("python.exe" if sys.platform == "win32" else "bin/python3.12")


def prepare(archive_path, output):
    lock = json.loads((ROOT/"research-assets.json").read_text(encoding="utf8"))
    if digest(archive_path) != lock["sha256"]:
        raise ValueError("research archive SHA256 does not match source lock")
    if output.exists():
        raise ValueError("research staging already exists; use a fresh target directory")
    files = expected_files(ROOT)
    output.mkdir(parents=True)
    extract(archive_path, output, files)
    os.environ.setdefault("UV_CACHE_DIR", str(ROOT/"src-tauri/target/uv-cache"))
    os.environ["UV_PYTHON_INSTALL_BIN"] = "0"
    os.environ["UV_PYTHON_INSTALL_REGISTRY"] = "0"
    build = ROOT/"src-tauri/target/research-python"
    arches = ("x86_64", "aarch64") if sys.platform == "darwin" else ("x86_64",)
    system = {"win32": "windows", "darwin": "macos", "linux": "linux"}[sys.platform]
    for arch in arches:
        key = "cpython-"+PYTHON+"-"+system+"-"+arch+("-gnu" if system == "linux" else "-none")
        subprocess.run(["uv", "python", "install", key, "--install-dir", str(build), "--no-bin", "--no-registry"], check=True)
        destination = output/("python-"+arch)
        shutil.copytree(build/key, destination, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        site = destination/("Lib/site-packages" if system == "windows" else "lib/python3.12/site-packages")
        triple = {"windows": "x86_64-pc-windows-msvc", "linux": "x86_64-unknown-linux-gnu", "macos": arch+"-apple-darwin"}[system]
        subprocess.run(["uv", "pip", "install", "--python", sys.executable, "--python-version", "3.12", "--python-platform", triple, "--target", str(site), "--only-binary", ":all:", "--require-hashes", "-r", str(ROOT/"scripts/research-requirements.txt")], check=True)
    (output/"runtime.json").write_text(json.dumps({"schema": "bull-research-runtime-v1", "python": PYTHON, "asset_sha256": lock["sha256"], "files": files}, indent=2), encoding="utf8")
    check = ROOT/"scripts/check-research-runtime.py"
    subprocess.run([str(runtime_python(output)), "-E", "-s", "-B", "-X", "utf8", str(check), str(output)], check=True)
    print("Prepared complete research runtime: " + str(output))


def self_check():
    for name in ("../secret", "research/../data/key", "research/a\\b", "research/C:/a", "/research/a", "research//a"):
        try:
            safe_name(name)
        except ValueError:
            pass
        else:
            raise AssertionError("unsafe name accepted")
    with tempfile.TemporaryDirectory() as folder:
        base = Path(folder)
        archive = base/"valid.zip"
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr("research/test.json", "valid")
        wanted = {"research/test.json": hashlib.sha256(b"valid").hexdigest()}
        extract(archive, base/"out", wanted)
        assert (base/"out/research/test.json").read_text() == "valid"
        for extra in ("research/private.db", "../secret", "research/test.json"):
            with zipfile.ZipFile(base/"bad.zip", "w") as output:
                output.writestr("research/test.json", "valid")
                output.writestr(extra, "private")
            try:
                extract(base/"bad.zip", base/"bad", wanted)
            except ValueError:
                pass
            else:
                raise AssertionError("unexpected/duplicate/traversal entry accepted")
        (base/"out/research/test.json").write_text("modified")
        try:
            verify(base/"out", wanted)
        except ValueError:
            pass
        else:
            raise AssertionError("modified data accepted")
    print("Research bundle self-check passed: finite manifest, hashes, no extra files/path traversal")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    pack_parser = sub.add_parser("pack")
    pack_parser.add_argument("source", type=Path)
    pack_parser.add_argument("output", type=Path)
    prepare_parser = sub.add_parser("prepare")
    prepare_parser.add_argument("archive", type=Path)
    prepare_parser.add_argument("output", type=Path)
    sub.add_parser("self-check")
    args = parser.parse_args()
    if args.command == "pack":
        pack(args.source.resolve(), args.output.resolve())
    elif args.command == "prepare":
        prepare(args.archive.resolve(), args.output.resolve())
    else:
        self_check()
