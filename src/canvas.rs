use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlElement, MouseEvent, WheelEvent};

use crate::highlight::tokenize_code_line;
use crate::types::{
    EdgeKind, GroupKind, LayoutResult, LodLevel, PositionedFunction, PositionedGroup, Rect,
    Viewport,
};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ReviewOverlay {
    pub active: bool,
    pub changed_only: bool,
    pub changed_ids: HashSet<String>,
    pub diff_stats: HashMap<String, (usize, usize)>,
}

#[derive(Clone)]
enum ActionKind {
    OpenEditor(String),
}

#[derive(Clone)]
struct ActionHit {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    kind: ActionKind,
}

#[derive(Clone, Copy)]
struct DragState {
    dragging: bool,
    moved: bool,
    start_x: f64,
    start_y: f64,
    orig_vx: f64,
    orig_vy: f64,
}

impl Default for DragState {
    fn default() -> Self {
        Self {
            dragging: false,
            moved: false,
            start_x: 0.0,
            start_y: 0.0,
            orig_vx: 0.0,
            orig_vy: 0.0,
        }
    }
}

fn round_rect(ctx: &CanvasRenderingContext2d, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    ctx.begin_path();
    ctx.move_to(x + r, y);
    ctx.line_to(x + w - r, y);
    let _ = ctx.quadratic_curve_to(x + w, y, x + w, y + r);
    ctx.line_to(x + w, y + h - r);
    let _ = ctx.quadratic_curve_to(x + w, y + h, x + w - r, y + h);
    ctx.line_to(x + r, y + h);
    let _ = ctx.quadratic_curve_to(x, y + h, x, y + h - r);
    ctx.line_to(x, y + r);
    let _ = ctx.quadratic_curve_to(x, y, x + r, y);
    ctx.close_path();
}

fn rects_intersect(a: &Rect, x: f64, y: f64, w: f64, h: f64) -> bool {
    a.x + a.width >= x && a.x <= x + w && a.y + a.height >= y && a.y <= y + h
}

fn truncate_to_width(ctx: &CanvasRenderingContext2d, text: &str, max_w: f64) -> String {
    if max_w <= 12.0 {
        return String::new();
    }
    if let Ok(m) = ctx.measure_text(text) {
        if m.width() <= max_w {
            return text.to_string();
        }
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if let Ok(m) = ctx.measure_text(&candidate) {
            if m.width() <= max_w {
                return candidate;
            }
        }
    }
    "…".to_string()
}

fn minimap_geometry(css_w: f64, css_h: f64, bounds: Rect) -> Option<(f64, f64, f64, f64, f64, f64, f64)> {
    if css_w <= 420.0 || css_h <= 300.0 || bounds.width <= 0.0 || bounds.height <= 0.0 {
        return None;
    }
    let mm_w = 180.0;
    let mm_h = 120.0;
    let mm_x = css_w - mm_w - 14.0;
    let mm_y = 48.0;
    let pad = 8.0;
    let mm_scale = ((mm_w - pad * 2.0) / bounds.width).min((mm_h - pad * 2.0) / bounds.height);
    let off_x = mm_x + (mm_w - bounds.width * mm_scale) / 2.0;
    let off_y = mm_y + (mm_h - bounds.height * mm_scale) / 2.0;
    Some((mm_x, mm_y, mm_w, mm_h, mm_scale, off_x, off_y))
}

fn fmt_edge_count(n: usize) -> String {
    if n > 99 {
        "99+".to_string()
    } else {
        n.to_string()
    }
}

