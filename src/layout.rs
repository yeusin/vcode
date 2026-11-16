use std::collections::HashMap;

use crate::types::{
    FunctionNode, GroupKind, GroupNode, LayoutResult, PositionedEdge, PositionedFunction,
    PositionedGroup, ProjectGraph, Rect,
};

struct GroupStyle {
    pad_x: f64,
    pad_top: f64,
    pad_bottom: f64,
    gap: f64,
    min_w: f64,
    min_h: f64,
}

fn group_style(kind: &GroupKind) -> GroupStyle {
    match kind {
        GroupKind::Directory => GroupStyle {
            pad_x: 36.0,
            pad_top: 72.0,
            pad_bottom: 36.0,
            gap: 40.0,
            min_w: 360.0,
            min_h: 180.0,
        },
        GroupKind::Module => GroupStyle {
            pad_x: 28.0,
            pad_top: 58.0,
            pad_bottom: 28.0,
            gap: 28.0,
            min_w: 320.0,
            min_h: 130.0,
        },
        GroupKind::Class => GroupStyle {
            pad_x: 22.0,
            pad_top: 50.0,
            pad_bottom: 22.0,
            gap: 22.0,
            min_w: 280.0,
            min_h: 110.0,
        },
    }
}

#[derive(Clone)]
enum BoxKind {
    Group,
    Function,
}

struct LayoutBox {
    id: String,
    kind: BoxKind,
    width: f64,
    height: f64,
    rel_x: f64,
    rel_y: f64,
    children: Vec<LayoutBox>,
    function_count: usize,
}

fn measure_function_box(fn_node: &FunctionNode) -> (f64, f64, usize) {
    let lines: Vec<&str> = fn_node.code.split('\n').collect();
    let max_line_len = lines
        .iter()
        .map(|l| l.chars().count())
        .fold(fn_node.name.chars().count() + fn_node.signature.chars().count(), usize::max);
    let width = (40.0 + (max_line_len as f64) * 7.4).clamp(340.0, 580.0).round();
    let code_lines = lines.len().min(28) as f64;
    let height = (56.0 + code_lines * 17.5 + 18.0).max(130.0).round();
    (width, height, lines.len())
}

fn pack_boxes(
    items: &mut [LayoutBox],
    gap: f64,
    pad_x: f64,
    pad_top: f64,
    pad_bottom: f64,
    min_w: f64,
    min_h: f64,
) -> (f64, f64) {
    if items.is_empty() {
        return (min_w, min_h);
    }

    let total_area: f64 = items
        .iter()
        .map(|item| (item.width + gap) * (item.height + gap))
        .sum();
    let max_item_w: f64 = items.iter().map(|i| i.width).fold(0.0, f64::max);
    let target_row_width = max_item_w.max(total_area.sqrt() * 1.35);

    let mut cursor_x = pad_x;
    let mut cursor_y = pad_top;
    let mut row_height: f64 = 0.0;
    let mut max_used_width: f64 = 0.0;

    for item in items.iter_mut() {
        if cursor_x > pad_x && cursor_x - pad_x + item.width > target_row_width {
            cursor_x = pad_x;
            cursor_y += row_height + gap;
            row_height = 0.0;
        }

        item.rel_x = cursor_x;
        item.rel_y = cursor_y;

        cursor_x += item.width + gap;
        row_height = row_height.max(item.height);
        max_used_width = max_used_width.max(cursor_x - gap + pad_x);
    }

    let total_height = cursor_y + row_height + pad_bottom;
    (
        max_used_width.round().max(min_w),
        total_height.round().max(min_h),
    )
}

fn anchor_points(a: &Rect, b: &Rect) -> (f64, f64, f64, f64) {
    let ax = a.x + a.width / 2.0;
    let ay = a.y + a.height / 2.0;
    let bx = b.x + b.width / 2.0;
    let by = b.y + b.height / 2.0;
    let dx = bx - ax;
    let dy = by - ay;

    if dx.abs() >= dy.abs() {
        if dx >= 0.0 {
            (a.x + a.width, ay, b.x, by)
        } else {
            (a.x, ay, b.x + b.width, by)
        }
    } else if dy >= 0.0 {
        (ax, a.y + a.height, bx, b.y)
    } else {
        (ax, a.y, bx, b.y + b.height)
    }
}

