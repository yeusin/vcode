import React, { useEffect, useMemo, useRef } from "react";
import {
  AlertTriangle,
  Box,
  Code2,
  FileCode2,
  Folder,
  FolderTree,
  GitBranch,
  Search,
  Workflow,
} from "lucide-react";
import type { FunctionNode, GroupNode, ProjectGraph } from "../types/graph";

interface SidebarProps {
  graph: ProjectGraph;
  searchQuery: string;
  onSearchChange: (q: string) => void;
  selectedId: string | null;
  onSelectGroup: (group: GroupNode) => void;
  onSelectFunction: (fn: FunctionNode) => void;
  hiddenImportGroupIds: Set<string>;
  hiddenCallGroupIds: Set<string>;
  onToggleGroupImport: (groupId: string) => void;
  onToggleGroupCall: (groupId: string) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  graph,
  searchQuery,
  onSearchChange,
  selectedId,
  onSelectGroup,
  onSelectFunction,
  hiddenImportGroupIds,
  hiddenCallGroupIds,
  onToggleGroupImport,
  onToggleGroupCall,
}) => {
  const treeRef = useRef<HTMLDivElement | null>(null);
  const query = searchQuery.trim().toLowerCase();

  useEffect(() => {
    if (!selectedId || !treeRef.current) return;
    const el = treeRef.current.querySelector<HTMLElement>(
      `[data-node-id="${CSS.escape(selectedId)}"]`
    );
    if (el) {
      el.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }
  }, [selectedId]);

  const { filteredGroups, filteredFunctions } = useMemo(() => {
    if (!query) {
      return { filteredGroups: graph.groups, filteredFunctions: graph.functions };
    }
    const fns = graph.functions.filter(
      (f) =>
        f.name.toLowerCase().includes(query) ||
        f.qualifiedName.toLowerCase().includes(query)
    );
    const fnParents = new Set(fns.map((f) => f.parentId));
    const groups = graph.groups.filter(
      (g) =>
        g.name.toLowerCase().includes(query) ||
        g.qualifiedName.toLowerCase().includes(query) ||
        fnParents.has(g.id)
    );
    return { filteredGroups: groups, filteredFunctions: fns };
  }, [graph.groups, graph.functions, query]);

  const fnsByParent = useMemo(() => {
    const map = new Map<string, FunctionNode[]>();
    for (const fn of filteredFunctions) {
      const list = map.get(fn.parentId) ?? [];
      list.push(fn);
      map.set(fn.parentId, list);
    }
    return map;
  }, [filteredFunctions]);

  const groupsByParent = useMemo(() => {
    const map = new Map<string, GroupNode[]>();
    for (const g of filteredGroups) {
      const key = g.parentId ?? "__root__";
      const list = map.get(key) ?? [];
      list.push(g);
      map.set(key, list);
    }
    return map;
  }, [filteredGroups]);

  const renderGroupRow = (g: GroupNode, paddingLeft: number, label: string) => {
    const importHidden = hiddenImportGroupIds.has(g.id);
    const callHidden = hiddenCallGroupIds.has(g.id);

    return (
      <div
        data-node-id={g.id}
        className={`tree-row ${selectedId === g.id ? "is-selected" : ""} ${
          importHidden || callHidden ? "has-hidden-edges" : ""
        }`}
      >
        <button
          className={`tree-item tree-${g.kind}`}
          style={{ paddingLeft }}
          onClick={() => onSelectGroup(g)}
          title={g.qualifiedName}
        >
          <span className="tree-icon">
            {g.kind === "directory" ? (
              g.isPackage ? (
                <FolderTree size={14} />
              ) : (
                <Folder size={14} />
              )
            ) : g.kind === "module" ? (
              <FileCode2 size={14} />
            ) : (
              <Box size={13} />
            )}
          </span>
          <span className="tree-label">{label}</span>
        </button>

        <div className="tree-edge-toggles">
          <button
            className={`tree-edge-btn ${importHidden ? "is-hidden" : ""}`}
            onClick={(e) => {
              e.stopPropagation();
              onToggleGroupImport(g.id);
            }}
            title={
              importHidden
                ? `Show import edges for ${g.name}`
                : `Hide import edges for ${g.name}`
            }
          >
            <GitBranch size={11} />
          </button>
          <button
            className={`tree-edge-btn ${callHidden ? "is-hidden" : ""}`}
            onClick={(e) => {
              e.stopPropagation();
              onToggleGroupCall(g.id);
            }}
            title={
              callHidden
                ? `Show call edges for ${g.name}`
                : `Hide call edges for ${g.name}`
            }
          >
            <Workflow size={11} />
          </button>
        </div>
      </div>
    );
  };

  const renderGroupTree = (parentKey: string, depth: number): React.ReactNode => {
    const children = groupsByParent.get(parentKey) ?? [];
    return children.map((g) => {
      const directFns = fnsByParent.get(g.id) ?? [];
      return (
        <div key={g.id}>
          {renderGroupRow(g, 10 + depth * 14, g.name)}
          {renderGroupTree(g.id, depth + 1)}
          {directFns.map((fn) => (
            <button
              key={fn.id}
              data-node-id={fn.id}
              className={`tree-item tree-fn ${selectedId === fn.id ? "is-selected" : ""}`}
              style={{ paddingLeft: 10 + (depth + 1) * 14 }}
              onClick={() => onSelectFunction(fn)}
              title={`${fn.qualifiedName}${fn.signature}`}
            >
              <span className="tree-icon">
                <Code2 size={13} />
              </span>
              <span className="tree-label">{fn.name}()</span>
            </button>
          ))}
        </div>
      );
    });
  };

  return (
    <aside className="sidebar">
      <div className="sidebar-stats">
        <div className="stat-chip">
          <span className="stat-val">{graph.stats.totalFiles}</span>
          <span className="stat-lbl">Files</span>
        </div>
        <div className="stat-chip">
          <span className="stat-val">{graph.stats.totalClasses}</span>
          <span className="stat-lbl">Classes</span>
        </div>
        <div className="stat-chip">
          <span className="stat-val">{graph.stats.totalFunctions}</span>
          <span className="stat-lbl">Functions</span>
        </div>
      </div>

      <div className="sidebar-section-title">Project Structure</div>
      <div className="sidebar-search">
        <div className="search-box">
          <Search size={14} className="search-icon" />
          <input
            type="text"
            placeholder="Filter files, classes, functions..."
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
      <div ref={treeRef} className="sidebar-tree">
        {query && filteredGroups.length === 0 && filteredFunctions.length === 0 ? (
          <div className="sidebar-empty">No matches for "{searchQuery}"</div>
        ) : query ? (
          <>
            {filteredGroups.map((g) => (
              <React.Fragment key={g.id}>
                {renderGroupRow(g, 10, g.qualifiedName)}
              </React.Fragment>
            ))}
            {filteredFunctions.map((fn) => (
              <button
                key={fn.id}
                data-node-id={fn.id}
                className={`tree-item tree-fn ${selectedId === fn.id ? "is-selected" : ""}`}
                onClick={() => onSelectFunction(fn)}
              >
                <Code2 size={13} />
                <span className="tree-label">{fn.qualifiedName}()</span>
              </button>
            ))}
          </>
        ) : (
          renderGroupTree("__root__", 0)
        )}
      </div>

      {graph.parseErrors.length > 0 && (
        <div className="sidebar-errors">
          <div className="errors-header">
            <AlertTriangle size={14} />
            <span>{graph.parseErrors.length} Parse Error(s)</span>
          </div>
          <div className="errors-list">
            {graph.parseErrors.map((err, idx) => (
              <div key={idx} className="error-item">
                <div className="error-file">{err.filePath}</div>
                <div className="error-msg">{err.error}</div>
              </div>
            ))}
          </div>
        </div>
      )}
    </aside>
  );
};
