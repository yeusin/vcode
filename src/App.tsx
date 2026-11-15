import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Canvas, getLodLevel, type Viewport } from "./components/Canvas";
import { Sidebar } from "./components/Sidebar";
import { Toolbar } from "./components/Toolbar";
import { SAMPLE_PROJECT } from "./data/sampleProject";
import { computeLayout } from "./layout/layoutEngine";
import type { FunctionNode, GroupNode, ProjectGraph, Rect } from "./types/graph";

export const App: React.FC = () => {
  const [graph, setGraph] = useState<ProjectGraph>(SAMPLE_PROJECT);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showImportEdges, setShowImportEdges] = useState(true);
  const [showCallEdges, setShowCallEdges] = useState(true);
  const [viewport, setViewport] = useState<Viewport>({ tx: 40, ty: 40, scale: 0.45 });

  const stageRef = useRef<HTMLDivElement | null>(null);
  const layout = useMemo(() => computeLayout(graph), [graph]);

  const fitRect = useCallback(
    (rect: Rect, minScale?: number) => {
      const stage = stageRef.current;
      const vw = stage ? stage.clientWidth : window.innerWidth - (sidebarOpen ? 280 : 0);
      const vh = stage ? stage.clientHeight : window.innerHeight - 52;
      const pad = 48;
      const fitScale = Math.min(
        (vw - pad * 2) / Math.max(1, rect.width),
        (vh - pad * 2) / Math.max(1, rect.height)
      );
      const scale = Math.min(2.2, Math.max(0.08, minScale ? Math.max(fitScale, minScale) : fitScale));
      const tx = vw / 2 - (rect.x + rect.width / 2) * scale;
      const ty = vh / 2 - (rect.y + rect.height / 2) * scale;
      setViewport({ tx, ty, scale });
    },
    [sidebarOpen]
  );

  const fitAll = useCallback(() => {
    fitRect({ x: 0, y: 0, width: layout.bounds.width, height: layout.bounds.height });
  }, [fitRect, layout.bounds.width, layout.bounds.height]);

  useEffect(() => {
    fitAll();
  }, [fitAll]);

  const handleOpenProject = async () => {
    setErrorMsg(null);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Project Folder",
      });
      if (!selected || Array.isArray(selected)) return;

      setLoading(true);
      const analyzed = await invoke<ProjectGraph>("analyze_project", {
        projectPath: selected,
      });
      setGraph(analyzed);
      setSelectedId(null);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setErrorMsg(msg);
    } finally {
      setLoading(false);
    }
  };

  const handleLoadSample = () => {
    setErrorMsg(null);
    setGraph(SAMPLE_PROJECT);
    setSelectedId(null);
  };

  const handleSelectGroup = (group: GroupNode) => {
    setSelectedId(group.id);
    const rect = layout.nodeRects[group.id];
    if (rect) {
      fitRect(rect);
    }
  };

  const handleSelectFunction = (fn: FunctionNode) => {
    setSelectedId(fn.id);
    const rect = layout.nodeRects[fn.id];
    if (rect) {
      fitRect(rect, 1.05);
    }
  };

  const zoomBy = (factor: number) => {
    const stage = stageRef.current;
    const cx = stage ? stage.clientWidth / 2 : 500;
    const cy = stage ? stage.clientHeight / 2 : 350;
    const nextScale = Math.min(2.6, Math.max(0.06, viewport.scale * factor));
    const ratio = nextScale / viewport.scale;
    setViewport({
      scale: nextScale,
      tx: cx - (cx - viewport.tx) * ratio,
      ty: cy - (cy - viewport.ty) * ratio,
    });
  };

  return (
    <div className="app-shell">
      <Toolbar
        projectName={graph.rootName}
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen((v) => !v)}
        onOpenProject={handleOpenProject}
        onLoadSample={handleLoadSample}
        loading={loading}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        showImportEdges={showImportEdges}
        onToggleImportEdges={() => setShowImportEdges((v) => !v)}
        showCallEdges={showCallEdges}
        onToggleCallEdges={() => setShowCallEdges((v) => !v)}
        scale={viewport.scale}
        lod={getLodLevel(viewport.scale)}
        onZoomIn={() => zoomBy(1.25)}
        onZoomOut={() => zoomBy(0.8)}
        onFitAll={fitAll}
      />

      {errorMsg && (
        <div className="error-banner">
          <span>{errorMsg}</span>
          <button onClick={() => setErrorMsg(null)}>Dismiss</button>
        </div>
      )}

      <div className="workspace">
        {sidebarOpen && (
          <Sidebar
            graph={graph}
            searchQuery={searchQuery}
            selectedId={selectedId}
            onSelectGroup={handleSelectGroup}
            onSelectFunction={handleSelectFunction}
          />
        )}

        <main ref={stageRef} className="canvas-stage">
          <Canvas
            layout={layout}
            viewport={viewport}
            onViewportCommit={setViewport}
            selectedId={selectedId}
            onSelect={setSelectedId}
            showImportEdges={showImportEdges}
            showCallEdges={showCallEdges}
            onFocusRect={fitRect}
          />
          <div className="canvas-hint">
            Scroll to zoom (overview → structure → readable code) · Drag to pan · Double-click any diagram to focus
          </div>
        </main>
      </div>
    </div>
  );
};

export default App;
