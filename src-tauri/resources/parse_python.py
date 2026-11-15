#!/usr/bin/env python3
"""Parse a Python project directory into a hierarchical diagram JSON graph."""

from __future__ import annotations

import ast
import json
import os
import sys
import textwrap
from pathlib import Path
from typing import Any

SKIP_DIRS = {
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    ".env",
    "node_modules",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    "dist",
    "build",
    ".eggs",
    "target",
}


def format_signature(node: ast.FunctionDef | ast.AsyncFunctionDef) -> str:
    """Build a compact human-readable signature string for a function."""
    args = node.args
    parts: list[str] = []

    pos_only = args.posonlyargs
    regular = args.args
    all_pos = pos_only + regular
    defaults_offset = len(all_pos) - len(args.defaults)

    for idx, arg in enumerate(all_pos):
        piece = arg.arg
        if arg.annotation:
            piece += f": {ast.unparse(arg.annotation)}"
        if idx >= defaults_offset:
            default_node = args.defaults[idx - defaults_offset]
            piece += f" = {ast.unparse(default_node)}"
        parts.append(piece)
        if pos_only and idx == len(pos_only) - 1:
            parts.append("/")

    if args.vararg:
        v = f"*{args.vararg.arg}"
        if args.vararg.annotation:
            v += f": {ast.unparse(args.vararg.annotation)}"
        parts.append(v)
    elif args.kwonlyargs:
        parts.append("*")

    for kw_arg, kw_default in zip(args.kwonlyargs, args.kw_defaults):
        piece = kw_arg.arg
        if kw_arg.annotation:
            piece += f": {ast.unparse(kw_arg.annotation)}"
        if kw_default is not None:
            piece += f" = {ast.unparse(kw_default)}"
        parts.append(piece)

    if args.kwarg:
        k = f"**{args.kwarg.arg}"
        if args.kwarg.annotation:
            k += f": {ast.unparse(args.kwarg.annotation)}"
        parts.append(k)

    ret = f" -> {ast.unparse(node.returns)}" if node.returns else ""
    return f"({', '.join(parts)}){ret}"


def extract_source_slice(lines: list[str], start_line: int, end_line: int) -> str:
    """Extract 1-indexed inclusive line range and dedent for clean display."""
    raw = "".join(lines[start_line - 1 : end_line])
    return textwrap.dedent(raw).rstrip("\n")


def first_docstring_line(node: ast.AST) -> str | None:
    doc = ast.get_docstring(node)  # type: ignore[arg-type]
    if not doc:
        return None
    first = doc.strip().splitlines()[0].strip()
    return first[:120] if first else None


def resolve_relative_module(current_parts: list[str], is_init: bool, level: int, module: str | None) -> str | None:
    """Resolve a relative import to a dotted module name."""
    package_parts = current_parts if is_init else current_parts[:-1]
    if level > len(package_parts) + 1:
        return None
    up = level - 1
    base = package_parts[: len(package_parts) - up] if up > 0 else list(package_parts)
    if module:
        return ".".join(base + module.split("."))
    return ".".join(base) if base else None


