export type GroupKind = "directory" | "module" | "class";

export interface GroupNode {
  id: string;
  kind: GroupKind;
  name: string;
  qualifiedName: string;
  parentId: string | null;
  filePath: string | null;
  isPackage: boolean;
  docstring: string | null;
  lineCount: number;
}

export interface FunctionNode {
  id: string;
  name: string;
  qualifiedName: string;
  parentId: string;
  filePath: string;
  isAsync: boolean;
  isMethod: boolean;
  decorators: string[];
  signature: string;
  docstring: string | null;
  startLine: number;
  endLine: number;
  code: string;
}

export type EdgeKind = "import" | "call";

export interface GraphEdge {
  id: string;
  kind: EdgeKind;
  sourceId: string;
  targetId: string;
  label: string | null;
}

export interface ParseError {
  filePath: string;
  error: string;
}

export interface ProjectStats {
  totalFiles: number;
  totalClasses: number;
  totalFunctions: number;
  totalImportEdges: number;
  totalCallEdges: number;
}

export interface ProjectGraph {
  rootName: string;
  rootPath: string;
  groups: GroupNode[];
  functions: FunctionNode[];
  edges: GraphEdge[];
  parseErrors: ParseError[];
  stats: ProjectStats;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PositionedGroup extends GroupNode, Rect {
  depth: number;
  functionCount: number;
}

export interface PositionedFunction extends FunctionNode, Rect {}

export interface PositionedEdge extends GraphEdge {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

export interface LayoutResult {
  groups: PositionedGroup[];
  functions: PositionedFunction[];
  edges: PositionedEdge[];
  nodeRects: Record<string, Rect>;
  bounds: { width: number; height: number };
}
