import React from "react";
import {
  FolderOpen,
  GitBranch,
  Maximize2,
  PanelLeft,
  Search,
  Sparkles,
  Workflow,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type { LodLevel } from "./Canvas";

interface ToolbarProps {
  projectName: string;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onOpenProject: () => void;
  onLoadSample: () => void;
  loading: boolean;
  searchQuery: string;
  onSearchChange: (q: string) => void;
  showImportEdges: boolean;
  onToggleImportEdges: () => void;
  showCallEdges: boolean;
  onToggleCallEdges: () => void;
  scale: number;
  lod: LodLevel;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onFitAll: () => void;
}

const LOD_LABELS: Record<LodLevel, string> = {
  macro: "Overview (Groups)",
  meso: "Structure (Signatures)",
  micro: "Code (Readable)",
};

export const Toolbar: React.FC<ToolbarProps> = ({
  projectName,
  sidebarOpen,
  onToggleSidebar,
  onOpenProject,
  onLoadSample,
  loading,
  searchQuery,
  onSearchChange,
  showImportEdges,
  onToggleImportEdges,
  showCallEdges,
  onToggleCallEdges,
  scale,
  lod,
  onZoomIn,
  onZoomOut,
  onFitAll,
}) => {
  return (
    <header className="toolbar">
      <div className="toolbar-left">
        <button
          className={`icon-btn ${sidebarOpen ? "is-active" : ""}`}
          onClick={onToggleSidebar}
          title="Toggle Project Explorer"
        >
          <PanelLeft size={16} />
        </button>
        <div className="brand">
          <span className="brand-logo">VCode</span>
          <span className="brand-project" title={projectName}>
            {projectName}
          </span>
        </div>
        <button className="btn btn-primary" onClick={onOpenProject} disabled={loading}>
          <FolderOpen size={15} />
          <span>{loading ? "Analyzing..." : "Open Project"}</span>
        </button>
        <button className="btn btn-ghost" onClick={onLoadSample} disabled={loading} title="Load built-in sample project">
          <Sparkles size={14} />
          <span>Sample</span>
        </button>
      </div>

      <div className="toolbar-center">
        <div className="search-box">
          <Search size={14} className="search-icon" />
          <input
            type="text"
            placeholder="Filter groups, classes, functions..."
            value={searchQuery}
            onChange={(e) => onSearchChange(e.target.value)}
          />
          {searchQuery && (
            <button className="search-clear" onClick={() => onSearchChange("")}>
              ×
            </button>
          )}
        </div>
      </div>

      <div className="toolbar-right">
        <button
          className={`toggle-pill import-pill ${showImportEdges ? "is-active" : ""}`}
          onClick={onToggleImportEdges}
          title="Toggle module import edges"
        >
          <GitBranch size={13} />
          <span>Imports</span>
        </button>
        <button
          className={`toggle-pill call-pill ${showCallEdges ? "is-active" : ""}`}
          onClick={onToggleCallEdges}
          title="Toggle function call edges"
        >
          <Workflow size={13} />
          <span>Calls</span>
        </button>

        <div className="divider" />

        <span className={`lod-badge lod-badge-${lod}`} title="Current semantic zoom level">
          {LOD_LABELS[lod]}
        </span>

        <div className="zoom-controls">
          <button className="icon-btn" onClick={onZoomOut} title="Zoom out">
            <ZoomOut size={15} />
          </button>
          <span className="zoom-pct">{Math.round(scale * 100)}%</span>
          <button className="icon-btn" onClick={onZoomIn} title="Zoom in">
            <ZoomIn size={15} />
          </button>
          <button className="icon-btn" onClick={onFitAll} title="Fit entire project">
            <Maximize2 size={14} />
          </button>
        </div>
      </div>
    </header>
  );
};