fn render_canvas(
    canvas: &HtmlCanvasElement,
    layout: &LayoutResult,
    viewport: Viewport,
    selected_id: Option<&str>,
    hovered_id: Option<&str>,
    group_meta: &HashMap<String, String>,
    fn_counts: &HashMap<String, (usize, usize)>,
    edges_by_node: &HashMap<String, Vec<usize>>,
    review: &ReviewOverlay,
    action_hits: &mut Vec<ActionHit>,
) {
    action_hits.clear();
    let Some(ctx_obj) = canvas.get_context("2d").ok().flatten() else {
        return;
    };
    let Ok(ctx) = ctx_obj.dyn_into::<CanvasRenderingContext2d>() else {
        return;
    };

    let dpr = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0)
        .max(1.0);
    let css_w = (canvas.width() as f64) / dpr;
    let css_h = (canvas.height() as f64) / dpr;
    if css_w <= 0.0 || css_h <= 0.0 {
        return;
    }

    ctx.save();
    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);

    // Background
    ctx.set_fill_style_str("#0b1120");
    ctx.fill_rect(0.0, 0.0, css_w, css_h);

    let scale = viewport.scale;
    let lod = LodLevel::from_scale(scale);

    // Subtle dot grid in screen space
    if scale > 0.22 {
        let grid = (32.0 * scale).max(14.0);
        let ox = viewport.x.rem_euclid(grid);
        let oy = viewport.y.rem_euclid(grid);
        ctx.set_fill_style_str("rgba(148, 163, 184, 0.14)");
        let mut gx = ox;
        while gx < css_w {
            let mut gy = oy;
            while gy < css_h {
                ctx.fill_rect(gx, gy, 1.2, 1.2);
                gy += grid;
            }
            gx += grid;
        }
    }

    let world_left = -viewport.x / scale;
    let world_top = -viewport.y / scale;
    let world_w = css_w / scale;
    let world_h = css_h / scale;

    // World transform
    ctx.save();
    let _ = ctx.translate(viewport.x, viewport.y);
    let _ = ctx.scale(scale, scale);

    // 1. Draw groups in depth order
    for g in &layout.groups {
        if g.rect.width * scale < 3.0 && g.rect.height * scale < 3.0 {
            continue;
        }
        if !rects_intersect(&g.rect, world_left, world_top, world_w, world_h) {
            continue;
        }
        let is_selected = selected_id == Some(g.node.id.as_str());
        let is_hovered = hovered_id == Some(g.node.id.as_str());
        let is_changed = review.active && review.changed_ids.contains(&g.node.id);
        let dimmed = review.active && review.changed_only && !is_changed;

        let (mut fill, mut stroke, radius) = match g.node.kind {
            GroupKind::Directory => (
                "rgba(15, 23, 42, 0.55)",
                if is_selected {
                    "#38bdf8"
                } else if is_hovered {
                    "#64748b"
                } else {
                    "rgba(71, 85, 105, 0.55)"
                },
                14.0,
            ),
            GroupKind::Module => (
                "rgba(30, 41, 59, 0.72)",
                if is_selected {
                    "#38bdf8"
                } else if is_hovered {
                    "#818cf8"
                } else {
                    "rgba(99, 102, 241, 0.35)"
                },
                11.0,
            ),
            GroupKind::Class => (
                "rgba(30, 27, 75, 0.55)",
                if is_selected {
                    "#f472b6"
                } else if is_hovered {
                    "#c084fc"
                } else {
                    "rgba(192, 132, 252, 0.35)"
                },
                9.0,
            ),
        };
        if is_changed {
            if is_selected {
                stroke = "#fb923c";
                fill = "rgba(245, 158, 11, 0.16)";
            } else if !is_hovered {
                stroke = "#f59e0b";
            }
        }

        ctx.save();
        if dimmed {
            ctx.set_global_alpha(0.35);
        }
        round_rect(&ctx, g.rect.x, g.rect.y, g.rect.width, g.rect.height, radius);
        ctx.set_fill_style_str(fill);
        ctx.fill();
        ctx.set_line_width(if is_selected || is_hovered || is_changed {
            2.2 / scale.sqrt()
        } else {
            1.2 / scale.sqrt()
        });
        ctx.set_stroke_style_str(stroke);
        ctx.stroke();
        ctx.restore();
    }

    // 2. Draw functions in world space
    for fn_item in &layout.functions {
        let sw = fn_item.rect.width * scale;
        let sh = fn_item.rect.height * scale;
        if sw < 3.0 || sh < 3.0 {
            continue;
        }
        if !rects_intersect(&fn_item.rect, world_left, world_top, world_w, world_h) {
            continue;
        }
        let is_selected = selected_id == Some(fn_item.node.id.as_str());
        let is_hovered = hovered_id == Some(fn_item.node.id.as_str());
        let is_changed = review.active && review.changed_ids.contains(&fn_item.node.id);

        if lod != LodLevel::Micro && sw < 16.0 && !is_selected && !is_hovered {
            ctx.set_fill_style_str(if is_changed {
                "#f59e0b"
            } else {
                "#1e293b"
            });
            ctx.fill_rect(
                fn_item.rect.x,
                fn_item.rect.y,
                fn_item.rect.width,
                fn_item.rect.height,
            );
            continue;
        }
        let dimmed = review.active && review.changed_only && !is_changed;

        ctx.save();
        if dimmed {
            ctx.set_global_alpha(0.28);
        }
        round_rect(
            &ctx,
            fn_item.rect.x,
            fn_item.rect.y,
            fn_item.rect.width,
            fn_item.rect.height,
            8.0,
        );
        ctx.set_fill_style_str(if is_selected && is_changed {
            "#2a1b0e"
        } else if is_selected {
            "#16233b"
        } else if is_hovered {
            "#141f35"
        } else {
            "#0f172a"
        });
        ctx.fill();
        ctx.set_line_width(if is_selected || is_changed {
            2.4
        } else if is_hovered {
            1.8
        } else {
            1.1
        });
        ctx.set_stroke_style_str(if is_selected && is_changed {
            "#fb923c"
        } else if is_selected {
            "#38bdf8"
        } else if is_changed {
            "#f59e0b"
        } else if is_hovered {
            "#60a5fa"
        } else {
            "#334155"
        });
        ctx.stroke();

        if lod == LodLevel::Micro {
            // In-card header
            let hx = fn_item.rect.x;
            let hy = fn_item.rect.y;
            let hw = fn_item.rect.width;

            ctx.set_fill_style_str("rgba(30, 41, 59, 0.85)");
            ctx.fill_rect(hx + 1.0, hy + 1.0, hw - 2.0, 34.0);

            // Line & caller/callee metadata badge
            let (c_in, c_out) = fn_counts.get(&fn_item.node.id).copied().unwrap_or((0, 0));
            let line_badge = format!(
                "{}L · L{}-{} · {} in · {} out",
                fn_item.line_count,
                fn_item.node.start_line,
                fn_item.node.end_line,
                fmt_edge_count(c_in),
                fmt_edge_count(c_out)
            );
            ctx.set_font("500 10.5px 'Inter', system-ui, sans-serif");
            let badge_w = ctx
                .measure_text(&line_badge)
                .map(|m| m.width())
                .unwrap_or(110.0);

            ctx.set_font("600 12.5px 'JetBrains Mono', ui-monospace, monospace");
            ctx.set_fill_style_str(if fn_item.node.is_async { "#c084fc" } else { "#38bdf8" });
            let prefix = if fn_item.node.is_async { "async " } else { "" };
            let title = format!("{prefix}{}", fn_item.node.name);
            let clipped_title = truncate_to_width(&ctx, &title, (hw - badge_w - 28.0).max(60.0));
            let _ = ctx.fill_text(&clipped_title, hx + 12.0, hy + 21.0);

            ctx.set_font("500 10.5px 'Inter', system-ui, sans-serif");
            ctx.set_fill_style_str("#94a3b8");
            let _ = ctx.fill_text(&line_badge, hx + hw - badge_w - 10.0, hy + 21.0);

            // Signature
            ctx.set_font("400 11px 'JetBrains Mono', ui-monospace, monospace");
            ctx.set_fill_style_str("#94a3b8");
            let sig = truncate_to_width(&ctx, &fn_item.node.signature, hw - 24.0);
            let _ = ctx.fill_text(&sig, hx + 12.0, hy + 48.0);

            // Code lines (skip if selected because DOM overlay renders scrollable code)
            if !is_selected {
                ctx.set_font("400 11.5px 'JetBrains Mono', ui-monospace, monospace");
                let max_lines = 28usize;
                for (idx, raw_line) in fn_item.node.code.lines().take(max_lines).enumerate() {
                    let ly = hy + 68.0 + (idx as f64) * 17.5;
                    if ly > hy + fn_item.rect.height - 8.0 {
                        break;
                    }
                    let line_num = fn_item.node.start_line + idx;
                    ctx.set_fill_style_str("#475569");
                    let _ = ctx.fill_text(&format!("{line_num:>3}"), hx + 10.0, ly);

                    let mut tx = hx + 40.0;
                    let max_x = hx + hw - 10.0;
                    for tok in tokenize_code_line(raw_line, Some(&fn_item.node.file_path)) {
                        if tx >= max_x {
                            break;
                        }
                        ctx.set_fill_style_str(tok.kind.color());
                        let clipped = truncate_to_width(&ctx, &tok.text, max_x - tx);
                        if !clipped.is_empty() {
                            let _ = ctx.fill_text(&clipped, tx, ly);
                            if let Ok(m) = ctx.measure_text(&clipped) {
                                tx += m.width();
                            }
                        }
                    }
                }
            }
        }
        ctx.restore();
    }

    // Restore world transform -> back to screen space for crisp title pills & action icons
    ctx.restore();

    // 4. Screen-space Group Title Pills + Icon Buttons
    let mut placed_pills: Vec<(f64, f64, f64, f64)> = Vec::new();
    let overlaps_placed = |x: f64, y: f64, w: f64, h: f64, placed: &[(f64, f64, f64, f64)]| {
        placed.iter().any(|&(px, py, pw, ph)| {
            !(x + w + 4.0 <= px || x >= px + pw + 4.0 || y + h + 2.0 <= py || y >= py + ph + 2.0)
        })
    };

    for g in &layout.groups {
        if g.rect.width * scale < 36.0 || g.rect.height * scale < 20.0 {
            continue;
        }
        if !rects_intersect(&g.rect, world_left, world_top, world_w, world_h) {
            continue;
        }
        let show_pill = match lod {
            LodLevel::Macro => g.node.kind == GroupKind::Directory,
            LodLevel::Meso | LodLevel::Micro => true,
        };
        if !show_pill {
            continue;
        }

        let sx = g.rect.x * scale + viewport.x + 10.0;
        let sy = g.rect.y * scale + viewport.y + 8.0;
        let max_pill_w = (g.rect.width * scale - 20.0).max(80.0);

        let (accent, bg) = match g.node.kind {
            GroupKind::Directory => ("#38bdf8", "rgba(15, 23, 42, 0.92)"),
            GroupKind::Module => ("#818cf8", "rgba(30, 41, 59, 0.94)"),
            GroupKind::Class => ("#c084fc", "rgba(46, 16, 101, 0.92)"),
        };

        ctx.set_font("600 11.5px 'Inter', system-ui, sans-serif");
        let icon_pad = 19.0;
        let meta = group_meta.get(&g.node.id).map(String::as_str).unwrap_or("");
        let btn_reserve = if g.node.kind == GroupKind::Module {
            28.0
        } else {
            8.0
        };
        let avail_text_w = (max_pill_w - btn_reserve - icon_pad).max(40.0);
        let clipped_name = truncate_to_width(&ctx, &g.node.name, avail_text_w);
        let name_w = ctx
            .measure_text(&clipped_name)
            .map(|m| m.width())
            .unwrap_or(60.0);

        let meta_str = if meta.is_empty() {
            String::new()
        } else {
            format!(" · {meta}")
        };
        ctx.set_font("500 10.5px 'Inter', system-ui, sans-serif");
        let clipped_meta = if !meta_str.is_empty() && avail_text_w - name_w > 34.0 {
            truncate_to_width(&ctx, &meta_str, avail_text_w - name_w)
        } else {
            String::new()
        };
        let meta_w = if clipped_meta.is_empty() {
            0.0
        } else {
            ctx.measure_text(&clipped_meta)
                .map(|m| m.width())
                .unwrap_or(0.0)
        };

        let pill_w = (name_w + meta_w + 18.0 + icon_pad).min(max_pill_w);
        let pill_h = 24.0;
        let has_editor_btn = g.node.kind == GroupKind::Module && g.node.file_path.is_some();
        let total_w = pill_w + if has_editor_btn { 28.0 } else { 0.0 };
        if overlaps_placed(sx, sy, total_w, pill_h, &placed_pills) {
            continue;
        }
        placed_pills.push((sx, sy, total_w, pill_h));

        round_rect(&ctx, sx, sy, pill_w, pill_h, 6.0);
        ctx.set_fill_style_str(bg);
        ctx.fill();
        ctx.set_line_width(1.0);
        ctx.set_stroke_style_str(accent);
        ctx.stroke();

        draw_group_icon(&ctx, &g.node.kind, sx + 8.0, sy + 6.0, 12.0, accent);

        ctx.set_font("600 11.5px 'Inter', system-ui, sans-serif");
        ctx.set_fill_style_str("#f1f5f9");
        let _ = ctx.fill_text(&clipped_name, sx + 9.0 + icon_pad, sy + 16.0);
        if !clipped_meta.is_empty() {
            ctx.set_font("500 10.5px 'Inter', system-ui, sans-serif");
            ctx.set_fill_style_str("#94a3b8");
            let _ = ctx.fill_text(&clipped_meta, sx + 9.0 + icon_pad + name_w, sy + 16.0);
        }

        if g.node.kind == GroupKind::Module {
            if let Some(fp) = &g.node.file_path {
                let bx = sx + pill_w + 6.0;
                let btn_size = 22.0;
                draw_icon_button(
                    &ctx,
                    bx,
                    sy + 1.0,
                    btn_size,
                    ButtonIcon::OpenEditor,
                    true,
                    "#94a3b8",
                );
                action_hits.push(ActionHit {
                    x: bx,
                    y: sy + 1.0,
                    w: btn_size,
                    h: btn_size,
                    kind: ActionKind::OpenEditor(fp.clone()),
                });
            }
        }
    }

    // 5. Screen-space Function Title Pills in Meso LOD
    if lod == LodLevel::Meso {
        ctx.set_font("600 11px 'JetBrains Mono', ui-monospace, monospace");
        for fn_item in &layout.functions {
            if fn_item.rect.width * scale < 32.0 || fn_item.rect.height * scale < 16.0 {
                continue;
            }
            if !rects_intersect(&fn_item.rect, world_left, world_top, world_w, world_h) {
                continue;
            }
            let (c_in, c_out) = fn_counts.get(&fn_item.node.id).copied().unwrap_or((0, 0));
            let sx = fn_item.rect.x * scale + viewport.x + 6.0;
            let sy = fn_item.rect.y * scale + viewport.y + 6.0;
            let max_w = (fn_item.rect.width * scale - 12.0).max(50.0);
            let full_label = format!(
                "{} · {}L · {} in · {} out",
                fn_item.node.name,
                fn_item.line_count,
                fmt_edge_count(c_in),
                fmt_edge_count(c_out)
            );
            let label = truncate_to_width(&ctx, &full_label, max_w - 14.0);
            let tw = ctx.measure_text(&label).map(|m| m.width()).unwrap_or(50.0);
            let pw = (tw + 14.0).min(max_w);
            if overlaps_placed(sx, sy, pw, 20.0, &placed_pills) {
                continue;
            }
            placed_pills.push((sx, sy, pw, 20.0));
            round_rect(&ctx, sx, sy, pw, 20.0, 5.0);
            ctx.set_fill_style_str("rgba(15, 23, 42, 0.92)");
            ctx.fill();
            ctx.set_stroke_style_str("#38bdf8");
            ctx.set_line_width(1.0);
            ctx.stroke();
            ctx.set_fill_style_str("#e2e8f0");
            let _ = ctx.fill_text(&label, sx + 7.0, sy + 14.0);
        }
    }

    // 6. Screen-space Caller / Callee Count Badge on Hovered or Selected Node
    if let Some(target_id) = hovered_id.or(selected_id) {
        if let Some(rect) = layout.node_rects.get(target_id) {
            let mut callers = HashSet::new();
            let mut callees = HashSet::new();
            if let Some(indices) = edges_by_node.get(target_id) {
                for &idx in indices {
                    let e = &layout.edges[idx];
                    if e.edge.target_id == target_id {
                        callers.insert(e.edge.source_id.as_str());
                    }
                    if e.edge.source_id == target_id {
                        callees.insert(e.edge.target_id.as_str());
                    }
                }
            }
            let is_fn = layout.functions.iter().any(|f| f.node.id == target_id);
            if is_fn || !callers.is_empty() || !callees.is_empty() {
                let c_in = callers.len();
                let c_out = callees.len();
                let in_lbl = if c_in == 1 { "caller" } else { "callers" };
                let out_lbl = if c_out == 1 { "callee" } else { "callees" };
                let badge = format!(
                    "{} {in_lbl} · {} {out_lbl}",
                    fmt_edge_count(c_in),
                    fmt_edge_count(c_out)
                );

                ctx.set_font("600 11px 'Inter', system-ui, sans-serif");
                let tw = ctx.measure_text(&badge).map(|m| m.width()).unwrap_or(90.0);
                let pw = tw + 18.0;
                let ph = 22.0;
                let bx = (rect.x + rect.width) * scale + viewport.x - pw;
                let by = (rect.y * scale + viewport.y - ph - 6.0).max(8.0);

                round_rect(&ctx, bx, by, pw, ph, 999.0);
                ctx.set_fill_style_str("rgba(15, 23, 42, 0.96)");
                ctx.fill();
                ctx.set_line_width(1.2);
                ctx.set_stroke_style_str("#fbbf24");
                ctx.stroke();

                ctx.set_fill_style_str("#fef3c7");
                let _ = ctx.fill_text(&badge, bx + 9.0, by + 15.0);
            }
        }
    }

    // 7. Top-right Overview Minimap
    if let Some((mm_x, mm_y, mm_w, mm_h, mm_scale, off_x, off_y)) =
        minimap_geometry(css_w, css_h, layout.bounds)
    {
        ctx.save();
        round_rect(&ctx, mm_x, mm_y, mm_w, mm_h, 8.0);
        ctx.set_fill_style_str("rgba(15, 23, 42, 0.90)");
        ctx.fill();
        ctx.set_line_width(1.0);
        ctx.set_stroke_style_str("rgba(148, 163, 184, 0.32)");
        ctx.stroke();

        ctx.save();
        round_rect(&ctx, mm_x + 1.0, mm_y + 1.0, mm_w - 2.0, mm_h - 2.0, 7.0);
        ctx.clip();

        for g in &layout.groups {
            if g.node.kind == GroupKind::Class {
                continue;
            }
            let mw = g.rect.width * mm_scale;
            let mh = g.rect.height * mm_scale;
            if mw < 1.5 && mh < 1.5 {
                continue;
            }
            let mx = off_x + (g.rect.x - layout.bounds.x) * mm_scale;
            let my = off_y + (g.rect.y - layout.bounds.y) * mm_scale;
            let is_changed = review.active && review.changed_ids.contains(&g.node.id);
            ctx.set_fill_style_str(if is_changed {
                "rgba(245, 158, 11, 0.78)"
            } else if g.node.kind == GroupKind::Directory {
                "rgba(71, 85, 105, 0.35)"
            } else {
                "rgba(129, 140, 248, 0.45)"
            });
            ctx.fill_rect(mx, my, mw.max(1.5), mh.max(1.5));
        }

        // Current viewport indicator
        let vx = off_x + (world_left - layout.bounds.x) * mm_scale;
        let vy = off_y + (world_top - layout.bounds.y) * mm_scale;
        let vw = (world_w * mm_scale).max(6.0);
        let vh = (world_h * mm_scale).max(6.0);
        ctx.set_fill_style_str("rgba(56, 189, 248, 0.16)");
        ctx.fill_rect(vx, vy, vw, vh);
        ctx.set_line_width(1.5);
        ctx.set_stroke_style_str("#38bdf8");
        ctx.stroke_rect(vx, vy, vw, vh);

        ctx.restore();
        ctx.restore();
    }

    ctx.restore();
}

