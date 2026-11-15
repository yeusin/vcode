import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  LayoutResult,
  PositionedEdge,
  PositionedFunction,
  PositionedGroup,
  Rect,
} from "../types/graph";
import { highlightPython, TOKEN_COLORS, tokenizePythonLine, type CodeToken } from "../utils/pythonHighlight";

export interface Viewport {
  tx: number;
  ty: number;
  scale: number;
}

export type LodLevel = "macro" | "meso" | "micro";

export function getLodLevel(scale: number): LodLevel {
  if (scale < 0.28) return "macro";
  if (scale < 0.72) return "meso";
  return "micro";
}

interface CanvasProps {
  layout: LayoutResult;
  viewport: Viewport;
  onViewportCommit: (vp: Viewport) => void;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  showImportEdges: boolean;
  showCallEdges: boolean;
  onFocusRect: (rect: Rect, minScale?: number) => void;
}

function rectsIntersect(a: Rect, b: Rect): boolean {
  return !(
    a.x + a.width < b.x ||
    b.x + b.width < a.x ||
    a.y + a.height < b.y ||
    b.y + b.height < a.y
  );
}

function pointInRect(wx: number, wy: number, r: Rect): boolean {
  return wx >= r.x && wx <= r.x + r.width && wy >= r.y && wy <= r.y + r.height;
}

function roundRectPath(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number
): void {
  const radius = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.arcTo(x + w, y, x + w, y + h, radius);
  ctx.arcTo(x + w, y + h, x, y + h, radius);
  ctx.arcTo(x, y + h, x, y, radius);
  ctx.arcTo(x, y, x + w, y, radius);
  ctx.closePath();
}

function drawEdge(
  ctx: CanvasRenderingContext2D,
  e: PositionedEdge,
  scale: number,
  isHighlighted: boolean,
  dimmed: boolean
): void {
  const dx = e.x2 - e.x1;
  const dy = e.y2 - e.y1;
  const horizontal = Math.abs(dx) >= Math.abs(dy);
  const c = Math.max(40, (horizontal ? Math.abs(dx) : Math.abs(dy)) * 0.45);

  const cp1x = horizontal ? e.x1 + (dx >= 0 ? c : -c) : e.x1;
  const cp1y = horizontal ? e.y1 : e.y1 + (dy >= 0 ? c : -c);
  const cp2x = horizontal ? e.x2 - (dx >= 0 ? c : -c) : e.x2;
  const cp2y = horizontal ? e.y2 : e.y2 - (dy >= 0 ? c : -c);

  const color = e.kind === "import" ? "#38bdf8" : "#f59e0b";
  ctx.save();
  ctx.globalAlpha = dimmed ? 0.1 : isHighlighted ? 1 : 0.48;
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = (isHighlighted ? 2.8 : 1.7) / Math.max(0.25, Math.min(1, scale));
  if (e.kind === "import") {
    ctx.setLineDash([7, 5]);
  }

  ctx.beginPath();
  ctx.moveTo(e.x1, e.y1);
  ctx.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, e.x2, e.y2);
  ctx.stroke();

  // Arrowhead
  const angle = Math.atan2(e.y2 - cp2y, e.x2 - cp2x);
  const headLen = 9 / Math.max(0.35, Math.min(1, scale));
  ctx.setLineDash([]);
  ctx.beginPath();
  ctx.moveTo(e.x2, e.y2);
  ctx.lineTo(
    e.x2 - headLen * Math.cos(angle - Math.PI / 6),
    e.y2 - headLen * Math.sin(angle - Math.PI / 6)
  );
  ctx.lineTo(
    e.x2 - headLen * Math.cos(angle + Math.PI / 6),
    e.y2 - headLen * Math.sin(angle + Math.PI / 6)
  );
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

const FONT_SANS = '"Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
const FONT_MONO = '"JetBrains Mono", "Fira Code", ui-monospace, SFMono-Regular, Menlo, monospace';

