import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Canvas, getLodLevel, type Viewport } from "./components/Canvas";
import { Sidebar } from "./components/Sidebar";
import { Toolbar, type RecentProject } from "./components/Toolbar";
import { SAMPLE_PROJECT } from "./data/sampleProject";
import { computeLayout } from "./layout/layoutEngine";
import type { FunctionNode, GroupNode, ProjectGraph, Rect } from "./types/graph";

const RECENT_STORAGE_KEY = "vcode.recentProjects";
const EDITOR_STORAGE_KEY = "vcode.editorCommand";
const MAX_RECENT_PROJECTS = 5;

interface ProjectSettingsPayload {
  showImportEdges: boolean;
  showCallEdges: boolean;
  hiddenImportGroupIds: string[];
  hiddenCallGroupIds: string[];
  editorCommand: string;
}

function loadEditorCommand(): string {
  try {
    return localStorage.getItem(EDITOR_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

function loadRecentProjects(): RecentProject[] {
  try {
    const raw = localStorage.getItem(RECENT_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is RecentProject =>
        Boolean(item && typeof item.name === "string" && typeof item.path === "string")
    );
  } catch {
    return [];
  }
}

export const App: React.FC = () => {
  const [graph, setGraph] = useState<ProjectGraph>(SAMPLE_PROJECT);
  const [recentProjects, setRecentProjects] = useState<RecentProject[]>(loadRecentProjects);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showImportEdges, setShowImportEdges] = useState(true);
  const [showCallEdges, setShowCallEdges] = useState(true);
  const [hiddenImportGroupIds, setHiddenImportGroupIds] = useState<Set<string>>(() => new Set());
  const [hiddenCallGroupIds, setHiddenCallGroupIds] = useState<Set<string>>(() => new Set());
  const [editorCommand, setEditorCommand] = useState<string>(loadEditorCommand);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [editorDraft, setEditorDraft] = useState("");
  const [pendingEditorFile, setPendingEditorFile] = useState<string | null>(null);
  const [viewport, setViewport] = useState<Viewport>({ tx: 40, ty: 40, scale: 0.45 });

  const stageRef = useRef<HTMLDivElement | null>(null);
  const settingsLoadedPathRef = useRef<string | null>(null);
  const layout = useMemo(() => computeLayout(graph), [graph]);

  const hiddenEdgeIds = useMemo(() => {
    const hidden = new Set<string>();
    if (hiddenImportGroupIds.size === 0 && hiddenCallGroupIds.size === 0) {
      return hidden;
    }

    const parentMap = new Map<string, string | null>();
    for (const g of graph.groups) {
      parentMap.set(g.id, g.parentId);
    }
    for (const fn of graph.functions) {
      parentMap.set(fn.id, fn.parentId);
    }

    const isBlocked = (nodeId: string, blockedGroups: Set<string>): boolean => {
      if (blockedGroups.size === 0) return false;
      let cur: string | null | undefined = nodeId;
      while (cur) {
        if (blockedGroups.has(cur)) return true;
        cur = parentMap.get(cur);
      }
      return false;
    };

    for (const edge of graph.edges) {
      const blocked = edge.kind === "import" ? hiddenImportGroupIds : hiddenCallGroupIds;
      if (isBlocked(edge.sourceId, blocked) || isBlocked(edge.targetId, blocked)) {
        hidden.add(edge.id);
      }
    }
    return hidden;
  }, [graph, hiddenImportGroupIds, hiddenCallGroupIds]);

  const toggleGroupImport = useCallback((groupId: string) => {
    setHiddenImportGroupIds((prev) => {
      const next = new Set(prev);
      if (next.has(groupId)) next.delete(groupId);
      else next.add(groupId);
      return next;
    });
  }, []);

  const toggleGroupCall = useCallback((groupId: string) => {
    setHiddenCallGroupIds((prev) => {
      const next = new Set(prev);
      if (next.has(groupId)) next.delete(groupId);
      else next.add(groupId);
      return next;
    });
  }, []);

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

  const recordRecentProject = useCallback((name: string, path: string) => {
    setRecentProjects((prev) => {
      const next = [{ name, path }, ...prev.filter((p) => p.path !== path)].slice(
        0,
        MAX_RECENT_PROJECTS
      );
      try {
        localStorage.setItem(RECENT_STORAGE_KEY, JSON.stringify(next));
      } catch {
        // ignore storage quota errors
      }
      return next;
    });
  }, []);

  const openProjectPath = useCallback(
    async (projectPath: string) => {
      setErrorMsg(null);
      setLoading(true);
      try {
        const analyzed = await invoke<ProjectGraph>("analyze_project", {
          projectPath,
        });
        let saved: ProjectSettingsPayload | null = null;
        try {
          saved = await invoke<ProjectSettingsPayload | null>("load_project_settings", {
            projectPath: analyzed.rootPath,
          });
        } catch {
          saved = null;
        }

        settingsLoadedPathRef.current = null;
        setGraph(analyzed);
        setSelectedId(null);
        if (saved) {
          setShowImportEdges(saved.showImportEdges);
          setShowCallEdges(saved.showCallEdges);
          setHiddenImportGroupIds(new Set(saved.hiddenImportGroupIds));
          setHiddenCallGroupIds(new Set(saved.hiddenCallGroupIds));
          if (saved.editorCommand) {
            setEditorCommand(saved.editorCommand);
          }
        } else {
          setShowImportEdges(true);
          setShowCallEdges(true);
          setHiddenImportGroupIds(new Set());
          setHiddenCallGroupIds(new Set());
        }
        settingsLoadedPathRef.current = analyzed.rootPath;
        recordRecentProject(analyzed.rootName, analyzed.rootPath);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setErrorMsg(msg);
      } finally {
        setLoading(false);
      }
    },
    [recordRecentProject]
  );

  useEffect(() => {
    if (
      graph.rootPath.startsWith("sample://") ||
      settingsLoadedPathRef.current !== graph.rootPath
    ) {
      return;
    }
    const payload: ProjectSettingsPayload = {
      showImportEdges,
      showCallEdges,
      hiddenImportGroupIds: Array.from(hiddenImportGroupIds),
      hiddenCallGroupIds: Array.from(hiddenCallGroupIds),
      editorCommand,
    };
    void invoke("save_project_settings", {
      projectPath: graph.rootPath,
      settings: payload,
    }).catch(() => {
      // ignore persistence errors when running outside Tauri
    });
  }, [
    graph.rootPath,
    showImportEdges,
    showCallEdges,
    hiddenImportGroupIds,
    hiddenCallGroupIds,
    editorCommand,
  ]);

  const handleOpenProject = async () => {
    setErrorMsg(null);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Project Folder",
      });
      if (!selected || Array.isArray(selected)) return;
      await openProjectPath(selected);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setErrorMsg(msg);
    }
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

  const launchEditor = useCallback(async (cmd: string, absFilePath: string) => {
    setErrorMsg(null);
    try {
      await invoke("open_in_editor", {
        editorCommand: cmd,
        filePath: absFilePath,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setErrorMsg(msg);
    }
  }, []);

  const handleOpenFileInEditor = useCallback(
    (relFilePath: string) => {
      if (graph.rootPath.startsWith("sample://")) {
        setErrorMsg("Open a local project folder first to open files in your editor.");
        return;
      }
      const sep = graph.rootPath.endsWith("/") || graph.rootPath.endsWith("\\") ? "" : "/";
      const absPath = `${graph.rootPath}${sep}${relFilePath}`;
      if (!editorCommand.trim()) {
        setPendingEditorFile(absPath);
        setEditorDraft("");
        setSettingsOpen(true);
        return;
      }
      void launchEditor(editorCommand, absPath);
    },
    [graph.rootPath, editorCommand, launchEditor]
  );

  const handleOpenSettings = () => {
    setPendingEditorFile(null);
    setEditorDraft(editorCommand);
    setSettingsOpen(true);
  };

  const handleSaveSettings = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = editorDraft.trim();
    setEditorCommand(trimmed);
    try {
      localStorage.setItem(EDITOR_STORAGE_KEY, trimmed);
    } catch {
      // ignore storage errors
    }
    setSettingsOpen(false);
    if (trimmed && pendingEditorFile) {
      const fileToOpen = pendingEditorFile;
      setPendingEditorFile(null);
      void launchEditor(trimmed, fileToOpen);
    } else {
      setPendingEditorFile(null);
    }
  };

  return (
    <div className="app-shell">
      <Toolbar
        projectName={graph.rootName}
        currentRootPath={graph.rootPath}
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen((v) => !v)}
        onOpenProject={handleOpenProject}
        recentProjects={recentProjects}
        onOpenRecent={openProjectPath}
        loading={loading}
        showImportEdges={showImportEdges}
        onToggleImportEdges={() => setShowImportEdges((v) => !v)}
        showCallEdges={showCallEdges}
        onToggleCallEdges={() => setShowCallEdges((v) => !v)}
        scale={viewport.scale}
        lod={getLodLevel(viewport.scale)}
        onZoomIn={() => zoomBy(1.25)}
        onZoomOut={() => zoomBy(0.8)}
        onFitAll={fitAll}
        onOpenSettings={handleOpenSettings}
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
            onSearchChange={setSearchQuery}
            selectedId={selectedId}
            onSelectGroup={handleSelectGroup}
            onSelectFunction={handleSelectFunction}
            hiddenImportGroupIds={hiddenImportGroupIds}
            hiddenCallGroupIds={hiddenCallGroupIds}
            onToggleGroupImport={toggleGroupImport}
            onToggleGroupCall={toggleGroupCall}
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
            hiddenEdgeIds={hiddenEdgeIds}
            hiddenImportGroupIds={hiddenImportGroupIds}
            hiddenCallGroupIds={hiddenCallGroupIds}
            onFocusRect={fitRect}
            onOpenFileInEditor={handleOpenFileInEditor}
          />
          <div className="canvas-hint">
            Scroll to zoom (overview → structure → readable code) · Drag to pan · Double-click any diagram to focus
          </div>
        </main>
      </div>

      {settingsOpen && (
        <div
          className="modal-backdrop"
          onClick={() => {
            setSettingsOpen(false);
            setPendingEditorFile(null);
          }}
        >
          <form
            className="modal-card"
            onClick={(e) => e.stopPropagation()}
            onSubmit={handleSaveSettings}
          >
            <div className="modal-header">
              <h3>Settings</h3>
            </div>
            <div className="modal-body">
              <label className="modal-label" htmlFor="editor-cmd-input">
                {pendingEditorFile
                  ? "Set an editor command to open this file:"
                  : "Editor command"}
              </label>
              <input
                id="editor-cmd-input"
                className="modal-input"
                type="text"
                autoFocus
                placeholder="e.g. code, cursor, zed, subl, code --goto {file}"
                value={editorDraft}
                onChange={(e) => setEditorDraft(e.target.value)}
              />
              <div className="modal-presets">
                {["code", "cursor", "zed", "subl"].map((preset) => (
                  <button
                    key={preset}
                    type="button"
                    className="preset-chip"
                    onClick={() => setEditorDraft(preset)}
                  >
                    {preset}
                  </button>
                ))}
              </div>
              <div className="modal-help">
                Uses <code>{"{file}"}</code> placeholder if provided, otherwise appends the file path.
              </div>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-ghost"
                onClick={() => {
                  setSettingsOpen(false);
                  setPendingEditorFile(null);
                }}
              >
                Cancel
              </button>
              <button type="submit" className="btn btn-primary" disabled={!editorDraft.trim()}>
                {pendingEditorFile ? "Save & Open File" : "Save"}
              </button>
            </div>
          </form>
        </div>
      )}
    </div>
  );
};

export default App;