#[derive(Clone, Copy)]
enum ButtonIcon {
    OpenEditor,
}

fn draw_group_icon(
    ctx: &CanvasRenderingContext2d,
    kind: &GroupKind,
    x: f64,
    y: f64,
    s: f64,
    color: &str,
) {
    ctx.save();
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.35);
    ctx.set_line_join("round");
    ctx.set_line_cap("round");
    ctx.begin_path();
    match kind {
        GroupKind::Directory => {
            ctx.move_to(x + s * 0.08, y + s * 0.25);
            ctx.line_to(x + s * 0.42, y + s * 0.25);
            ctx.line_to(x + s * 0.54, y + s * 0.38);
            ctx.line_to(x + s * 0.92, y + s * 0.38);
            ctx.line_to(x + s * 0.92, y + s * 0.85);
            ctx.line_to(x + s * 0.08, y + s * 0.85);
            ctx.close_path();
        }
        GroupKind::Module => {
            ctx.move_to(x + s * 0.18, y + s * 0.10);
            ctx.line_to(x + s * 0.62, y + s * 0.10);
            ctx.line_to(x + s * 0.84, y + s * 0.32);
            ctx.line_to(x + s * 0.84, y + s * 0.90);
            ctx.line_to(x + s * 0.18, y + s * 0.90);
            ctx.close_path();
            ctx.move_to(x + s * 0.62, y + s * 0.10);
            ctx.line_to(x + s * 0.62, y + s * 0.32);
            ctx.line_to(x + s * 0.84, y + s * 0.32);
        }
        GroupKind::Class => {
            let cx = x + s * 0.5;
            let cy = y + s * 0.5;
            let r = s * 0.42;
            ctx.move_to(cx, cy - r);
            ctx.line_to(cx + r * 0.87, cy - r * 0.5);
            ctx.line_to(cx + r * 0.87, cy + r * 0.5);
            ctx.line_to(cx, cy + r);
            ctx.line_to(cx - r * 0.87, cy + r * 0.5);
            ctx.line_to(cx - r * 0.87, cy - r * 0.5);
            ctx.close_path();
            ctx.move_to(cx - r * 0.87, cy - r * 0.5);
            ctx.line_to(cx, cy);
            ctx.line_to(cx + r * 0.87, cy - r * 0.5);
            ctx.move_to(cx, cy);
            ctx.line_to(cx, cy + r);
        }
    }
    ctx.stroke();
    ctx.restore();
}

