#!/usr/bin/env python3
"""Extract bundled static data from vendored reference Python sources.

This script keeps the Rust crate independent from parsing Python source files
at runtime. It reads the vendored reference text under `data/reference_text/`
and writes plain-text numeric data files under `data/static/`.
"""

from __future__ import annotations

import argparse
import ast
from pathlib import Path


def load_assignment(path: Path, name: str):
    module = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    for node in module.body:
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and target.id == name:
                    return ast.literal_eval(node.value)
        if isinstance(node, ast.AnnAssign):
            target = node.target
            if isinstance(target, ast.Name) and target.id == name and node.value is not None:
                return ast.literal_eval(node.value)
    raise ValueError(f"{name} not found in {path}")


def write_default_eval_para(path: Path, values: tuple[float, ...]) -> None:
    lines = [repr(value) for value in values]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_shape_table(path: Path, rows: tuple[tuple[int, ...], ...]) -> None:
    lines = [" ".join(str(value) for value in row) for row in rows]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def read_default_eval_para(path: Path) -> tuple[float, ...]:
    return tuple(
        float(line.strip())
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    )


def read_shape_table(path: Path) -> tuple[tuple[int, ...], ...]:
    return tuple(
        tuple(int(token) for token in line.split())
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate generated files against the vendored reference source",
    )
    args = parser.parse_args()

    root = args.root.resolve()
    source_dir = root / "data" / "reference_text"
    output_dir = root / "data" / "static"
    output_dir.mkdir(parents=True, exist_ok=True)

    config_path = source_dir / "config.py"
    shape_table_path = source_dir / "shape_table.py"

    default_eval_para = load_assignment(config_path, "DEFAULT_EVAL_PARA")
    shape_table = load_assignment(shape_table_path, "SHAPE_TABLE")

    if not isinstance(default_eval_para, tuple):
        raise TypeError("DEFAULT_EVAL_PARA must be a tuple")
    if not isinstance(shape_table, tuple):
        raise TypeError("SHAPE_TABLE must be a tuple of rows")

    default_eval_para_out = output_dir / "default_eval_para.txt"
    shape_table_out = output_dir / "shape_table.txt"

    if args.check:
        generated_para = read_default_eval_para(default_eval_para_out)
        generated_shape_table = read_shape_table(shape_table_out)
        if generated_para != default_eval_para:
            raise SystemExit("default_eval_para.txt does not match source")
        if generated_shape_table != shape_table:
            raise SystemExit("shape_table.txt does not match source")
        print("OK")
        return

    write_default_eval_para(default_eval_para_out, default_eval_para)
    write_shape_table(shape_table_out, shape_table)


if __name__ == "__main__":
    main()