pub fn compute_layout(graph: &ProjectGraph) -> LayoutResult {
    let mut group_by_id: HashMap<&str, &GroupNode> = HashMap::new();
    let mut child_group_ids: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut child_fn_map: HashMap<&str, Vec<&FunctionNode>> = HashMap::new();
    let mut root_group_ids: Vec<&str> = Vec::new();

    for g in &graph.groups {
        group_by_id.insert(g.id.as_str(), g);
        if let Some(pid) = &g.parent_id {
            child_group_ids
                .entry(pid.as_str())
                .or_default()
                .push(g.id.as_str());
        } else {
            root_group_ids.push(g.id.as_str());
        }
    }

    for f in &graph.functions {
        child_fn_map
            .entry(f.parent_id.as_str())
            .or_default()
            .push(f);
    }

    let fn_by_id: HashMap<&str, &FunctionNode> =
        graph.functions.iter().map(|f| (f.id.as_str(), f)).collect();

    fn build_box<'a>(
        group_id: &'a str,
        group_by_id: &HashMap<&'a str, &'a GroupNode>,
        child_group_ids: &HashMap<&'a str, Vec<&'a str>>,
        child_fn_map: &HashMap<&'a str, Vec<&'a FunctionNode>>,
    ) -> LayoutBox {
        let group = group_by_id[group_id];
        let mut child_boxes: Vec<LayoutBox> = Vec::new();

        if let Some(sub_ids) = child_group_ids.get(group_id) {
            for &sub_id in sub_ids {
                child_boxes.push(build_box(
                    sub_id,
                    group_by_id,
                    child_group_ids,
                    child_fn_map,
                ));
            }
        }

        if let Some(sub_fns) = child_fn_map.get(group_id) {
            for &fn_node in sub_fns {
                let (width, height, _) = measure_function_box(fn_node);
                child_boxes.push(LayoutBox {
                    id: fn_node.id.clone(),
                    kind: BoxKind::Function,
                    width,
                    height,
                    rel_x: 0.0,
                    rel_y: 0.0,
                    children: Vec::new(),
                    function_count: 1,
                });
            }
        }

        let style = group_style(&group.kind);
        let (width, height) = pack_boxes(
            &mut child_boxes,
            style.gap,
            style.pad_x,
            style.pad_top,
            style.pad_bottom,
            style.min_w,
            style.min_h,
        );
        let function_count = child_boxes.iter().map(|c| c.function_count).sum();

        LayoutBox {
            id: group.id.clone(),
            kind: BoxKind::Group,
            width,
            height,
            rel_x: 0.0,
            rel_y: 0.0,
            children: child_boxes,
            function_count,
        }
    }

    let mut root_boxes: Vec<LayoutBox> = root_group_ids
        .iter()
        .map(|&id| build_box(id, &group_by_id, &child_group_ids, &child_fn_map))
        .collect();
    let (bound_w, bound_h) = pack_boxes(&mut root_boxes, 48.0, 40.0, 40.0, 40.0, 400.0, 300.0);

    let mut positioned_groups = Vec::new();
    let mut positioned_functions = Vec::new();
    let mut node_rects: HashMap<String, Rect> = HashMap::new();

    fn flatten(
        box_node: &LayoutBox,
        offset_x: f64,
        offset_y: f64,
        depth: usize,
        group_by_id: &HashMap<&str, &GroupNode>,
        fn_by_id: &HashMap<&str, &FunctionNode>,
        positioned_groups: &mut Vec<PositionedGroup>,
        positioned_functions: &mut Vec<PositionedFunction>,
        node_rects: &mut HashMap<String, Rect>,
    ) {
        let abs_x = offset_x + box_node.rel_x;
        let abs_y = offset_y + box_node.rel_y;
        let rect = Rect {
            x: abs_x,
            y: abs_y,
            width: box_node.width,
            height: box_node.height,
        };
        node_rects.insert(box_node.id.clone(), rect);

        match box_node.kind {
            BoxKind::Function => {
                let fn_node = fn_by_id[box_node.id.as_str()];
                let line_count = fn_node.code.split('\n').count();
                positioned_functions.push(PositionedFunction {
                    node: (*fn_node).clone(),
                    rect,
                    line_count,
                });
            }
            BoxKind::Group => {
                let group = group_by_id[box_node.id.as_str()];
                positioned_groups.push(PositionedGroup {
                    node: (*group).clone(),
                    rect,
                    depth,
                    function_count: box_node.function_count,
                });
                for child in &box_node.children {
                    flatten(
                        child,
                        abs_x,
                        abs_y,
                        depth + 1,
                        group_by_id,
                        fn_by_id,
                        positioned_groups,
                        positioned_functions,
                        node_rects,
                    );
                }
            }
        }
    }

    for rb in &root_boxes {
        flatten(
            rb,
            0.0,
            0.0,
            0,
            &group_by_id,
            &fn_by_id,
            &mut positioned_groups,
            &mut positioned_functions,
            &mut node_rects,
        );
    }

    let mut positioned_edges = Vec::new();
    for edge in &graph.edges {
        let Some(src_rect) = node_rects.get(&edge.source_id) else {
            continue;
        };
        let Some(dst_rect) = node_rects.get(&edge.target_id) else {
            continue;
        };
        let (x1, y1, x2, y2) = anchor_points(src_rect, dst_rect);
        positioned_edges.push(PositionedEdge {
            edge: edge.clone(),
            x1,
            y1,
            x2,
            y2,
        });
    }

    LayoutResult {
        groups: positioned_groups,
        functions: positioned_functions,
        edges: positioned_edges,
        node_rects,
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            width: bound_w,
            height: bound_h,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::compute_layout;
    use crate::types::{
        EdgeKind, FunctionNode, GraphEdge, GroupKind, GroupNode, ProjectGraph, ProjectStats,
    };

    #[test]
    fn computes_nested_boxes_and_edges() {
        let graph = ProjectGraph {
            root_name: "demo".to_string(),
            root_path: "/tmp/demo".to_string(),
            groups: vec![
                GroupNode {
                    id: "dir:src".to_string(),
                    kind: GroupKind::Directory,
                    name: "src".to_string(),
                    qualified_name: "src".to_string(),
                    parent_id: None,
                    file_path: None,
                    is_package: true,
                    docstring: None,
                    line_count: 0,
                },
                GroupNode {
                    id: "mod:src.app".to_string(),
                    kind: GroupKind::Module,
                    name: "app".to_string(),
                    qualified_name: "src.app".to_string(),
                    parent_id: Some("dir:src".to_string()),
                    file_path: Some("/tmp/demo/src/app.py".to_string()),
                    is_package: false,
                    docstring: None,
                    line_count: 10,
                },
            ],
            functions: vec![
                FunctionNode {
                    id: "fn:a".to_string(),
                    name: "alpha".to_string(),
                    qualified_name: "src.app.alpha".to_string(),
                    parent_id: "mod:src.app".to_string(),
                    file_path: "/tmp/demo/src/app.py".to_string(),
                    is_async: false,
                    is_method: false,
                    decorators: vec![],
                    signature: "() -> None".to_string(),
                    docstring: None,
                    start_line: 1,
                    end_line: 2,
                    code: "def alpha():\n    beta()".to_string(),
                },
                FunctionNode {
                    id: "fn:b".to_string(),
                    name: "beta".to_string(),
                    qualified_name: "src.app.beta".to_string(),
                    parent_id: "mod:src.app".to_string(),
                    file_path: "/tmp/demo/src/app.py".to_string(),
                    is_async: false,
                    is_method: false,
                    decorators: vec![],
                    signature: "() -> None".to_string(),
                    docstring: None,
                    start_line: 4,
                    end_line: 5,
                    code: "def beta():\n    return 1".to_string(),
                },
            ],
            edges: vec![GraphEdge {
                id: "edge:1".to_string(),
                kind: EdgeKind::Call,
                source_id: "fn:a".to_string(),
                target_id: "fn:b".to_string(),
                label: None,
            }],
            parse_errors: vec![],
            stats: ProjectStats::default(),
        };

        let layout = compute_layout(&graph);
        assert_eq!(layout.groups.len(), 2);
        assert_eq!(layout.functions.len(), 2);
        assert_eq!(layout.edges.len(), 1);
        assert!(layout.bounds.width >= 400.0);
        assert!(layout.bounds.height >= 300.0);
        let dir = layout.groups.iter().find(|g| g.node.id == "dir:src").unwrap();
        let mod_g = layout
            .groups
            .iter()
            .find(|g| g.node.id == "mod:src.app")
            .unwrap();
        assert!(mod_g.rect.x > dir.rect.x);
        assert!(mod_g.rect.y > dir.rect.y);
        assert_eq!(dir.function_count, 2);
    }

    #[test]
    fn computes_large_project_layout_5k_files_40k_functions() {
        let num_dirs = 200usize;
        let num_files = 5000usize;
        let num_classes = 6000usize;
        let num_functions = 40000usize;

        let mut groups = Vec::with_capacity(1 + num_dirs + num_files + num_classes);
        groups.push(GroupNode {
            id: "dir:.".to_string(),
            kind: GroupKind::Directory,
            name: "root".to_string(),
            qualified_name: "root".to_string(),
            parent_id: None,
            file_path: None,
            is_package: true,
            docstring: None,
            line_count: 0,
        });
        for d in 0..num_dirs {
            groups.push(GroupNode {
                id: format!("dir:d{d}"),
                kind: GroupKind::Directory,
                name: format!("d{d}"),
                qualified_name: format!("d{d}"),
                parent_id: Some("dir:.".to_string()),
                file_path: None,
                is_package: true,
                docstring: None,
                line_count: 0,
            });
        }
        for f in 0..num_files {
            let d = f % num_dirs;
            groups.push(GroupNode {
                id: format!("mod:m{f}"),
                kind: GroupKind::Module,
                name: format!("m{f}.py"),
                qualified_name: format!("d{d}.m{f}"),
                parent_id: Some(format!("dir:d{d}")),
                file_path: Some(format!("/tmp/big/d{d}/m{f}.py")),
                is_package: false,
                docstring: None,
                line_count: 50,
            });
        }
        for c in 0..num_classes {
            let f = c % num_files;
            groups.push(GroupNode {
                id: format!("cls:c{c}"),
                kind: GroupKind::Class,
                name: format!("Class{c}"),
                qualified_name: format!("Class{c}"),
                parent_id: Some(format!("mod:m{f}")),
                file_path: None,
                is_package: false,
                docstring: None,
                line_count: 20,
            });
        }

        let mut functions = Vec::with_capacity(num_functions);
        for i in 0..num_functions {
            let c = i % num_classes;
            let f = c % num_files;
            functions.push(FunctionNode {
                id: format!("fn:f{i}"),
                name: format!("fn_{i}"),
                qualified_name: format!("Class{c}.fn_{i}"),
                parent_id: format!("cls:c{c}"),
                file_path: format!("/tmp/big/d{}/m{f}.py", f % num_dirs),
                is_async: false,
                is_method: true,
                decorators: vec![],
                signature: "(self, x: int) -> int".to_string(),
                docstring: None,
                start_line: 1,
                end_line: 4,
                code: "def fn(self, x):\n    return x + 1".to_string(),
            });
        }

        let mut edges = Vec::with_capacity(num_functions * 2);
        for i in 0..num_functions {
            edges.push(GraphEdge {
                id: format!("e:c{i}"),
                kind: EdgeKind::Call,
                source_id: format!("fn:f{i}"),
                target_id: format!("fn:f{}", (i + 1) % num_functions),
                label: None,
            });
            edges.push(GraphEdge {
                id: format!("e:c2_{i}"),
                kind: EdgeKind::Call,
                source_id: format!("fn:f{i}"),
                target_id: format!("fn:f{}", (i + 7) % num_functions),
                label: None,
            });
        }

        let graph = ProjectGraph {
            root_name: "big".to_string(),
            root_path: "/tmp/big".to_string(),
            groups,
            functions,
            edges,
            parse_errors: vec![],
            stats: ProjectStats::default(),
        };

        let layout = compute_layout(&graph);
        assert_eq!(layout.groups.len(), 1 + num_dirs + num_files + num_classes);
        assert_eq!(layout.functions.len(), num_functions);
        assert_eq!(layout.edges.len(), num_functions * 2);
        let fit_scale = (1000.0 / layout.bounds.width).min(720.0 / layout.bounds.height);
        assert!(fit_scale > 0.001, "fit_scale was {fit_scale}");
    }
}