def analyze_project(root_path_str: str) -> dict[str, Any]:
    root_path = Path(root_path_str).resolve()
    if not root_path.is_dir():
        raise ValueError(f"Not a directory: {root_path_str}")

    py_files: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root_path):
        dirnames[:] = sorted(
            d for d in dirnames if d not in SKIP_DIRS and not d.endswith(".egg-info")
        )
        for fname in sorted(filenames):
            if fname.endswith(".py"):
                py_files.append(Path(dirpath) / fname)

    groups: list[dict[str, Any]] = []
    functions: list[dict[str, Any]] = []
    parse_errors: list[dict[str, str]] = []

    # Track directories that actually contain .py files
    dir_ids_added: set[str] = set()

    def ensure_dir_chain(rel_dir: Path) -> str:
        rel_str = "." if str(rel_dir) in ("", ".") else rel_dir.as_posix()
        dir_id = f"dir:{rel_str}"
        if dir_id in dir_ids_added:
            return dir_id

        if rel_str == ".":
            parent_id = None
            name = root_path.name or str(root_path)
            qualified = name
        else:
            parent_id = ensure_dir_chain(rel_dir.parent)
            name = rel_dir.name
            qualified = rel_str.replace("/", ".")

        is_package = (root_path / rel_dir / "__init__.py").is_file()
        dir_ids_added.add(dir_id)
        groups.append(
            {
                "id": dir_id,
                "kind": "directory",
                "name": name,
                "qualifiedName": qualified,
                "parentId": parent_id,
                "filePath": None if rel_str == "." else rel_str,
                "isPackage": is_package,
                "docstring": None,
                "lineCount": 0,
            }
        )
        return dir_id

    # Always ensure root directory exists
    ensure_dir_chain(Path("."))

    # Lookup tables for resolving imports and calls
    module_by_dotted: dict[str, str] = {}
    parsed_modules: list[dict[str, Any]] = []
    fn_by_module_name: dict[tuple[str, str], str] = {}
    fn_by_class_method: dict[tuple[str, str], str] = {}
    fn_by_global_name: dict[str, list[str]] = {}

    for file_path in py_files:
        rel_path = file_path.relative_to(root_path)
        rel_posix = rel_path.as_posix()
        parent_dir_id = ensure_dir_chain(rel_path.parent)

        try:
            source_text = file_path.read_text(encoding="utf-8", errors="replace")
            tree = ast.parse(source_text, filename=rel_posix)
        except SyntaxError as exc:
            parse_errors.append(
                {
                    "filePath": rel_posix,
                    "error": f"Line {exc.lineno}: {exc.msg}",
                }
            )
            continue
        except OSError as exc:
            parse_errors.append({"filePath": rel_posix, "error": str(exc)})
            continue

        lines = source_text.splitlines(keepends=True)
        parts = list(rel_path.with_suffix("").parts)
        is_init = rel_path.name == "__init__.py"
        if is_init and len(parts) > 1:
            dotted_name = ".".join(parts[:-1])
        elif is_init:
            dotted_name = root_path.name
        else:
            dotted_name = ".".join(parts)

        module_id = f"file:{rel_posix}"
        module_by_dotted[dotted_name] = module_id
        # Also map full dotted with __init__ stripped or kept
        module_by_dotted[".".join(parts)] = module_id

        groups.append(
            {
                "id": module_id,
                "kind": "module",
                "name": rel_path.name,
                "qualifiedName": dotted_name,
                "parentId": parent_dir_id,
                "filePath": rel_posix,
                "isPackage": is_init,
                "docstring": first_docstring_line(tree),
                "lineCount": len(lines),
            }
        )

        # Collect imports in this module for later edge & call resolution
        raw_imports: list[tuple[str, str | None]] = []  # (target_dotted, local_alias)
        symbol_imports: dict[str, tuple[str, str]] = {}  # local_name -> (target_dotted, symbol_name)
        module_aliases: dict[str, str] = {}  # local_alias -> target_dotted

        for stmt in tree.body:
            if isinstance(stmt, ast.Import):
                for alias in stmt.names:
                    local = alias.asname or alias.name.split(".")[0]
                    module_aliases[local] = alias.name
                    raw_imports.append((alias.name, alias.asname))
            elif isinstance(stmt, ast.ImportFrom):
                if stmt.level and stmt.level > 0:
                    target_mod = resolve_relative_module(parts, is_init, stmt.level, stmt.module)
                else:
                    target_mod = stmt.module
                if target_mod:
                    raw_imports.append((target_mod, None))
                    for alias in stmt.names:
                        if alias.name == "*":
                            continue
                        local = alias.asname or alias.name
                        symbol_imports[local] = (target_mod, alias.name)
                        # Could also be importing a submodule: from pkg import submod
                        sub_dotted = f"{target_mod}.{alias.name}"
                        module_aliases[local] = sub_dotted
                        raw_imports.append((sub_dotted, None))

        # Extract top-level functions and classes
        fn_asts: list[tuple[str, str | None, ast.FunctionDef | ast.AsyncFunctionDef]] = []

        def add_function(
            fn_node: ast.FunctionDef | ast.AsyncFunctionDef,
            parent_id: str,
            class_id: str | None,
            prefix: str,
        ) -> None:
            start_line = fn_node.lineno
            if fn_node.decorator_list:
                start_line = min(d.lineno for d in fn_node.decorator_list)
            end_line = fn_node.end_lineno or fn_node.lineno
            fn_id = f"fn:{rel_posix}::{prefix}{fn_node.name}"
            code = extract_source_slice(lines, start_line, end_line)
            decorators = [f"@{ast.unparse(d)}" for d in fn_node.decorator_list]

            functions.append(
                {
                    "id": fn_id,
                    "name": fn_node.name,
                    "qualifiedName": f"{dotted_name}.{prefix}{fn_node.name}",
                    "parentId": parent_id,
                    "filePath": rel_posix,
                    "isAsync": isinstance(fn_node, ast.AsyncFunctionDef),
                    "isMethod": class_id is not None,
                    "decorators": decorators,
                    "signature": format_signature(fn_node),
                    "docstring": first_docstring_line(fn_node),
                    "startLine": start_line,
                    "endLine": end_line,
                    "code": code,
                }
            )

            if class_id is not None:
                fn_by_class_method[(class_id, fn_node.name)] = fn_id
            else:
                fn_by_module_name[(module_id, fn_node.name)] = fn_id

            fn_by_global_name.setdefault(fn_node.name, []).append(fn_id)
            fn_asts.append((fn_id, class_id, fn_node))

        for stmt in tree.body:
            if isinstance(stmt, (ast.FunctionDef, ast.AsyncFunctionDef)):
                add_function(stmt, module_id, None, "")
            elif isinstance(stmt, ast.ClassDef):
                class_id = f"class:{rel_posix}::{stmt.name}"
                c_start = stmt.lineno
                c_end = stmt.end_lineno or stmt.lineno
                groups.append(
                    {
                        "id": class_id,
                        "kind": "class",
                        "name": stmt.name,
                        "qualifiedName": f"{dotted_name}.{stmt.name}",
                        "parentId": module_id,
                        "filePath": rel_posix,
                        "isPackage": False,
                        "docstring": first_docstring_line(stmt),
                        "lineCount": max(1, c_end - c_start + 1),
                    }
                )
                for member in stmt.body:
                    if isinstance(member, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        add_function(member, class_id, class_id, f"{stmt.name}.")

        parsed_modules.append(
            {
                "module_id": module_id,
                "raw_imports": raw_imports,
                "symbol_imports": symbol_imports,
                "module_aliases": module_aliases,
                "fn_asts": fn_asts,
            }
        )

    # Build import & call edges
    edges: list[dict[str, Any]] = []
    seen_import_edges: set[tuple[str, str]] = set()
    seen_call_edges: set[tuple[str, str]] = set()

    for mod in parsed_modules:
        src_mod_id = mod["module_id"]

        for target_dotted, _ in mod["raw_imports"]:
            target_mod_id = module_by_dotted.get(target_dotted)
            if target_mod_id and target_mod_id != src_mod_id:
                pair = (src_mod_id, target_mod_id)
                if pair not in seen_import_edges:
                    seen_import_edges.add(pair)
                    edges.append(
                        {
                            "id": f"imp:{src_mod_id}->{target_mod_id}",
                            "kind": "import",
                            "sourceId": src_mod_id,
                            "targetId": target_mod_id,
                            "label": "imports",
                        }
                    )

        symbol_imports = mod["symbol_imports"]
        module_aliases = mod["module_aliases"]

        for fn_id, class_id, fn_node in mod["fn_asts"]:
            for sub in ast.walk(fn_node):
                if not isinstance(sub, ast.Call):
                    continue
                target_fn_id: str | None = None
                call_label: str | None = None

                if isinstance(sub.func, ast.Name):
                    callee = sub.func.id
                    call_label = callee
                    if callee in symbol_imports:
                        imp_mod_dotted, imp_sym = symbol_imports[callee]
                        imp_mod_id = module_by_dotted.get(imp_mod_dotted)
                        if imp_mod_id:
                            target_fn_id = fn_by_module_name.get((imp_mod_id, imp_sym))
                    if not target_fn_id:
                        target_fn_id = fn_by_module_name.get((src_mod_id, callee))
                    if not target_fn_id:
                        candidates = fn_by_global_name.get(callee, [])
                        if len(candidates) == 1:
                            target_fn_id = candidates[0]

                elif isinstance(sub.func, ast.Attribute):
                    attr = sub.func.attr
                    call_label = attr
                    if isinstance(sub.func.value, ast.Name):
                        recv = sub.func.value.id
                        if recv in ("self", "cls") and class_id:
                            target_fn_id = fn_by_class_method.get((class_id, attr))
                        elif recv in module_aliases:
                            mod_dotted = module_aliases[recv]
                            imp_mod_id = module_by_dotted.get(mod_dotted)
                            if imp_mod_id:
                                target_fn_id = fn_by_module_name.get((imp_mod_id, attr))

                if target_fn_id and target_fn_id != fn_id:
                    pair = (fn_id, target_fn_id)
                    if pair not in seen_call_edges:
                        seen_call_edges.add(pair)
                        edges.append(
                            {
                                "id": f"call:{fn_id}->{target_fn_id}",
                                "kind": "call",
                                "sourceId": fn_id,
                                "targetId": target_fn_id,
                                "label": call_label,
                            }
                        )

    total_files = sum(1 for g in groups if g["kind"] == "module")
    total_classes = sum(1 for g in groups if g["kind"] == "class")

    return {
        "rootName": root_path.name or str(root_path),
        "rootPath": str(root_path),
        "groups": groups,
        "functions": functions,
        "edges": edges,
        "parseErrors": parse_errors,
        "stats": {
            "totalFiles": total_files,
            "totalClasses": total_classes,
            "totalFunctions": len(functions),
            "totalImportEdges": len(seen_import_edges),
            "totalCallEdges": len(seen_call_edges),
        },
    }


def main() -> None:
    if len(sys.argv) < 2:
        print(json.dumps({"error": "Missing project path argument"}))
        sys.exit(1)
    result = analyze_project(sys.argv[1])
    print(json.dumps(result))


if __name__ == "__main__":
    main()
