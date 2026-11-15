import tempfile
import unittest
from pathlib import Path

from parse_python import analyze_project


class TestParsePython(unittest.TestCase):
    def test_analyze_project_hierarchy_and_edges(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "sample_proj"
            pkg = root / "pkg"
            pkg.mkdir(parents=True)
            (pkg / "__init__.py").write_text('"""Sample package."""\n', encoding="utf-8")
            (pkg / "math_ops.py").write_text(
                '''"""Math helpers."""

def add(a: int, b: int = 0) -> int:
    """Add two numbers."""
    return a + b


class Calculator:
    """Simple calculator."""

    def double_add(self, x: int, y: int) -> int:
        return self._scale(add(x, y))

    def _scale(self, val: int) -> int:
        return val * 2
''',
                encoding="utf-8",
            )
            (pkg / "runner.py").write_text(
                '''from .math_ops import Calculator, add

async def run(x: int) -> int:
    return add(x, 1)
''',
                encoding="utf-8",
            )
            (root / "broken.py").write_text("def oops(:\n    pass\n", encoding="utf-8")

            graph = analyze_project(str(root))

            self.assertEqual(graph["rootName"], "sample_proj")
            self.assertEqual(graph["stats"]["totalFiles"], 3)
            self.assertEqual(graph["stats"]["totalClasses"], 1)
            self.assertEqual(graph["stats"]["totalFunctions"], 4)
            self.assertEqual(len(graph["parseErrors"]), 1)
            self.assertEqual(graph["parseErrors"][0]["filePath"], "broken.py")

            group_ids = {g["id"] for g in graph["groups"]}
            self.assertIn("dir:.", group_ids)
            self.assertIn("dir:pkg", group_ids)
            self.assertIn("file:pkg/math_ops.py", group_ids)
            self.assertIn("class:pkg/math_ops.py::Calculator", group_ids)

            fn_by_name = {f["name"]: f for f in graph["functions"]}
            self.assertTrue(fn_by_name["run"]["isAsync"])
            self.assertTrue(fn_by_name["double_add"]["isMethod"])
            self.assertIn("def double_add(self, x: int, y: int) -> int:", fn_by_name["double_add"]["code"])

            edges = {(e["kind"], e["sourceId"], e["targetId"]) for e in graph["edges"]}
            self.assertIn(("import", "file:pkg/runner.py", "file:pkg/math_ops.py"), edges)
            self.assertIn(
                ("call", "fn:pkg/runner.py::run", "fn:pkg/math_ops.py::add"),
                edges,
            )
            self.assertIn(
                (
                    "call",
                    "fn:pkg/math_ops.py::Calculator.double_add",
                    "fn:pkg/math_ops.py::Calculator._scale",
                ),
                edges,
            )
            self.assertIn(
                (
                    "call",
                    "fn:pkg/math_ops.py::Calculator.double_add",
                    "fn:pkg/math_ops.py::add",
                ),
                edges,
            )


if __name__ == "__main__":
    unittest.main()