fn draw_icon_button(
    ctx: &CanvasRenderingContext2d,
    x: f64,
    y: f64,
    size: f64,
    icon: ButtonIcon,
    active: bool,
    accent: &str,
) {
    round_rect(ctx, x, y, size, size, 5.0);
    ctx.set_fill_style_str(if active {
        "rgba(15, 23, 42, 0.92)"
    } else {
        "rgba(15, 23, 42, 0.45)"
    });
    ctx.fill();
    ctx.set_line_width(1.0);
    ctx.set_stroke_style_str(if active {
        accent
    } else {
        "rgba(100, 116, 139, 0.45)"
    });
    ctx.stroke();

    let color = if active { accent } else { "#64748b" };
    let ix = x + 5.0;
    let iy = y + 5.0;
    let s = size - 10.0;

    ctx.save();
    ctx.set_stroke_style_str(color);
    ctx.set_line_width(1.4);
    ctx.set_line_cap("round");
    ctx.set_line_join("round");
    ctx.begin_path();
    match icon {
        ButtonIcon::OpenEditor => {
            // External link box + arrow
            ctx.move_to(ix + s * 0.45, iy + s * 0.15);
            ctx.line_to(ix + s * 0.15, iy + s * 0.15);
            ctx.line_to(ix + s * 0.15, iy + s * 0.85);
            ctx.line_to(ix + s * 0.85, iy + s * 0.85);
            ctx.line_to(ix + s * 0.85, iy + s * 0.55);
            ctx.move_to(ix + s * 0.45, iy + s * 0.55);
            ctx.line_to(ix + s * 0.85, iy + s * 0.15);
            ctx.move_to(ix + s * 0.60, iy + s * 0.15);
            ctx.line_to(ix + s * 0.85, iy + s * 0.15);
            ctx.line_to(ix + s * 0.85, iy + s * 0.40);
        }
    }
    ctx.stroke();
    ctx.restore();
}