export const Canvas: React.FC<CanvasProps> = ({
  layout,
  viewport,
  onViewportCommit,
  selectedId,
  onSelect,
  showImportEdges,
  showCallEdges,
  onFocusRect,
}) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sizeRef = useRef<{ width: number; height: number; dpr: number }>({
    width: 1000,
    height: 700,
    dpr: window.devicePixelRatio || 1,
  });

  const vpRef = useRef<Viewport>(viewport);
  const lastCommittedRef = useRef<Viewport>(viewport);
  const rafRef = useRef<number | null>(null);
  const commitTimerRef = useRef<number | null>(null);
  const hoveredIdRef = useRef<string | null>(null);
  const selectedIdRef = useRef<string | null>(selectedId);
  const showImportRef = useRef(showImportEdges);
  const showCallRef = useRef(showCallEdges);

  const [renderVp, setRenderVp] = useState<Viewport>(viewport);
  const [isDragging, setIsDragging] = useState(false);

  selectedIdRef.current = selectedId;
  showImportRef.current = showImportEdges;
  showCallRef.current = showCallEdges;

  // Pre-tokenize all function code once per layout for instant 2D canvas rendering
  const tokenizedByFn = useMemo(() => {
    const map = new Map<string, CodeToken[][]>();
    for (const fn of layout.functions) {
      map.set(
        fn.id,
        fn.code.split("\n").map((line) => tokenizePythonLine(line))
      );
    }
    return map;
  }, [layout.functions]);

  const drawScene = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const { width: vw, height: vh, dpr } = sizeRef.current;
    const { tx, ty, scale } = vpRef.current;
    const lod = getLodLevel(scale);
    const activeId = hoveredIdRef.current ?? selectedIdRef.current;

    // 1. Clear & dot background in screen space
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = "#0b0f17";
    ctx.fillRect(0, 0, vw, vh);

    // Subtle screen-space grid dots
    const gridStep = 28;
    const offsetX = ((tx % gridStep) + gridStep) % gridStep;
    const offsetY = ((ty % gridStep) + gridStep) % gridStep;
    ctx.fillStyle = "rgba(148, 163, 184, 0.14)";
    for (let gx = offsetX; gx < vw; gx += gridStep) {
      for (let gy = offsetY; gy < vh; gy += gridStep) {
        ctx.fillRect(gx, gy, 1.2, 1.2);
      }
    }

    // Visible world bounds for culling
    const inv = 1 / Math.max(0.05, scale);
    const pad = 200 * inv;
    const viewWorld: Rect = {
      x: -tx * inv - pad,
      y: -ty * inv - pad,
      width: vw * inv + pad * 2,
      height: vh * inv + pad * 2,
    };

    // 2. World-space transform for boxes & edges
    ctx.setTransform(dpr * scale, 0, 0, dpr * scale, dpr * tx, dpr * ty);

    // Draw visible groups (parent-first order)
    const visibleGroups: PositionedGroup[] = [];
    for (const g of layout.groups) {
      if (!rectsIntersect(g, viewWorld)) continue;
      visibleGroups.push(g);

      const isSel = selectedIdRef.current === g.id;
      const isHov = hoveredIdRef.current === g.id;
      const dimmedClass = lod === "macro" && g.kind === "class";

      ctx.save();
      if (dimmedClass) ctx.globalAlpha = 0.28;

      roundRectPath(ctx, g.x, g.y, g.width, g.height, 12);

      if (g.kind === "directory") {
        ctx.fillStyle = "rgba(17, 24, 39, 0.65)";
        ctx.fill();
        ctx.setLineDash([10, 6]);
        ctx.strokeStyle = isSel ? "#ffffff" : isHov ? "#38bdf8" : "rgba(56, 189, 248, 0.4)";
        ctx.lineWidth = (isSel ? 2.5 : 1.8) / Math.max(0.3, scale);
        ctx.stroke();
      } else if (g.kind === "module") {
        ctx.fillStyle = "#141d2f";
        ctx.fill();
        ctx.strokeStyle = isSel ? "#ffffff" : isHov ? "#38bdf8" : "rgba(99, 102, 241, 0.5)";
        ctx.lineWidth = (isSel ? 2.4 : 1.5) / Math.max(0.35, scale);
        ctx.stroke();
      } else {
        ctx.fillStyle = "rgba(30, 27, 54, 0.78)";
        ctx.fill();
        ctx.strokeStyle = isSel ? "#ffffff" : isHov ? "#38bdf8" : "rgba(167, 139, 250, 0.48)";
        ctx.lineWidth = (isSel ? 2.2 : 1.3) / Math.max(0.45, scale);
        ctx.stroke();
      }
      ctx.restore();
    }

    // Draw visible functions (skip in macro overview for speed & clarity)
    const visibleFns: PositionedFunction[] = [];
    if (lod !== "macro") {
      for (const fn of layout.functions) {
        if (!rectsIntersect(fn, viewWorld)) continue;
        visibleFns.push(fn);

        const isSel = selectedIdRef.current === fn.id;
        const isHov = hoveredIdRef.current === fn.id;

        roundRectPath(ctx, fn.x, fn.y, fn.width, fn.height, 9);
        ctx.fillStyle = "#0d1320";
        ctx.fill();
        ctx.strokeStyle = isSel ? "#ffffff" : isHov ? "#34d399" : "#3b4c6e";
        ctx.lineWidth = isSel ? 2.2 : 1.4;
        ctx.stroke();

        // Function header bar
        roundRectPath(ctx, fn.x, fn.y, fn.width, 46, 9);
        ctx.fillStyle = "#172033";
        ctx.fill();

        if (lod === "micro") {
          // Header text in world space
          ctx.font = `700 13px ${FONT_SANS}`;
          ctx.fillStyle = "#e5edf7";
          ctx.fillText(fn.name, fn.x + 12, fn.y + 20, fn.width - 90);

          ctx.font = `400 10.5px ${FONT_MONO}`;
          ctx.fillStyle = "#64748b";
          ctx.fillText(`L${fn.startLine}-${fn.endLine}`, fn.x + fn.width - 66, fn.y + 19);

          ctx.font = `400 11px ${FONT_MONO}`;
          ctx.fillStyle = "#94a3b8";
          ctx.fillText(fn.signature, fn.x + 12, fn.y + 37, fn.width - 24);

          // Code lines
          const tokenLines = tokenizedByFn.get(fn.id) ?? [];
          const maxLines = Math.floor((fn.height - 58) / 17.5);
          const lineCount = Math.min(tokenLines.length, maxLines);
          ctx.font = `400 11.5px ${FONT_MONO}`;

          for (let i = 0; i < lineCount; i++) {
            const ly = fn.y + 64 + i * 17.5;
            ctx.fillStyle = "#475569";
            ctx.fillText(String(fn.startLine + i), fn.x + 10, ly);

            let lx = fn.x + 44;
            const maxX = fn.x + fn.width - 10;
            for (const tok of tokenLines[i]) {
              if (lx >= maxX) break;
              ctx.fillStyle = TOKEN_COLORS[tok.kind];
              ctx.fillText(tok.text, lx, ly);
              lx += ctx.measureText(tok.text).width;
            }
          }
        } else {
          // Meso mode: show docstring in card body; title is drawn crisp in screen space below
          if (fn.docstring) {
            ctx.font = `400 12px ${FONT_SANS}`;
            ctx.fillStyle = "#94a3b8";
            ctx.fillText(fn.docstring, fn.x + 12, fn.y + 70, fn.width - 24);
          }
        }
      }
    }

    // Draw edges in world space
    for (const edge of layout.edges) {
      if (edge.kind === "import" && !showImportRef.current) continue;
      if (edge.kind === "call" && !showCallRef.current) continue;
      const edgeBox: Rect = {
        x: Math.min(edge.x1, edge.x2) - 60,
        y: Math.min(edge.y1, edge.y2) - 60,
        width: Math.abs(edge.x2 - edge.x1) + 120,
        height: Math.abs(edge.y2 - edge.y1) + 120,
      };
      if (!rectsIntersect(edgeBox, viewWorld)) continue;

      const isConnected =
        activeId !== null && (edge.sourceId === activeId || edge.targetId === activeId);
      const dimmed = activeId !== null && !isConnected;
      drawEdge(ctx, edge, scale, isConnected, dimmed);
    }

    // 3. Screen-space crisp headers (progressive disclosure as you zoom in):
    // - macro: Directory titles only
    // - meso:  Directory + Module + Class + Function titles
    // - micro: Directory + Module + Class titles + Function code cards (in-card headers & code)
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    for (const g of visibleGroups) {
      if (lod === "macro" && g.kind !== "directory") continue;

      const sx = g.x * scale + tx;
      const sy = g.y * scale + ty;
      const sw = g.width * scale;
      if (sw < 28) continue;

      const isDir = g.kind === "directory";
      const isMod = g.kind === "module";
      const fontSize = isDir ? 13 : isMod ? 12 : 11.5;
      ctx.font = `600 ${fontSize}px ${FONT_SANS}`;

      const label = isMod ? `${g.name} (${g.functionCount})` : g.name;
      const textW = ctx.measureText(label).width;
      const pillW = Math.min(Math.max(sw - 8, 64), textW + 18);
      const pillH = fontSize + 10;

      roundRectPath(ctx, sx + 4, sy + 4, pillW, pillH, 6);
      ctx.fillStyle = isDir
        ? "rgba(11, 15, 23, 0.9)"
        : isMod
        ? "rgba(17, 24, 39, 0.92)"
        : "rgba(30, 27, 54, 0.92)";
      ctx.fill();
      ctx.strokeStyle = isDir
        ? "rgba(56, 189, 248, 0.45)"
        : isMod
        ? "rgba(99, 102, 241, 0.45)"
        : "rgba(167, 139, 250, 0.45)";
      ctx.lineWidth = 1;
      ctx.stroke();

      ctx.fillStyle = isDir ? "#38bdf8" : isMod ? "#c7d2fe" : "#ddd6fe";
      ctx.fillText(label, sx + 12, sy + 4 + fontSize + 2, pillW - 14);
    }

    if (lod === "meso") {
      ctx.font = `600 11.5px ${FONT_SANS}`;
      for (const fn of visibleFns) {
        const sx = fn.x * scale + tx;
        const sy = fn.y * scale + ty;
        const sw = fn.width * scale;
        if (sw < 40) continue;

        const label = `${fn.name}()`;
        const textW = ctx.measureText(label).width;
        const pillW = Math.min(Math.max(sw - 4, 60), textW + 16);
        const pillH = 22;

        roundRectPath(ctx, sx + 2, sy + 2, pillW, pillH, 5);
        ctx.fillStyle = "rgba(23, 32, 51, 0.95)";
        ctx.fill();
        ctx.strokeStyle =
          selectedIdRef.current === fn.id ? "#34d399" : "rgba(59, 76, 110, 0.8)";
        ctx.lineWidth = 1;
        ctx.stroke();

        ctx.fillStyle = "#34d399";
        ctx.fillText(label, sx + 9, sy + 16.5, pillW - 12);
      }
    }
  }, [layout, tokenizedByFn]);

  const scheduleDraw = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = null;
      drawScene();
    });
  }, [drawScene]);

  // ResizeObserver for crisp HiDPI canvas
  useEffect(() => {
    const el = containerRef.current;
    const canvas = canvasRef.current;
    if (!el || !canvas) return;

    const updateSize = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = el.clientWidth;
      const h = el.clientHeight;
      sizeRef.current = { width: w, height: h, dpr };
      canvas.width = Math.max(1, Math.floor(w * dpr));
      canvas.height = Math.max(1, Math.floor(h * dpr));
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
      scheduleDraw();
    };

    updateSize();
    const ro = new ResizeObserver(updateSize);
    ro.observe(el);
    return () => ro.disconnect();
  }, [scheduleDraw]);

  // Sync external viewport changes (Fit All, Zoom buttons, Sidebar click)
  useEffect(() => {
    if (viewport === lastCommittedRef.current) return;
    vpRef.current = viewport;
    setRenderVp(viewport);
    scheduleDraw();
  }, [viewport, scheduleDraw]);

  // Redraw when layout, selection, or edge toggles change
  useEffect(() => {
    scheduleDraw();
  }, [layout, selectedId, showImportEdges, showCallEdges, scheduleDraw]);

  const commitNow = useCallback(() => {
    if (commitTimerRef.current !== null) {
      window.clearTimeout(commitTimerRef.current);
      commitTimerRef.current = null;
    }
    const snap = { ...vpRef.current };
    lastCommittedRef.current = snap;
    setRenderVp(snap);
    onViewportCommit(snap);
  }, [onViewportCommit]);

  const scheduleCommit = useCallback(
    (immediate = false) => {
      const prevLod = getLodLevel(renderVp.scale);
      const nextLod = getLodLevel(vpRef.current.scale);
      if (immediate || prevLod !== nextLod) {
        commitNow();
        return;
      }
      if (commitTimerRef.current !== null) {
        window.clearTimeout(commitTimerRef.current);
      }
      commitTimerRef.current = window.setTimeout(commitNow, 120);
    },
    [commitNow, renderVp.scale]
  );

  useEffect(() => {
    return () => {
      if (rafRef.current !== null) {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      if (commitTimerRef.current !== null) {
        window.clearTimeout(commitTimerRef.current);
        commitTimerRef.current = null;
      }
    };
  }, []);

  const hitTest = useCallback(
    (clientX: number, clientY: number): { kind: "fn"; node: PositionedFunction } | { kind: "group"; node: PositionedGroup } | null => {
      const el = containerRef.current;
      if (!el) return null;
      const rect = el.getBoundingClientRect();
      const sx = clientX - rect.left;
      const sy = clientY - rect.top;
      const { tx, ty, scale } = vpRef.current;
      const wx = (sx - tx) / scale;
      const wy = (sy - ty) / scale;

      if (getLodLevel(scale) !== "macro") {
        for (let i = layout.functions.length - 1; i >= 0; i--) {
          const fn = layout.functions[i];
          if (pointInRect(wx, wy, fn)) {
            return { kind: "fn", node: fn };
          }
        }
      }

      for (let i = layout.groups.length - 1; i >= 0; i--) {
        const g = layout.groups[i];
        if (pointInRect(wx, wy, g)) {
          return { kind: "group", node: g };
        }
      }
      return null;
    },
    [layout]
  );

  const dragRef = useRef<{
    active: boolean;
    startX: number;
    startY: number;
    origTx: number;
    origTy: number;
    moved: boolean;
  }>({ active: false, startX: 0, startY: 0, origTx: 0, origTy: 0, moved: false });

  const handleWheel = useCallback(
    (e: React.WheelEvent<HTMLDivElement>) => {
      e.preventDefault();
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const mouseX = e.clientX - rect.left;
      const mouseY = e.clientY - rect.top;

      const cur = vpRef.current;
      const factor = Math.exp(-e.deltaY * 0.0014);
      const nextScale = Math.min(2.6, Math.max(0.06, cur.scale * factor));
      const ratio = nextScale / cur.scale;

      vpRef.current = {
        scale: nextScale,
        tx: mouseX - (mouseX - cur.tx) * ratio,
        ty: mouseY - (mouseY - cur.ty) * ratio,
      };
      scheduleDraw();
      scheduleCommit(false);
    },
    [scheduleDraw, scheduleCommit]
  );

  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0 && e.button !== 1) return;
    dragRef.current = {
      active: true,
      startX: e.clientX,
      startY: e.clientY,
      origTx: vpRef.current.tx,
      origTy: vpRef.current.ty,
      moved: false,
    };
  }, []);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (dragRef.current.active) return;
      const hit = hitTest(e.clientX, e.clientY);
      const nextHover = hit ? hit.node.id : null;
      if (hoveredIdRef.current !== nextHover) {
        hoveredIdRef.current = nextHover;
        scheduleDraw();
      }
    },
    [hitTest, scheduleDraw]
  );

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const d = dragRef.current;
      if (!d.active) return;
      const dx = e.clientX - d.startX;
      const dy = e.clientY - d.startY;
      if (!d.moved && (Math.abs(dx) > 3 || Math.abs(dy) > 3)) {
        d.moved = true;
        setIsDragging(true);
      }
      vpRef.current = {
        ...vpRef.current,
        tx: d.origTx + dx,
        ty: d.origTy + dy,
      };
      scheduleDraw();
    };

    const onUp = () => {
      if (!dragRef.current.active) return;
      const moved = dragRef.current.moved;
      dragRef.current.active = false;
      setIsDragging(false);
      if (moved) {
        scheduleCommit(true);
      }
    };

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [scheduleDraw, scheduleCommit]);

  const handleClick = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (dragRef.current.moved) return;
      const hit = hitTest(e.clientX, e.clientY);
      onSelect(hit ? hit.node.id : null);
    },
    [hitTest, onSelect]
  );

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const hit = hitTest(e.clientX, e.clientY);
      if (!hit) return;
      onSelect(hit.node.id);
      if (hit.kind === "fn") {
        onFocusRect(hit.node, 1.05);
      } else {
        onFocusRect(hit.node);
      }
    },
    [hitTest, onSelect, onFocusRect]
  );

  // Selected function DOM overlay at micro zoom so users can select/copy/scroll long code
  const selectedFnOverlay = useMemo(() => {
    if (!selectedId || getLodLevel(renderVp.scale) !== "micro") return null;
    const fn = layout.functions.find((f) => f.id === selectedId);
    if (!fn) return null;
    return fn;
  }, [selectedId, renderVp.scale, layout.functions]);

  return (
    <div
      ref={containerRef}
      className={`canvas-viewport ${isDragging ? "is-dragging" : ""}`}
      onWheel={handleWheel}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseLeave={() => {
        if (hoveredIdRef.current !== null) {
          hoveredIdRef.current = null;
          scheduleDraw();
        }
      }}
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
    >
      <canvas ref={canvasRef} className="canvas-2d" />

      {selectedFnOverlay && !isDragging && (
        <div
          className="fn-code-overlay"
          style={{
            left: selectedFnOverlay.x * renderVp.scale + renderVp.tx + 2,
            top: (selectedFnOverlay.y + 48) * renderVp.scale + renderVp.ty,
            width: (selectedFnOverlay.width - 4) * renderVp.scale,
            height: (selectedFnOverlay.height - 50) * renderVp.scale,
            fontSize: `${11.5 * renderVp.scale}px`,
          }}
          onMouseDown={(e) => e.stopPropagation()}
          onWheel={(e) => e.stopPropagation()}
        >
          <pre className="fn-code">
            {selectedFnOverlay.code.split("\n").map((line, idx) => (
              <div key={idx} className="code-line">
                <span className="line-num">{selectedFnOverlay.startLine + idx}</span>
                <span className="line-text">{highlightPython(line)}</span>
              </div>
            ))}
          </pre>
        </div>
      )}
    </div>
  );
};
