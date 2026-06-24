#!/usr/bin/env python3
"""Build the original SlowRenju core with a Linux Gomocup compatibility entry."""

from __future__ import annotations

import argparse
import shutil
import subprocess
from pathlib import Path


CORE_SOURCES = [
    "AI/AIs.cpp",
    "AI/AIx.cpp",
    "AI/Hash.cpp",
    "Common/global_value.cpp",
    "Shape/ShapeList.cpp",
    "Shape/line.cpp",
    "Shape/line4v.cpp",
    "VCF/VCF.cpp",
    "Value/ValueB.cpp",
    "Value/ValueW.cpp",
    "Value/ValueWide.cpp",
]


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def normalize_includes(path: Path) -> None:
    text = path.read_text(encoding="utf-8", errors="surrogateescape")
    normalized = text.replace(r"..\Headers\game.h", "../Headers/game.h")
    normalized = normalized.replace(
        "allocator<pair<wstring,int>>",
        "allocator<pair<const wstring,int>>",
    )
    path.write_text(normalized, encoding="utf-8", errors="surrogateescape")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source",
        type=Path,
        default=Path.home() / "downloads" / "oracle_ws" / "SlowRenju",
    )
    parser.add_argument(
        "--build-dir",
        type=Path,
        default=repo_root() / "target" / "slowrenju-linux",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=repo_root() / "target" / "release" / "slowrenju_linux",
    )
    args = parser.parse_args()

    source = args.source.expanduser().resolve()
    build_dir = args.build_dir.expanduser().resolve()
    output = args.output.expanduser().resolve()
    if not (source / "AI" / "AIx.cpp").is_file():
        raise SystemExit(f"SlowRenju source not found: {source}")

    if build_dir.exists():
        shutil.rmtree(build_dir)
    build_dir.mkdir(parents=True)

    for relative in CORE_SOURCES:
        destination = build_dir / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / relative, destination)
        normalize_includes(destination)
    shutil.copytree(source / "Headers", build_dir / "Headers")

    tools = repo_root() / "tools" / "slowrenju_linux"
    output.parent.mkdir(parents=True, exist_ok=True)
    command = [
        "g++",
        "-std=c++17",
        "-O3",
        "-DNDEBUG",
        "-include",
        str(tools / "compat.hpp"),
        "-I",
        str(tools),
        "-I",
        str(build_dir / "Headers"),
        str(tools / "main.cpp"),
        *(str(build_dir / relative) for relative in CORE_SOURCES),
        "-o",
        str(output),
    ]
    subprocess.run(command, check=True)
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
