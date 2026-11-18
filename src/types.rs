use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    #[serde(default)]
    pub complexity: usize,
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
    #[serde(default)]
    pub open_in_terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub pr_ref: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub file_path: String,
    pub abs_path: String,
    pub status: DiffStatus,
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub provider: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub base_branch: String,
    pub head_branch: String,
    pub head_sha: String,
    #[serde(default)]
    pub repo_name: String,
    #[serde(default)]
    pub checkout_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrListItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    #[serde(default)]
    pub repo_name: String,
    pub head_branch: String,
    pub base_branch: String,
    pub draft: bool,
    pub is_author: bool,
    pub is_reviewer: bool,
    pub is_assigned: bool,
    #[serde(default)]
    pub is_involved: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffSummary {
    pub branch: String,
    pub base_ref: String,
    pub files: Vec<FileDiff>,
    pub pr: Option<PrInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeProgress {
    pub phase: String,
    pub processed: usize,
    pub total: usize,
    pub elapsed_ms: u64,
    pub eta_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: String,
    pub details_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCommit {
    pub sha: String,
    pub message: String,
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReaction {
    pub content: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrIssueComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
    #[serde(default)]
    pub reactions: Vec<PrReaction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub path: String,
    pub line: Option<usize>,
    pub side: String,
    pub reactions: Vec<PrReaction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrReviewThread {
    pub thread_id: String,
    pub path: String,
    pub line: Option<usize>,
    pub side: String,
    pub is_resolved: bool,
    pub comments: Vec<PrReviewComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDetails {
    pub provider: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub author: String,
    pub url: String,
    pub head_sha: String,
    pub mergeable: String,
    pub auto_merge: bool,
    pub current_user: Option<String>,
    pub labels: Vec<PrLabel>,
    pub assignees: Vec<String>,
    pub reviewers: Vec<String>,
    pub checks: Vec<PrCheckRun>,
    pub commits: Vec<PrCommit>,
    pub comments: Vec<PrIssueComment>,
    pub threads: Vec<PrReviewThread>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingReviewComment {
    pub path: String,
    pub line: usize,
    pub side: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrSuggestions {
    pub labels: Vec<String>,
    pub users: Vec<String>,
}
