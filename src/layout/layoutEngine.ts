import type {
  FunctionNode,
  GroupKind,
  GroupNode,
  LayoutResult,
  PositionedEdge,
  PositionedFunction,
  PositionedGroup,
  ProjectGraph,
  Rect,
} from "../types/graph";

interface LayoutBox {
  id: string;
  kind: GroupKind | "function";
  width: number;
  height: number;
  relX: number;
  relY: number;
  children: LayoutBox[];
  functionCount: number;
}

const GROUP_STYLE: Record<
  GroupKind,
  { padX: number; padTop: number; padBottom: number; gap: number; minW: number; minH: number }
> = {
  directory: { padX: 36, padTop: 72, padBottom: 36, gap: 40, minW: 360, minH: 180 },
  module: { padX: 28, padTop: 58, padBottom: 28, gap: 28, minW: 320, minH: 130 },
  class: { padX: 22, padTop: 50, padBottom: 22, gap: 22, minW: 280, minH: 110 },
};

function measureFunctionBox(fn: FunctionNode): { width: number; height: number } {
  const lines = fn.code.split("\n");
  const maxLineLen = lines.reduce((acc, line) => Math.max(acc, line.length), fn.name.length + fn.signature.length);
  const width = Math.max(340, Math.min(580, 40 + maxLineLen * 7.4));
  const codeLines = Math.min(lines.length, 28);
  const height = Math.max(130, 56 + codeLines * 17.5 + 18);
  return { width: Math.round(width), height: Math.round(height) };
}

function packBoxes(
  items: LayoutBox[],
  gap: number,
  padX: number,
  padTop: number,
  padBottom: number,
  minW: number,
  minH: number
): { width: number; height: number } {
  if (items.length === 0) {
    return { width: minW, height: minH };
  }

  const totalArea = items.reduce((sum, item) => sum + (item.width + gap) * (item.height + gap), 0);
  const maxItemW = items.reduce((m, item) => Math.max(m, item.width), 0);
  const targetRowWidth = Math.max(maxItemW, Math.sqrt(totalArea) * 1.35);

  let cursorX = padX;
  let cursorY = padTop;
  let rowHeight = 0;
  let maxUsedWidth = 0;

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (cursorX > padX && cursorX - padX + item.width > targetRowWidth) {
      cursorX = padX;
      cursorY += rowHeight + gap;
      rowHeight = 0;
    }

    item.relX = cursorX;
    item.relY = cursorY;

    cursorX += item.width + gap;
    rowHeight = Math.max(rowHeight, item.height);
    maxUsedWidth = Math.max(maxUsedWidth, cursorX - gap + padX);
  }

  const totalHeight = cursorY + rowHeight + padBottom;
  return {
    width: Math.max(minW, Math.round(maxUsedWidth)),
    height: Math.max(minH, Math.round(totalHeight)),
  };
}

function anchorPoints(a: Rect, b: Rect): { x1: number; y1: number; x2: number; y2: number } {
  const ax = a.x + a.width / 2;
  const ay = a.y + a.height / 2;
  const bx = b.x + b.width / 2;
  const by = b.y + b.height / 2;
  const dx = bx - ax;
  const dy = by - ay;

  if (Math.abs(dx) >= Math.abs(dy)) {
    return dx >= 0
      ? { x1: a.x + a.width, y1: ay, x2: b.x, y2: by }
      : { x1: a.x, y1: ay, x2: b.x + b.width, y2: by };
  }
  return dy >= 0
    ? { x1: ax, y1: a.y + a.height, x2: bx, y2: b.y }
    : { x1: ax, y1: a.y, x2: bx, y2: b.y + b.height };
}

export function computeLayout(graph: ProjectGraph): LayoutResult {
  const groupById = new Map<string, GroupNode>();
  const childGroupIds = new Map<string, string[]>();
  const childFnMap = new Map<string, FunctionNode[]>();
  const rootGroupIds: string[] = [];

  for (const g of graph.groups) {
    groupById.set(g.id, g);
    if (g.parentId) {
      const list = childGroupIds.get(g.parentId) ?? [];
      list.push(g.id);
      childGroupIds.set(g.parentId, list);
    } else {
      rootGroupIds.push(g.id);
    }
  }

  for (const fn of graph.functions) {
    const list = childFnMap.get(fn.parentId) ?? [];
    list.push(fn);
    childFnMap.set(fn.parentId, list);
  }

  const fnById = new Map<string, FunctionNode>(graph.functions.map((f) => [f.id, f]));

  function buildBox(groupId: string): LayoutBox {
    const group = groupById.get(groupId)!;
    const subGroupIds = childGroupIds.get(groupId) ?? [];
    const subFns = childFnMap.get(groupId) ?? [];

    const childBoxes: LayoutBox[] = [];
    for (const subId of subGroupIds) {
      childBoxes.push(buildBox(subId));
    }
    for (const fn of subFns) {
      const { width, height } = measureFunctionBox(fn);
      childBoxes.push({
        id: fn.id,
        kind: "function",
        width,
        height,
        relX: 0,
        relY: 0,
        children: [],
        functionCount: 1,
      });
    }

    const style = GROUP_STYLE[group.kind];
    const { width, height } = packBoxes(
      childBoxes,
      style.gap,
      style.padX,
      style.padTop,
      style.padBottom,
      style.minW,
      style.minH
    );

    const functionCount = childBoxes.reduce((sum, c) => sum + c.functionCount, 0);

    return {
      id: group.id,
      kind: group.kind,
      width,
      height,
      relX: 0,
      relY: 0,
      children: childBoxes,
      functionCount,
    };
  }

  const rootBoxes = rootGroupIds.map(buildBox);
  const rootPacked = packBoxes(rootBoxes, 48, 40, 40, 40, 400, 300);

  const positionedGroups: PositionedGroup[] = [];
  const positionedFunctions: PositionedFunction[] = [];
  const nodeRects: Record<string, Rect> = {};

  function flatten(box: LayoutBox, offsetX: number, offsetY: number, depth: number): void {
    const absX = offsetX + box.relX;
    const absY = offsetY + box.relY;
    const rect: Rect = { x: absX, y: absY, width: box.width, height: box.height };
    nodeRects[box.id] = rect;

    if (box.kind === "function") {
      const fn = fnById.get(box.id)!;
      positionedFunctions.push({ ...fn, ...rect });
      return;
    }

    const group = groupById.get(box.id)!;
    positionedGroups.push({
      ...group,
      ...rect,
      depth,
      functionCount: box.functionCount,
    });

    for (const child of box.children) {
      flatten(child, absX, absY, depth + 1);
    }
  }

  for (const rb of rootBoxes) {
    flatten(rb, 0, 0, 0);
  }

  const positionedEdges: PositionedEdge[] = [];
  for (const edge of graph.edges) {
    const srcRect = nodeRects[edge.sourceId];
    const dstRect = nodeRects[edge.targetId];
    if (!srcRect || !dstRect) continue;
    const pts = anchorPoints(srcRect, dstRect);
    positionedEdges.push({ ...edge, ...pts });
  }

  return {
    groups: positionedGroups,
    functions: positionedFunctions,
    edges: positionedEdges,
    nodeRects,
    bounds: { width: rootPacked.width, height: rootPacked.height },
  };
}
