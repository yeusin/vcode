import { describe, expect, it } from "vitest";
import { computeLayout } from "./layoutEngine";
import type { ProjectGraph, Rect } from "../types/graph";

function overlaps(a: Rect, b: Rect): boolean {
  return !(
    a.x + a.width <= b.x ||
    b.x + b.width <= a.x ||
    a.y + a.height <= b.y ||
    b.y + b.height <= a.y
  );
}

function contains(parent: Rect, child: Rect): boolean {
  return (
    child.x >= parent.x &&
    child.y >= parent.y &&
    child.x + child.width <= parent.x + parent.width &&
    child.y + child.height <= parent.y + parent.height
  );
}

describe("computeLayout", () => {
  it("encloses children within parents without sibling overlap", () => {
    const graph: ProjectGraph = {
      rootName: "demo",
      rootPath: "/tmp/demo",
      groups: [
        {
          id: "dir:.",
          kind: "directory",
          name: "demo",
          qualifiedName: "demo",
          parentId: null,
          filePath: null,
          isPackage: false,
          docstring: null,
          lineCount: 0,
        },
        {
          id: "file:a.py",
          kind: "module",
          name: "a.py",
          qualifiedName: "a",
          parentId: "dir:.",
          filePath: "a.py",
          isPackage: false,
          docstring: null,
          lineCount: 20,
        },
        {
          id: "file:b.py",
          kind: "module",
          name: "b.py",
          qualifiedName: "b",
          parentId: "dir:.",
          filePath: "b.py",
          isPackage: false,
          docstring: null,
          lineCount: 5,
        },
        {
          id: "class:a.py::Worker",
          kind: "class",
          name: "Worker",
          qualifiedName: "a.Worker",
          parentId: "file:a.py",
          filePath: "a.py",
          isPackage: false,
          docstring: null,
          lineCount: 10,
        },
      ],
      functions: [
        {
          id: "fn:a.py::Worker.run",
          name: "run",
          qualifiedName: "a.Worker.run",
          parentId: "class:a.py::Worker",
          filePath: "a.py",
          isAsync: false,
          isMethod: true,
          decorators: [],
          signature: "(self) -> None",
          docstring: null,
          startLine: 1,
          endLine: 3,
          code: "def run(self) -> None:\n    return helper()",
        },
        {
          id: "fn:a.py::helper",
          name: "helper",
          qualifiedName: "a.helper",
          parentId: "file:a.py",
          filePath: "a.py",
          isAsync: false,
          isMethod: false,
          decorators: [],
          signature: "() -> None",
          docstring: null,
          startLine: 5,
          endLine: 6,
          code: "def helper() -> None:\n    pass",
        },
      ],
      edges: [
        {
          id: "call:1",
          kind: "call",
          sourceId: "fn:a.py::Worker.run",
          targetId: "fn:a.py::helper",
          label: "helper",
        },
        {
          id: "imp:1",
          kind: "import",
          sourceId: "file:a.py",
          targetId: "file:b.py",
          label: "imports",
        },
      ],
      parseErrors: [],
      stats: {
        totalFiles: 2,
        totalClasses: 1,
        totalFunctions: 2,
        totalImportEdges: 1,
        totalCallEdges: 1,
      },
    };

    const layout = computeLayout(graph);
    const root = layout.nodeRects["dir:."];
    const modA = layout.nodeRects["file:a.py"];
    const modB = layout.nodeRects["file:b.py"];
    const cls = layout.nodeRects["class:a.py::Worker"];
    const fnRun = layout.nodeRects["fn:a.py::Worker.run"];
    const fnHelper = layout.nodeRects["fn:a.py::helper"];

    expect(contains(root, modA)).toBe(true);
    expect(contains(root, modB)).toBe(true);
    expect(overlaps(modA, modB)).toBe(false);

    expect(contains(modA, cls)).toBe(true);
    expect(contains(modA, fnHelper)).toBe(true);
    expect(overlaps(cls, fnHelper)).toBe(false);
    expect(contains(cls, fnRun)).toBe(true);

    expect(layout.edges).toHaveLength(2);
    expect(modB.width).toBeGreaterThan(0);
    expect(modB.height).toBeGreaterThan(0);
  });
});