fn hit_test_world(
    wx: f64,
    wy: f64,
    groups: &[PositionedGroup],
    functions: &[PositionedFunction],
) -> Option<String> {
    for fn_item in functions.iter().rev() {
        let r = &fn_item.rect;
        if wx >= r.x && wx <= r.x + r.width && wy >= r.y && wy <= r.y + r.height {
            return Some(fn_item.node.id.clone());
        }
    }
    for g in groups.iter().rev() {
        let r = &g.rect;
        if wx >= r.x && wx <= r.x + r.width && wy >= r.y && wy <= r.y + r.height {
            return Some(g.node.id.clone());
        }
    }
    None
}

#[component]
pub fn CanvasView(
    layout: Memo<Arc<LayoutResult>>,
    viewport: RwSignal<Viewport>,
    selected_id: RwSignal<Option<String>>,
    hovered_id: RwSignal<Option<String>>,
    review_overlay: Memo<ReviewOverlay>,
    on_open_in_editor: Callback<String>,
    on_focus_node: Callback<String>,
) -> impl IntoView {
    let container_ref = NodeRef::<leptos::html::Div>::new();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let action_hits = Rc::new(RefCell::new(Vec::<ActionHit>::new()));
    let drag_state = Rc::new(RefCell::new(DragState::default()));

    // ResizeObserver to keep canvas crisp on HiDPI
    Effect::new(move |_| {
        let Some(container) = container_ref.get() else {
            return;
        };
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let container_el: HtmlElement = container.into();
        let canvas_el: HtmlCanvasElement = canvas.into();

        let sync_size = move |c: &HtmlElement, cv: &HtmlCanvasElement| {
            let rect = c.get_bounding_client_rect();
            let dpr = web_sys::window()
                .map(|w| w.device_pixel_ratio())
                .unwrap_or(1.0)
                .max(1.0);
            let w = (rect.width() * dpr).round().max(1.0) as u32;
            let h = (rect.height() * dpr).round().max(1.0) as u32;
            if cv.width() != w || cv.height() != h {
                cv.set_width(w);
                cv.set_height(h);
            }
        };
        sync_size(&container_el, &canvas_el);

        let c_clone = container_el.clone();
        let cv_clone = canvas_el.clone();
        let cb = Closure::<dyn FnMut(js_sys::Array)>::new(move |_entries: js_sys::Array| {
            sync_size(&c_clone, &cv_clone);
            // Trigger redraw by touching viewport
            viewport.update(|_| {});
        });
        if let Ok(obs) = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()) {
            obs.observe(&container_el);
            std::mem::forget(obs);
        }
        cb.forget();
    });

    let group_meta = Memo::new(move |_| {
        let l = layout.get();
        let mut dir_children: HashMap<&str, usize> = HashMap::new();
        let mut file_children: HashMap<&str, usize> = HashMap::new();
        let mut class_children: HashMap<&str, usize> = HashMap::new();
        for g in &l.groups {
            if let Some(pid) = g.node.parent_id.as_deref() {
                match g.node.kind {
                    GroupKind::Directory => *dir_children.entry(pid).or_insert(0) += 1,
                    GroupKind::Module => *file_children.entry(pid).or_insert(0) += 1,
                    GroupKind::Class => *class_children.entry(pid).or_insert(0) += 1,
                }
            }
        }
        let mut imp_in: HashMap<&str, usize> = HashMap::new();
        let mut imp_out: HashMap<&str, usize> = HashMap::new();
        for e in &l.edges {
            if e.edge.kind == EdgeKind::Import {
                *imp_out.entry(e.edge.source_id.as_str()).or_insert(0) += 1;
                *imp_in.entry(e.edge.target_id.as_str()).or_insert(0) += 1;
            }
        }
        let mut out = HashMap::new();
        for g in &l.groups {
            let gid = g.node.id.as_str();
            let text = match g.node.kind {
                GroupKind::Directory => {
                    let d = dir_children.get(gid).copied().unwrap_or(0);
                    let f = file_children.get(gid).copied().unwrap_or(0);
                    let d_lbl = if d == 1 { "dir" } else { "dirs" };
                    let f_lbl = if f == 1 { "file" } else { "files" };
                    format!("{d} {d_lbl} · {f} {f_lbl}")
                }
                GroupKind::Module => {
                    let c = class_children.get(gid).copied().unwrap_or(0);
                    let fns = g.function_count;
                    let ii = imp_in.get(gid).copied().unwrap_or(0);
                    let io = imp_out.get(gid).copied().unwrap_or(0);
                    let mut parts = vec![format!("{}L", g.node.line_count)];
                    if c > 0 {
                        parts.push(format!("{c} cls"));
                    }
                    parts.push(format!("{fns} {}", if fns == 1 { "fn" } else { "fns" }));
                    if ii > 0 || io > 0 {
                        parts.push(format!("{io} imp · {ii} dep"));
                    }
                    parts.join(" · ")
                }
                GroupKind::Class => {
                    let fns = g.function_count;
                    let mut parts = Vec::new();
                    if g.node.line_count > 0 {
                        parts.push(format!("{}L", g.node.line_count));
                    }
                    parts.push(format!("{fns} {}", if fns == 1 { "fn" } else { "fns" }));
                    parts.join(" · ")
                }
            };
            out.insert(g.node.id.clone(), text);
        }
        Arc::new(out)
    });

    let fn_counts = Memo::new(move |_| {
        let l = layout.get();
        let mut in_map: HashMap<&str, HashSet<&str>> = HashMap::new();
        let mut out_map: HashMap<&str, HashSet<&str>> = HashMap::new();
        for e in &l.edges {
            out_map
                .entry(e.edge.source_id.as_str())
                .or_default()
                .insert(e.edge.target_id.as_str());
            in_map
                .entry(e.edge.target_id.as_str())
                .or_default()
                .insert(e.edge.source_id.as_str());
        }
        Arc::new(
            l.functions
                .iter()
                .map(|f| {
                    let fid = f.node.id.as_str();
                    let c_in = in_map.get(fid).map_or(0, HashSet::len);
                    let c_out = out_map.get(fid).map_or(0, HashSet::len);
                    (f.node.id.clone(), (c_in, c_out))
                })
                .collect::<HashMap<String, (usize, usize)>>(),
        )
    });

    let edges_by_node = Memo::new(move |_| {
        let l = layout.get();
        let mut map: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, e) in l.edges.iter().enumerate() {
            map.entry(e.edge.source_id.clone()).or_default().push(idx);
            if e.edge.target_id != e.edge.source_id {
                map.entry(e.edge.target_id.clone()).or_default().push(idx);
            }
        }
        Arc::new(map)
    });

    // Render on reactive state changes
    let hits_for_render = action_hits.clone();
    Effect::new(move |_| {
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas.into();
        let l = layout.get();
        let vp = viewport.get();
        let sel = selected_id.get();
        let hov = hovered_id.get();
        let gm = group_meta.get();
        let fc = fn_counts.get();
        let ebn = edges_by_node.get();
        let rev = review_overlay.get();

        let mut hits = hits_for_render.borrow_mut();
        render_canvas(
            &canvas_el,
            &l,
            vp,
            sel.as_deref(),
            hov.as_deref(),
            &gm,
            &fc,
            &ebn,
            &rev,
            &mut hits,
        );
    });

    let on_wheel = move |ev: WheelEvent| {
        ev.prevent_default();
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas.into();
        let rect = canvas_el.get_bounding_client_rect();
        let mx = ev.client_x() as f64 - rect.left();
        let my = ev.client_y() as f64 - rect.top();
        let factor = if ev.delta_y() < 0.0 { 1.12 } else { 1.0 / 1.12 };

        viewport.update(|vp| {
            let new_scale = (vp.scale * factor).clamp(0.001, 3.0);
            let ratio = new_scale / vp.scale;
            vp.x = mx - (mx - vp.x) * ratio;
            vp.y = my - (my - vp.y) * ratio;
            vp.scale = new_scale;
        });
    };

    let drag_down = drag_state.clone();
    let on_mousedown = move |ev: MouseEvent| {
        if ev.button() != 0 {
            return;
        }
        let vp = viewport.get_untracked();
        let mut ds = drag_down.borrow_mut();
        ds.dragging = true;
        ds.moved = false;
        ds.start_x = ev.client_x() as f64;
        ds.start_y = ev.client_y() as f64;
        ds.orig_vx = vp.x;
        ds.orig_vy = vp.y;
    };

    let drag_move = drag_state.clone();
    let on_mousemove = move |ev: MouseEvent| {
        let cx = ev.client_x() as f64;
        let cy = ev.client_y() as f64;
        let mut ds = drag_move.borrow_mut();
        if ds.dragging {
            let dx = cx - ds.start_x;
            let dy = cy - ds.start_y;
            if dx.abs() > 3.0 || dy.abs() > 3.0 {
                ds.moved = true;
            }
            let orig_x = ds.orig_vx;
            let orig_y = ds.orig_vy;
            drop(ds);
            viewport.update(|vp| {
                vp.x = orig_x + dx;
                vp.y = orig_y + dy;
            });
            return;
        }
        drop(ds);

        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas.into();
        let rect = canvas_el.get_bounding_client_rect();
        let sx = cx - rect.left();
        let sy = cy - rect.top();
        let vp = viewport.get_untracked();
        let wx = (sx - vp.x) / vp.scale;
        let wy = (sy - vp.y) / vp.scale;
        let l = layout.get_untracked();
        let hit = hit_test_world(wx, wy, &l.groups, &l.functions);
        if hovered_id.get_untracked() != hit {
            hovered_id.set(hit);
        }
    };

    let drag_up = drag_state.clone();
    let hits_click = action_hits.clone();
    let on_mouseup = move |ev: MouseEvent| {
        let mut ds = drag_up.borrow_mut();
        let was_moved = ds.moved;
        ds.dragging = false;
        ds.moved = false;
        drop(ds);
        if was_moved {
            return;
        }

        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas.into();
        let rect = canvas_el.get_bounding_client_rect();
        let sx = ev.client_x() as f64 - rect.left();
        let sy = ev.client_y() as f64 - rect.top();

        // 1. Check screen-space action buttons first
        if let Some(action) = hits_click
            .borrow()
            .iter()
            .rev()
            .find(|h| sx >= h.x && sx <= h.x + h.w && sy >= h.y && sy <= h.y + h.h)
            .map(|h| h.kind.clone())
        {
            match action {
                ActionKind::OpenEditor(fp) => on_open_in_editor.run(fp),
            }
            return;
        }

        // 1b. Check overview minimap click to pan viewport
        let l = layout.get_untracked();
        if let Some((mm_x, mm_y, mm_w, mm_h, mm_scale, off_x, off_y)) =
            minimap_geometry(rect.width(), rect.height(), l.bounds)
        {
            if sx >= mm_x && sx <= mm_x + mm_w && sy >= mm_y && sy <= mm_y + mm_h {
                let wx = l.bounds.x + (sx - off_x) / mm_scale;
                let wy = l.bounds.y + (sy - off_y) / mm_scale;
                let css_w = rect.width();
                let css_h = rect.height();
                viewport.update(|vp| {
                    vp.x = css_w / 2.0 - wx * vp.scale;
                    vp.y = css_h / 2.0 - wy * vp.scale;
                });
                return;
            }
        }

        // 2. Otherwise select world node
        let vp = viewport.get_untracked();
        let wx = (sx - vp.x) / vp.scale;
        let wy = (sy - vp.y) / vp.scale;
        let hit = hit_test_world(wx, wy, &l.groups, &l.functions);
        selected_id.set(hit);
    };

    let drag_leave = drag_state.clone();
    let on_mouseleave = move |_| {
        drag_leave.borrow_mut().dragging = false;
        hovered_id.set(None);
    };

    let on_dblclick = move |ev: MouseEvent| {
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas.into();
        let rect = canvas_el.get_bounding_client_rect();
        let sx = ev.client_x() as f64 - rect.left();
        let sy = ev.client_y() as f64 - rect.top();
        let vp = viewport.get_untracked();
        let wx = (sx - vp.x) / vp.scale;
        let wy = (sy - vp.y) / vp.scale;
        let l = layout.get_untracked();
        if let Some(id) = hit_test_world(wx, wy, &l.groups, &l.functions) {
            selected_id.set(Some(id.clone()));
            on_focus_node.run(id);
        }
    };

    // Hover / Selected caller / callee badge
    let hover_badge = Memo::new(move |_| {
        let hid = hovered_id.get().or_else(|| selected_id.get())?;
        let l = layout.get();
        let ebn = edges_by_node.get();
        let mut callers = HashSet::new();
        let mut callees = HashSet::new();
        if let Some(indices) = ebn.get(&hid) {
            for &idx in indices {
                let e = &l.edges[idx];
                if e.edge.target_id == hid {
                    callers.insert(e.edge.source_id.clone());
                }
                if e.edge.source_id == hid {
                    callees.insert(e.edge.target_id.clone());
                }
            }
        }
        let is_fn = l.functions.iter().any(|f| f.node.id == hid);
        if !is_fn && callers.is_empty() && callees.is_empty() {
            return None;
        }
        let c_in = callers.len();
        let c_out = callees.len();
        let in_lbl = if c_in == 1 { "caller" } else { "callers" };
        let out_lbl = if c_out == 1 { "callee" } else { "callees" };
        Some(format!(
            "{} {in_lbl} · {} {out_lbl}",
            fmt_edge_count(c_in),
            fmt_edge_count(c_out)
        ))
    });

    // Breadcrumb path for hovered/selected node
    let breadcrumb = Memo::new(move |_| {
        let Some(target_id) = hovered_id.get().or_else(|| selected_id.get()) else {
            return Vec::<(String, String)>::new();
        };
        let l = layout.get();
        let group_map: HashMap<&str, (&str, Option<&str>)> = l
            .groups
            .iter()
            .map(|g| (g.node.id.as_str(), (g.node.name.as_str(), g.node.parent_id.as_deref())))
            .collect();

        let mut chain: Vec<(String, String)> = Vec::new();
        let mut cur_group: Option<&str> = if let Some(&(name, pid)) = group_map.get(target_id.as_str()) {
            chain.push((target_id.clone(), name.to_string()));
            pid
        } else if let Some(fn_item) = l.functions.iter().find(|f| f.node.id == target_id) {
            chain.push((fn_item.node.id.clone(), fn_item.node.name.clone()));
            Some(fn_item.node.parent_id.as_str())
        } else {
            None
        };

        let mut guard = 0usize;
        while let Some(gid) = cur_group {
            guard += 1;
            if guard > 64 {
                break;
            }
            if let Some(&(name, pid)) = group_map.get(gid) {
                chain.push((gid.to_string(), name.to_string()));
                cur_group = pid;
            } else {
                break;
            }
        }
        chain.reverse();
        chain
    });

    // Selected function DOM code overlay in Micro LOD
    let selected_fn_overlay = Memo::new(move |_| {
        let vp = viewport.get();
        if LodLevel::from_scale(vp.scale) != LodLevel::Micro {
            return None;
        }
        let sid = selected_id.get()?;
        let l = layout.get();
        let fn_item = l.functions.iter().find(|f| f.node.id == sid)?.clone();
        Some((fn_item, vp))
    });

    view! {
        <div class="canvas-viewport" node_ref=container_ref>
            <canvas
                class="canvas-2d"
                node_ref=canvas_ref
                on:wheel=on_wheel
                on:mousedown=on_mousedown
                on:mousemove=on_mousemove
                on:mouseup=on_mouseup
                on:mouseleave=on_mouseleave
                on:dblclick=on_dblclick
            />
            {move || {
                selected_fn_overlay.get().map(|(fn_item, vp)| {
                    let left = fn_item.rect.x * vp.scale + vp.x;
                    let top = (fn_item.rect.y + 56.0) * vp.scale + vp.y;
                    let width = fn_item.rect.width * vp.scale;
                    let height = (fn_item.rect.height - 58.0) * vp.scale;
                    let style = format!(
                        "left:{left:.1}px;top:{top:.1}px;width:{width:.1}px;height:{height:.1}px;font-size:{:.1}px;",
                        (11.5 * vp.scale).clamp(9.0, 18.0)
                    );
                    let file_path = fn_item.node.file_path.clone();
                    let start_line = fn_item.node.start_line;
                    let lines: Vec<(usize, String)> = fn_item
                        .node
                        .code
                        .lines()
                        .enumerate()
                        .map(|(i, s)| (start_line + i, s.to_string()))
                        .collect();
                    view! {
                        <div class="fn-code-overlay" style=style>
                            <pre class="fn-code-pre">
                                {lines.into_iter().map(|(ln, text)| {
                                    let tokens = tokenize_code_line(&text, Some(&file_path));
                                    view! {
                                        <div class="fn-code-line">
                                            <span class="fn-line-no">{format!("{ln:>3}")}</span>
                                            <span class="fn-line-src">
                                                {tokens.into_iter().map(|tok| {
                                                    let cls = tok.kind.css_class();
                                                    if cls.is_empty() {
                                                        view! { <span>{tok.text}</span> }.into_any()
                                                    } else {
                                                        view! { <span class=cls>{tok.text}</span> }.into_any()
                                                    }
                                                }).collect_view()}
                                            </span>
                                        </div>
                                    }
                                }).collect_view()}
                            </pre>
                        </div>
                    }
                })
            }}
            {move || {
                let crumbs = breadcrumb.get();
                if crumbs.is_empty() {
                    view! { <span /> }.into_any()
                } else {
                    let last_idx = crumbs.len().saturating_sub(1);
                    view! {
                        <div class="canvas-breadcrumb">
                            {crumbs.into_iter().enumerate().map(|(idx, (cid, label))| {
                                view! {
                                    <>
                                        {(idx > 0).then(|| view! { <span class="canvas-breadcrumb-sep">"›"</span> })}
                                        <button
                                            class=if idx == last_idx {
                                                "canvas-breadcrumb-item is-current"
                                            } else {
                                                "canvas-breadcrumb-item"
                                            }
                                            on:click=move |ev| {
                                                ev.stop_propagation();
                                                on_focus_node.run(cid.clone());
                                            }
                                        >
                                            {label}
                                        </button>
                                    </>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }
            }}
            {move || {
                hover_badge.get().map(|badge| view! {
                    <div class="canvas-hover-badge">{badge}</div>
                })
            }}
        </div>
    }
}
