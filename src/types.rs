use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupKind {
    Directory,
    Module,
    Class,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupNode {
    pub id: String,
    pub kind: GroupKind,
    pub name: String,
    pub qualified_name: String,
    pub parent_id: Option<String>,
    pub file_path: Option<String>,
    pub is_package: bool,
    pub docstring: Option<String>,
    pub line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionNode {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub parent_id: String,
    pub file_path: String,
    pub is_async: bool,
    pub is_method: bool,
    pub decorators: Vec<String>,
    pub signature: String,
    pub docstring: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeKind {
    Import,
    Call,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub kind: EdgeKind,
    pub source_id: String,
    pub target_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseError {
    pub file_path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStats {
    pub total_files: usize,
    pub total_classes: usize,
    pub total_functions: usize,
    pub total_import_edges: usize,
    pub total_call_edges: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGraph {
    pub root_name: String,
    pub root_path: String,
    pub groups: Vec<GroupNode>,
    pub functions: Vec<FunctionNode>,
    pub edges: Vec<GraphEdge>,
    pub parse_errors: Vec<ParseError>,
    pub stats: ProjectStats,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    pub show_import_edges: bool,
    pub show_call_edges: bool,
    pub hidden_import_group_ids: Vec<String>,
    pub hidden_call_group_ids: Vec<String>,
    pub editor_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PositionedGroup {
    pub node: GroupNode,
    pub rect: Rect,
    pub depth: usize,
    pub function_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PositionedFunction {
    pub node: FunctionNode,
    pub rect: Rect,
    pub line_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PositionedEdge {
    pub edge: GraphEdge,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutResult {
    pub groups: Vec<PositionedGroup>,
    pub functions: Vec<PositionedFunction>,
    pub edges: Vec<PositionedEdge>,
    pub node_rects: HashMap<String, Rect>,
    pub bounds: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeVisibility {
    pub show_imports: bool,
    pub show_calls: bool,
    pub hidden_import_group_ids: HashSet<String>,
    pub hidden_call_group_ids: HashSet<String>,
}

impl Default for EdgeVisibility {
    fn default() -> Self {
        Self {
            show_imports: true,
            show_calls: true,
            hidden_import_group_ids: HashSet::new(),
            hidden_call_group_ids: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 60.0,
            y: 60.0,
            scale: 0.78,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LodLevel {
    Macro,
    Meso,
    Micro,
}

impl LodLevel {
    pub fn from_scale(scale: f64) -> Self {
        if scale < 0.28 {
            Self::Macro
        } else if scale < 0.72 {
            Self::Meso
        } else {
            Self::Micro
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Macro => "Macro",
            Self::Meso => "Meso",
            Self::Micro => "Micro",
        }
    }
}
