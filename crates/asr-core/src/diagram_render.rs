//! Pure-Rust process flowchart layout engine, collision-free edge/label solver,
//! hand-drawn Excalidraw-style SVG renderer, and editable `.excalidraw` v2 JSON exporter.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::diagram::{DiagramNode, NodeKind, ProcessDiagram};

const TITLE_FONT_SIZE: f64 = 20.0;
const TITLE_LINE_HEIGHT: f64 = 28.0;
const LABEL_FONT_SIZE: f64 = 14.0;
const LABEL_LINE_HEIGHT: f64 = 20.0;
const ROLE_FONT_SIZE: f64 = 11.5;
const ROLE_LINE_HEIGHT: f64 = 16.0;
const EDGE_FONT_SIZE: f64 = 12.0;
const EDGE_LINE_HEIGHT: f64 = 17.0;

const PAD_X: f64 = 48.0;
const PAD_TOP: f64 = 36.0;
const PAD_BOTTOM: f64 = 44.0;
const LAYER_GAP_Y: f64 = 108.0;
const NODE_GAP_X: f64 = 64.0;

/// Palette aligned with `drawing-excalidraw-figures`: solid pastel fills + crisp ink borders.
const COLOR_BG: &str = "#ffffff";
const COLOR_INK: &str = "#111A2B";
const COLOR_ACCENT: &str = "#1E5AE8";
const COLOR_START_FILL: &str = "#DBEAFE";
const COLOR_STEP_FILL: &str = "#F2F6FB";
const COLOR_DECISION_FILL: &str = "#FEF3C7";
const COLOR_LABEL_FILL: &str = "#FFFFFF";
const COLOR_LABEL_BORDER: &str = "#5C6B82";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn centered(cx: f64, cy: f64, width: f64, height: f64) -> Self {
        Self {
            x: (cx - width / 2.0).round(),
            y: (cy - height / 2.0).round(),
            width,
            height,
        }
    }

    fn padded(self, pad: f64) -> Self {
        Self {
            x: self.x - pad,
            y: self.y - pad,
            width: self.width + pad * 2.0,
            height: self.height + pad * 2.0,
        }
    }

    fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub role: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub layer: usize,
    pub label_lines: Vec<String>,
    pub role_lines: Vec<String>,
}

impl LayoutNode {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    fn center_x(&self) -> f64 {
        (self.x + self.width / 2.0).round()
    }

    fn center_y(&self) -> f64 {
        (self.y + self.height / 2.0).round()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutEdge {
    pub from: String,
    pub to: String,
    pub label: String,
    pub index: usize,
    pub is_back_edge: bool,
    pub points: Vec<Point>,
    pub label_lines: Vec<String>,
    pub label_box: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutDiagram {
    pub diagram: ProcessDiagram,
    pub title_lines: Vec<String>,
    pub width: f64,
    pub height: f64,
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
}

/// Complete rendered output returned to the Tauri UI in a single IPC call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedDiagram {
    pub width: f64,
    pub height: f64,
    pub svg: String,
    pub excalidraw_json: String,
    pub drawio_xml: String,
}

fn node_fill_color(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Start | NodeKind::End => COLOR_START_FILL,
        NodeKind::Decision => COLOR_DECISION_FILL,
        NodeKind::Step => COLOR_STEP_FILL,
    }
}

// ---------------------------------------------------------------------------
// Vietnamese-aware text metrics & wrapping (pure Rust, no DOM dependency)
// ---------------------------------------------------------------------------

fn char_width_factor(ch: char) -> f64 {
    match ch {
        ' ' => 0.30,
        'i' | 'l' | 'j' | 't' | 'f' | 'r' | 'I' | '1' | '.' | ',' | ':' | ';' | '!' | '|'
        | '\'' | '(' | ')' | '[' | ']' | '/' | '\\' | 'í' | 'ì' | 'ỉ' | 'ĩ' | 'ị' | 'Í' | 'Ì'
        | 'Ỉ' | 'Ĩ' | 'Ị' => 0.37,
        'm' | 'w' | 'M' | 'W' | 'G' | 'O' | 'Q' | 'D' | 'H' | 'N' | 'U' | 'Đ' | 'Ô' | 'Ơ'
        | 'Ư' | 'Ă' | 'Â' | '@' | '%' => 0.78,
        c if c.is_uppercase() => 0.66,
        _ => 0.56,
    }
}

fn measure_text_width(text: &str, font_size: f64, weight: u16) -> f64 {
    let weight_factor = if weight >= 600 { 1.05 } else { 1.0 };
    let safety = 1.06;
    text.chars()
        .map(|c| char_width_factor(c) * font_size * weight_factor * safety)
        .sum()
}

fn wrap_text(text: &str, max_width: f64, font_size: f64, weight: u16) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    let push_long_word = |word: &str, lines: &mut Vec<String>, current: &mut String| {
        let mut chunk = String::new();
        for ch in word.chars() {
            let mut next = chunk.clone();
            next.push(ch);
            if !chunk.is_empty() && measure_text_width(&next, font_size, weight) > max_width {
                lines.push(chunk);
                chunk = ch.to_string();
            } else {
                chunk = next;
            }
        }
        if !chunk.is_empty() {
            *current = chunk;
        }
    };

    for word in words {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if measure_text_width(&candidate, font_size, weight) <= max_width {
            current = candidate;
        } else if current.is_empty() {
            push_long_word(word, &mut lines, &mut current);
        } else {
            lines.push(current);
            current = String::new();
            if measure_text_width(word, font_size, weight) <= max_width {
                current = word.to_owned();
            } else {
                push_long_word(word, &mut lines, &mut current);
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn max_line_width(lines: &[String], font_size: f64, weight: u16) -> f64 {
    lines
        .iter()
        .map(|l| measure_text_width(l, font_size, weight))
        .fold(0.0_f64, f64::max)
}

struct MeasuredNode {
    width: f64,
    height: f64,
    label_lines: Vec<String>,
    role_lines: Vec<String>,
}

fn measure_node(node: &DiagramNode) -> MeasuredNode {
    let max_label_w = if node.kind == NodeKind::Decision {
        192.0
    } else {
        232.0
    };
    let label_lines = wrap_text(&node.label, max_label_w, LABEL_FONT_SIZE, 600);
    let role_lines = if node.role.trim().is_empty() {
        Vec::new()
    } else {
        wrap_text(&node.role, max_label_w, ROLE_FONT_SIZE, 500)
    };

    let text_w = max_line_width(&label_lines, LABEL_FONT_SIZE, 600)
        .max(max_line_width(&role_lines, ROLE_FONT_SIZE, 500));
    let label_h = label_lines.len().max(1) as f64 * LABEL_LINE_HEIGHT;
    let role_h = if role_lines.is_empty() {
        0.0
    } else {
        role_lines.len() as f64 * ROLE_LINE_HEIGHT + 6.0
    };
    let total_text_h = label_h + role_h;

    let (width, height) = match node.kind {
        NodeKind::Decision => (
            (text_w * 1.72 + 56.0).max(224.0).ceil(),
            (total_text_h * 1.82 + 48.0).max(118.0).ceil(),
        ),
        NodeKind::Start | NodeKind::End => (
            (text_w + 54.0).max(184.0).ceil(),
            (total_text_h + 28.0).max(60.0).ceil(),
        ),
        NodeKind::Step => (
            (text_w + 46.0).max(206.0).ceil(),
            (total_text_h + 28.0).max(66.0).ceil(),
        ),
    };

    MeasuredNode {
        width,
        height,
        label_lines,
        role_lines,
    }
}

// ---------------------------------------------------------------------------
// Geometry helpers for collision-free routing & label placement
// ---------------------------------------------------------------------------

/// Checks whether a rectangle intersects an axis-aligned diamond (optionally inflated by `pad`).
fn rect_intersects_diamond(rect: Rect, node: &LayoutNode, pad: f64) -> bool {
    let bbox = node.rect().padded(pad);
    if !rect.intersects(bbox) {
        return false;
    }
    let cx = node.x + node.width / 2.0;
    let cy = node.y + node.height / 2.0;
    let hw = node.width / 2.0 + pad;
    let hh = node.height / 2.0 + pad;
    // Closest point in `rect` to `(cx, cy)` minimizes |dx|/hw + |dy|/hh for an axis-aligned diamond.
    let closest_x = cx.clamp(rect.x, rect.right());
    let closest_y = cy.clamp(rect.y, rect.bottom());
    let norm = (closest_x - cx).abs() / hw + (closest_y - cy).abs() / hh;
    norm <= 1.0
}

fn rect_intersects_node(rect: Rect, node: &LayoutNode, pad: f64) -> bool {
    if node.kind == NodeKind::Decision {
        rect_intersects_diamond(rect, node, pad + 2.0)
    } else {
        rect.intersects(node.rect().padded(pad))
    }
}

/// Checks whether an axis-aligned segment `(a, b)` intersects `rect`.
fn ortho_segment_intersects_rect(a: Point, b: Point, rect: Rect) -> bool {
    let min_x = a.x.min(b.x);
    let max_x = a.x.max(b.x);
    let min_y = a.y.min(b.y);
    let max_y = a.y.max(b.y);
    max_x >= rect.x && min_x <= rect.right() && max_y >= rect.y && min_y <= rect.bottom()
}

fn points_intersect_any_intermediate(
    points: &[Point],
    nodes: &[LayoutNode],
    src_id: &str,
    dst_id: &str,
    pad: f64,
) -> bool {
    for seg in points.windows(2) {
        for n in nodes {
            if n.id == src_id || n.id == dst_id {
                continue;
            }
            if ortho_segment_intersects_rect(seg[0], seg[1], n.rect().padded(pad)) {
                return true;
            }
        }
    }
    false
}

struct LabelSolverCtx<'a> {
    nodes: &'a [LayoutNode],
    placed_labels: &'a [Rect],
    title_obstacle: Rect,
    all_edge_segments: &'a [(usize, Point, Point)],
}

/// Finds a guaranteed collision-free `Rect` for an edge label.
fn place_edge_label(
    edge_idx: usize,
    points: &[Point],
    label_size: (f64, f64),
    preferred: Point,
    ctx: &LabelSolverCtx<'_>,
) -> Rect {
    let (label_w, label_h) = label_size;
    let nodes = ctx.nodes;
    let placed_labels = ctx.placed_labels;
    let title_obstacle = ctx.title_obstacle;
    let all_edge_segments = ctx.all_edge_segments;
    let mut candidates: Vec<(Point, f64)> = Vec::new();
    candidates.push((preferred, 0.0));

    // Sample candidate points along every segment of this edge (preferring vertical drops first).
    let mut segments: Vec<(Point, Point, bool, f64)> = points
        .windows(2)
        .map(|w| {
            let dx = (w[1].x - w[0].x).abs();
            let dy = (w[1].y - w[0].y).abs();
            let is_vert = dy >= dx;
            let len = dx.hypot(dy);
            (w[0], w[1], is_vert, len)
        })
        .filter(|s| s.3 >= 22.0)
        .collect();

    // Sort: longest vertical segments first (closest to end of edge), then long horizontal segments.
    segments.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal))
    });

    for (seg_rank, &(p0, p1, is_vert, len)) in segments.iter().enumerate() {
        let base_pen = (seg_rank as f64) * 18.0 + if is_vert { 0.0 } else { 25.0 };
        for &t in &[0.5, 0.38, 0.62, 0.26, 0.74, 0.18, 0.82] {
            let pt = Point {
                x: (p0.x + (p1.x - p0.x) * t).round(),
                y: (p0.y + (p1.y - p0.y) * t).round(),
            };
            candidates.push((pt, base_pen + ((t - 0.5).abs() * 40.0)));
        }
        // Side offsets off the segment midpoint (very helpful when two edges run near each other).
        if len >= 24.0 {
            let mid = Point {
                x: ((p0.x + p1.x) / 2.0).round(),
                y: ((p0.y + p1.y) / 2.0).round(),
            };
            let off_x = (label_w / 2.0 + 18.0).round();
            let off_y = (label_h / 2.0 + 16.0).round();
            if is_vert {
                candidates.push((Point { x: mid.x + off_x, y: mid.y }, base_pen + 36.0));
                candidates.push((Point { x: mid.x - off_x, y: mid.y }, base_pen + 38.0));
            } else {
                candidates.push((Point { x: mid.x, y: mid.y - off_y }, base_pen + 36.0));
                candidates.push((Point { x: mid.x, y: mid.y + off_y }, base_pen + 38.0));
            }
        }
    }

    let is_clear = |cand: Rect| -> bool {
        if cand.intersects(title_obstacle) {
            return false;
        }
        if nodes.iter().any(|n| rect_intersects_node(cand, n, 12.0)) {
            return false;
        }
        if placed_labels
            .iter()
            .any(|prev| cand.intersects(prev.padded(10.0)))
        {
            return false;
        }
        true
    };

    let foreign_edge_crossings = |cand: Rect| -> usize {
        let padded = cand.padded(4.0);
        all_edge_segments
            .iter()
            .filter(|&&(idx, a, b)| idx != edge_idx && ortho_segment_intersects_rect(a, b, padded))
            .count()
    };

    let mut best: Option<(Rect, f64)> = None;
    for (pt, pen) in candidates {
        let cand = Rect::centered(pt.x, pt.y, label_w, label_h);
        if !is_clear(cand) {
            continue;
        }
        let dist = (pt.x - preferred.x).hypot(pt.y - preferred.y);
        let crossings = foreign_edge_crossings(cand) as f64;
        let score = pen + dist * 0.25 + crossings * 400.0;
        if best.is_none_or(|(_, best_score)| score < best_score) {
            best = Some((cand, score));
        }
    }
    if let Some((rect, _)) = best {
        return rect;
    }

    // Guaranteed fallback: expanding spiral search around `preferred` until a collision-free slot is found.
    let step = 16.0;
    for ring in 1..=30 {
        let r = ring as f64 * step;
        let mut ring_pts = Vec::new();
        for k in -ring..=ring {
            let offset = k as f64 * step;
            ring_pts.push(Point { x: preferred.x + offset, y: preferred.y - r });
            ring_pts.push(Point { x: preferred.x + offset, y: preferred.y + r });
            ring_pts.push(Point { x: preferred.x - r, y: preferred.y + offset });
            ring_pts.push(Point { x: preferred.x + r, y: preferred.y + offset });
        }
        let mut ring_best: Option<(Rect, f64)> = None;
        for pt in ring_pts {
            let cand = Rect::centered(pt.x, pt.y, label_w, label_h);
            if !is_clear(cand) {
                continue;
            }
            let dist = (pt.x - preferred.x).hypot(pt.y - preferred.y);
            let score = dist + foreign_edge_crossings(cand) as f64 * 400.0;
            if ring_best.is_none_or(|(_, s)| score < s) {
                ring_best = Some((cand, score));
            }
        }
        if let Some((rect, _)) = ring_best {
            return rect;
        }
    }

    Rect::centered(preferred.x, preferred.y, label_w, label_h)
}

// ---------------------------------------------------------------------------
// Main layout engine
// ---------------------------------------------------------------------------

/// Validates diagram structure and computes deterministic, collision-free flowchart coordinates.
pub fn layout_diagram(diagram: &ProcessDiagram) -> Result<LayoutDiagram, String> {
    if diagram.nodes.len() < 2 {
        return Err("Sơ đồ cần ít nhất 2 bước để dựng hình.".into());
    }
    if diagram.edges.is_empty() {
        return Err("Sơ đồ chưa có đường nối giữa các bước.".into());
    }

    let mut node_by_id: HashMap<&str, &DiagramNode> = HashMap::with_capacity(diagram.nodes.len());
    for node in &diagram.nodes {
        if node.id.trim().is_empty() || node.label.trim().is_empty() {
            return Err("Dữ liệu bước trong sơ đồ bị thiếu mã hoặc nhãn.".into());
        }
        if node_by_id.insert(node.id.as_str(), node).is_some() {
            return Err(format!("Trùng mã bước \"{}\" trong sơ đồ.", node.id));
        }
    }

    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::with_capacity(diagram.nodes.len());
    let mut in_degree: HashMap<&str, usize> = HashMap::with_capacity(diagram.nodes.len());
    for node in &diagram.nodes {
        outgoing.insert(node.id.as_str(), Vec::new());
        in_degree.insert(node.id.as_str(), 0);
    }
    for edge in &diagram.edges {
        if !node_by_id.contains_key(edge.from.as_str())
            || !node_by_id.contains_key(edge.to.as_str())
        {
            return Err(format!(
                "Đường nối trỏ đến bước không tồn tại ({} -> {}).",
                edge.from, edge.to
            ));
        }
        if let Some(list) = outgoing.get_mut(edge.from.as_str()) {
            list.push(edge.to.as_str());
        }
        if let Some(deg) = in_degree.get_mut(edge.to.as_str()) {
            *deg += 1;
        }
    }

    // Classify cycle back-edges via DFS starting from start/root nodes.
    let mut roots: Vec<&str> = diagram
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Start || *in_degree.get(n.id.as_str()).unwrap_or(&0) == 0)
        .map(|n| n.id.as_str())
        .collect();
    if roots.is_empty() {
        roots.push(diagram.nodes[0].id.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut on_stack: HashSet<&str> = HashSet::new();
    let mut back_edge_pairs: HashSet<(String, String)> = HashSet::new();
    let mut order: Vec<&str> = Vec::new();

    fn dfs<'a>(
        id: &'a str,
        outgoing: &HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        on_stack: &mut HashSet<&'a str>,
        back_edge_pairs: &mut HashSet<(String, String)>,
        order: &mut Vec<&'a str>,
    ) {
        visited.insert(id);
        on_stack.insert(id);
        order.push(id);
        if let Some(nexts) = outgoing.get(id) {
            for &next in nexts {
                if on_stack.contains(next) {
                    back_edge_pairs.insert((id.to_owned(), next.to_owned()));
                } else if !visited.contains(next) {
                    dfs(next, outgoing, visited, on_stack, back_edge_pairs, order);
                }
            }
        }
        on_stack.remove(id);
    }

    for r in roots {
        if !visited.contains(r) {
            dfs(
                r,
                &outgoing,
                &mut visited,
                &mut on_stack,
                &mut back_edge_pairs,
                &mut order,
            );
        }
    }
    for n in &diagram.nodes {
        if !visited.contains(n.id.as_str()) {
            dfs(
                n.id.as_str(),
                &outgoing,
                &mut visited,
                &mut on_stack,
                &mut back_edge_pairs,
                &mut order,
            );
        }
    }

    // Compute topological longest-path layers on the DAG (ignoring back-edges).
    let mut layer_of: HashMap<&str, usize> = HashMap::with_capacity(diagram.nodes.len());
    for id in &order {
        layer_of.insert(*id, 0);
    }
    for _ in 0..diagram.nodes.len() {
        let mut changed = false;
        for edge in &diagram.edges {
            if edge.from == edge.to
                || back_edge_pairs.contains(&(edge.from.clone(), edge.to.clone()))
            {
                continue;
            }
            let cand = layer_of.get(edge.from.as_str()).copied().unwrap_or(0) + 1;
            let cur = layer_of.get(edge.to.as_str()).copied().unwrap_or(0);
            if cand > cur {
                layer_of.insert(edge.to.as_str(), cand);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Group nodes by layer, preserving input order within each layer.
    let mut layers: HashMap<usize, Vec<&DiagramNode>> = HashMap::new();
    let mut max_layer = 0usize;
    for node in &diagram.nodes {
        let l = layer_of.get(node.id.as_str()).copied().unwrap_or(0);
        max_layer = max_layer.max(l);
        layers.entry(l).or_default().push(node);
    }

    let mut measured: HashMap<&str, MeasuredNode> = HashMap::with_capacity(diagram.nodes.len());
    for node in &diagram.nodes {
        measured.insert(node.id.as_str(), measure_node(node));
    }

    let mut row_width: HashMap<usize, f64> = HashMap::new();
    let mut row_height: HashMap<usize, f64> = HashMap::new();
    let mut max_row_w = 420.0_f64;
    for l in 0..=max_layer {
        let list = layers.get(&l).map(Vec::as_slice).unwrap_or(&[]);
        let mut w = 0.0_f64;
        let mut h = 0.0_f64;
        for (idx, n) in list.iter().enumerate() {
            let m = &measured[n.id.as_str()];
            w += m.width + if idx > 0 { NODE_GAP_X } else { 0.0 };
            h = h.max(m.height);
        }
        row_width.insert(l, w);
        row_height.insert(l, h);
        max_row_w = max_row_w.max(w);
    }

    // Reserve space for title at top.
    let title_text = if diagram.title.trim().is_empty() {
        "Sơ đồ quy trình"
    } else {
        diagram.title.trim()
    };
    let title_lines = wrap_text(title_text, max_row_w.max(480.0), TITLE_FONT_SIZE, 600);
    let title_block_h = title_lines.len() as f64 * TITLE_LINE_HEIGHT + 28.0;
    let title_obstacle = Rect {
        x: -10000.0,
        y: -10000.0,
        width: 20000.0,
        height: 10000.0 + PAD_TOP + title_block_h - 6.0,
    };

    // Position nodes.
    let mut layout_nodes: Vec<LayoutNode> = Vec::with_capacity(diagram.nodes.len());
    let mut index_by_id: HashMap<String, usize> = HashMap::with_capacity(diagram.nodes.len());
    let mut cursor_y = PAD_TOP + title_block_h;
    let mut content_max_x = PAD_X + max_row_w;

    for l in 0..=max_layer {
        let list = layers.get(&l).map(Vec::as_slice).unwrap_or(&[]);
        let rw = row_width.get(&l).copied().unwrap_or(0.0);
        let rh = row_height.get(&l).copied().unwrap_or(0.0);
        let mut cursor_x = PAD_X + (max_row_w - rw) / 2.0;
        for node in list {
            let m = &measured[node.id.as_str()];
            let ln = LayoutNode {
                id: node.id.clone(),
                kind: node.kind,
                label: node.label.clone(),
                role: node.role.clone(),
                x: cursor_x.round(),
                y: (cursor_y + (rh - m.height) / 2.0).round(),
                width: m.width,
                height: m.height,
                layer: l,
                label_lines: m.label_lines.clone(),
                role_lines: m.role_lines.clone(),
            };
            content_max_x = content_max_x.max(ln.x + ln.width);
            index_by_id.insert(ln.id.clone(), layout_nodes.len());
            layout_nodes.push(ln);
            cursor_x += m.width + NODE_GAP_X;
        }
        if !list.is_empty() {
            cursor_y += rh + LAYER_GAP_Y;
        }
    }

    // Classify forward vs back edges and compute port ordering per source and per target.
    let mut is_back_flags: Vec<bool> = Vec::with_capacity(diagram.edges.len());
    let mut out_forward: HashMap<String, Vec<usize>> = HashMap::new();
    let mut in_forward: HashMap<String, Vec<usize>> = HashMap::new();

    for (idx, edge) in diagram.edges.iter().enumerate() {
        let src = &layout_nodes[index_by_id[&edge.from]];
        let dst = &layout_nodes[index_by_id[&edge.to]];
        let is_back =
            back_edge_pairs.contains(&(edge.from.clone(), edge.to.clone())) || dst.layer <= src.layer;
        is_back_flags.push(is_back);
        if !is_back {
            out_forward.entry(edge.from.clone()).or_default().push(idx);
            in_forward.entry(edge.to.clone()).or_default().push(idx);
        }
    }

    // Sort outgoing edges left-to-right by target center X, incoming edges left-to-right by source center X.
    for indices in out_forward.values_mut() {
        indices.sort_by(|&a, &b| {
            let da = &layout_nodes[index_by_id[&diagram.edges[a].to]];
            let db = &layout_nodes[index_by_id[&diagram.edges[b].to]];
            da.center_x()
                .partial_cmp(&db.center_x())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(&b))
        });
    }
    for indices in in_forward.values_mut() {
        indices.sort_by(|&a, &b| {
            let ea = &diagram.edges[a];
            let eb = &diagram.edges[b];
            let sa = &layout_nodes[index_by_id[&ea.from]];
            let sb = &layout_nodes[index_by_id[&eb.from]];
            let da = &layout_nodes[index_by_id[&ea.to]];
            let db = &layout_nodes[index_by_id[&eb.to]];
            // Multi-layer bypass edges approach from outside the intermediate nodes (right side by default),
            // so assign them outer ports instead of crossing shorter local edges.
            let key_a = if da.layer > sa.layer + 1 {
                f64::INFINITY
            } else {
                sa.center_x()
            };
            let key_b = if db.layer > sb.layer + 1 {
                f64::INFINITY
            } else {
                sb.center_x()
            };
            key_a
                .partial_cmp(&key_b)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(&b))
        });
    }

    // Pass 1: Route every edge with obstacle-aware bypass channels and staggered ports.
    let mut routed_points: Vec<Vec<Point>> = Vec::with_capacity(diagram.edges.len());
    let mut preferred_anchors: Vec<Point> = Vec::with_capacity(diagram.edges.len());
    let mut back_edge_lane = 0usize;
    let mut bypass_right_lane = 0usize;
    let mut bypass_left_lane = 0usize;
    let mut right_most_x = content_max_x;

    for (index, edge) in diagram.edges.iter().enumerate() {
        let src = &layout_nodes[index_by_id[&edge.from]];
        let dst = &layout_nodes[index_by_id[&edge.to]];
        let is_back = is_back_flags[index];

        let (points, pref_anchor) = if is_back {
            let lane_x = (content_max_x + 56.0 + back_edge_lane as f64 * 56.0).round();
            back_edge_lane += 1;
            let start_pt = Point {
                x: src.x + src.width,
                y: src.center_y(),
            };
            let end_pt = Point {
                x: dst.x + dst.width,
                y: dst.center_y(),
            };
            let pts = vec![
                start_pt,
                Point { x: lane_x, y: start_pt.y },
                Point { x: lane_x, y: end_pt.y },
                end_pt,
            ];
            let anchor = Point {
                x: lane_x,
                y: ((start_pt.y + end_pt.y) / 2.0).round(),
            };
            right_most_x = right_most_x.max(lane_x + 36.0);
            (pts, anchor)
        } else {
            let out_sibs = &out_forward[&edge.from];
            let in_sibs = &in_forward[&edge.to];
            let out_pos = out_sibs.iter().position(|&i| i == index).unwrap_or(0);
            let in_pos = in_sibs.iter().position(|&i| i == index).unwrap_or(0);

            // Stagger incoming ports across the target top edge when multiple edges converge.
            let end_x = if in_sibs.len() >= 2 {
                let span = (dst.width * 0.58).min((in_sibs.len() - 1) as f64 * 62.0);
                (dst.center_x() - span / 2.0 + span * (in_pos as f64) / ((in_sibs.len() - 1) as f64))
                    .round()
            } else {
                dst.center_x()
            };
            let end_pt = Point { x: end_x, y: dst.y };

            // Determine start port (side of diamond only when target is strictly outside diamond span).
            let mut exited_diamond_side = false;
            let start_pt = if src.kind == NodeKind::Decision && out_sibs.len() >= 2 {
                if end_x <= src.x - 24.0 {
                    exited_diamond_side = true;
                    Point {
                        x: src.x,
                        y: src.center_y(),
                    }
                } else if end_x >= src.x + src.width + 24.0 {
                    exited_diamond_side = true;
                    Point {
                        x: src.x + src.width,
                        y: src.center_y(),
                    }
                } else {
                    Point {
                        x: src.center_x(),
                        y: src.y + src.height,
                    }
                }
            } else if out_sibs.len() >= 2 {
                let span = (src.width * 0.44).min((out_sibs.len() - 1) as f64 * 34.0);
                let sx = (src.center_x() - span / 2.0
                    + span * (out_pos as f64) / ((out_sibs.len() - 1) as f64))
                    .round();
                Point {
                    x: sx,
                    y: src.y + src.height,
                }
            } else {
                Point {
                    x: src.center_x(),
                    y: src.y + src.height,
                }
            };

            // Build nominal orthogonal path first.
            let (nominal_pts, nominal_anchor) = if (start_pt.x - end_pt.x).abs() <= 4.0 {
                let aligned_end = Point {
                    x: start_pt.x,
                    y: end_pt.y,
                };
                (
                    vec![start_pt, aligned_end],
                    Point {
                        x: start_pt.x,
                        y: ((start_pt.y + aligned_end.y) / 2.0).round(),
                    },
                )
            } else if exited_diamond_side {
                let elbow = Point {
                    x: end_pt.x,
                    y: start_pt.y,
                };
                let safe_top = (start_pt.y + 26.0).max(src.y + src.height - 6.0);
                let safe_bot = safe_top.max(end_pt.y - 26.0);
                (
                    vec![start_pt, elbow, end_pt],
                    Point {
                        x: end_pt.x,
                        y: ((safe_top + safe_bot) / 2.0).round(),
                    },
                )
            } else {
                // Stagger horizontal jog Y slightly for sibling branches so lines don't overlap.
                let stagger = (out_pos as f64 - (out_sibs.len() as f64 - 1.0) / 2.0) * 10.0;
                let base_mid = start_pt.y + (end_pt.y - start_pt.y) * 0.34 + stagger;
                let mid_y = base_mid
                    .clamp(start_pt.y + 18.0, (end_pt.y - 44.0).max(start_pt.y + 18.0))
                    .round();
                (
                    vec![
                        start_pt,
                        Point { x: start_pt.x, y: mid_y },
                        Point { x: end_pt.x, y: mid_y },
                        end_pt,
                    ],
                    Point {
                        x: end_pt.x,
                        y: ((mid_y + end_pt.y) / 2.0).round(),
                    },
                )
            };

            // Check if nominal path passes through or hugs any intermediate node (multi-layer skip edge).
            if dst.layer > src.layer + 1
                && points_intersect_any_intermediate(
                    &nominal_pts,
                    &layout_nodes,
                    &src.id,
                    &dst.id,
                    20.0,
                )
            {
                let blockers: Vec<&LayoutNode> = layout_nodes
                    .iter()
                    .filter(|n| n.layer > src.layer && n.layer < dst.layer)
                    .collect();
                let max_blocker_right = blockers
                    .iter()
                    .map(|n| n.x + n.width)
                    .fold(src.x + src.width, f64::max);
                let min_blocker_left = blockers
                    .iter()
                    .map(|n| n.x)
                    .fold(src.x, f64::min);

                let avg_x = (start_pt.x + end_pt.x) / 2.0;
                let dist_right = (max_blocker_right - avg_x).abs();
                let dist_left = (avg_x - min_blocker_left).abs();

                let lane_x = if dist_right <= dist_left {
                    let x = (max_blocker_right + 48.0 + bypass_right_lane as f64 * 48.0).round();
                    bypass_right_lane += 1;
                    right_most_x = right_most_x.max(x + 36.0);
                    x
                } else {
                    let x = (min_blocker_left - 48.0 - bypass_left_lane as f64 * 48.0).round();
                    bypass_left_lane += 1;
                    x
                };

                let leave_y = (src.y + src.height + 22.0).round();
                let enter_y = (dst.y - 26.0).round();
                let bypass_start = Point {
                    x: src.center_x(),
                    y: src.y + src.height,
                };
                let pts = vec![
                    bypass_start,
                    Point {
                        x: bypass_start.x,
                        y: leave_y,
                    },
                    Point {
                        x: lane_x,
                        y: leave_y,
                    },
                    Point {
                        x: lane_x,
                        y: enter_y,
                    },
                    Point {
                        x: end_pt.x,
                        y: enter_y,
                    },
                    end_pt,
                ];
                let anchor = Point {
                    x: lane_x,
                    y: ((leave_y + enter_y) / 2.0).round(),
                };
                (pts, anchor)
            } else {
                (nominal_pts, nominal_anchor)
            }
        };

        routed_points.push(points);
        preferred_anchors.push(pref_anchor);
    }

    // Collect all edge segments so the label solver can avoid crossing unrelated edges too.
    let mut all_edge_segments: Vec<(usize, Point, Point)> = Vec::new();
    for (idx, pts) in routed_points.iter().enumerate() {
        for seg in pts.windows(2) {
            all_edge_segments.push((idx, seg[0], seg[1]));
        }
    }

    // Pass 2: Place every edge label with guaranteed zero overlap against nodes, diamonds, title, and other labels.
    let mut layout_edges: Vec<LayoutEdge> = Vec::with_capacity(diagram.edges.len());
    let mut placed_labels: Vec<Rect> = Vec::new();

    for (index, edge) in diagram.edges.iter().enumerate() {
        let label_lines = if edge.label.trim().is_empty() {
            Vec::new()
        } else {
            wrap_text(&edge.label, 168.0, EDGE_FONT_SIZE, 500)
        };
        let label_box = if label_lines.is_empty() {
            None
        } else {
            let label_w = (max_line_width(&label_lines, EDGE_FONT_SIZE, 500) + 18.0).ceil();
            let label_h = label_lines.len() as f64 * EDGE_LINE_HEIGHT + 10.0;
            let rect = place_edge_label(
                index,
                &routed_points[index],
                (label_w, label_h),
                preferred_anchors[index],
                &LabelSolverCtx {
                    nodes: &layout_nodes,
                    placed_labels: &placed_labels,
                    title_obstacle,
                    all_edge_segments: &all_edge_segments,
                },
            );
            right_most_x = right_most_x.max(rect.right() + 18.0);
            placed_labels.push(rect);
            Some(rect)
        };

        layout_edges.push(LayoutEdge {
            from: edge.from.clone(),
            to: edge.to.clone(),
            label: edge.label.clone(),
            index,
            is_back_edge: is_back_flags[index],
            points: routed_points[index].clone(),
            label_lines,
            label_box,
        });
    }

    // Ensure nothing bleeds past x = 24 on the left.
    let mut min_x = PAD_X;
    for n in &layout_nodes {
        min_x = min_x.min(n.x);
    }
    for e in &layout_edges {
        for p in &e.points {
            min_x = min_x.min(p.x - 12.0);
        }
        if let Some(b) = e.label_box {
            min_x = min_x.min(b.x);
        }
    }
    let shift_x = if min_x < 24.0 {
        (24.0 - min_x).ceil()
    } else {
        0.0
    };
    if shift_x > 0.0 {
        for n in &mut layout_nodes {
            n.x += shift_x;
        }
        for e in &mut layout_edges {
            for p in &mut e.points {
                p.x += shift_x;
            }
            if let Some(b) = &mut e.label_box {
                b.x += shift_x;
            }
        }
        right_most_x += shift_x;
    }

    let mut max_y = cursor_y - LAYER_GAP_Y;
    for e in &layout_edges {
        if let Some(b) = e.label_box {
            max_y = max_y.max(b.bottom());
        }
    }

    let total_width = (right_most_x + PAD_X).max(560.0).ceil();
    let total_height = (max_y + PAD_BOTTOM).max(260.0).ceil();

    Ok(LayoutDiagram {
        diagram: diagram.clone(),
        title_lines,
        width: total_width,
        height: total_height,
        nodes: layout_nodes,
        edges: layout_edges,
    })
}

// ---------------------------------------------------------------------------
// Pure-Rust Rough.js hand-drawn stroke engine (deterministic FNV-1a + Mulberry32)
// ---------------------------------------------------------------------------

struct Rng(u32);

impl Rng {
    fn from_key(key: &str) -> Self {
        let mut h: u32 = 2166136261;
        for b in key.as_bytes() {
            h ^= *b as u32;
            h = h.wrapping_mul(16777619);
        }
        Self(if h == 0 { 1337 } else { h })
    }

    fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0
    }

    /// Returns a uniform random offset in `[-max, max]`.
    fn offset(&mut self, max: f64) -> f64 {
        (self.next_f64() * 2.0 - 1.0) * max
    }
}

fn fmt_num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-6 {
        format!("{}", r.round() as i64)
    } else {
        format!("{r:.2}")
    }
}

#[derive(Clone, Copy)]
struct RoughStyle {
    roughness: f64,
    bowing: f64,
}

/// Appends Rough.js-style double-stroke cubic Bézier sketch of a line segment.
fn push_rough_line(out: &mut String, from: Point, to: Point, style: RoughStyle, rng: &mut Rng) {
    let (x1, y1, x2, y2) = (from.x, from.y, to.x, to.y);
    let RoughStyle { roughness, bowing } = style;
    let len = (x2 - x1).hypot(y2 - y1);
    if len < 0.5 {
        return;
    }
    let max_offset = (roughness * if len < 200.0 { 1.0 } else if len > 500.0 { 0.4 } else { 0.7 })
        .min(len / 12.0);
    let diverge_point = 0.2 + rng.next_f64() * 0.2;

    for pass in 0..2 {
        let scale = if pass == 0 { 1.0 } else { 0.65 };
        let mo = max_offset * scale;
        let mid_disp_x = bowing * max_offset * (y2 - y1) / 200.0;
        let mid_disp_y = bowing * max_offset * (x1 - x2) / 200.0;
        let dx_bow = rng.offset(mid_disp_x.abs().max(mo * 0.5));
        let dy_bow = rng.offset(mid_disp_y.abs().max(mo * 0.5));

        let sx = x1 + rng.offset(mo);
        let sy = y1 + rng.offset(mo);
        let ex = x2 + rng.offset(mo);
        let ey = y2 + rng.offset(mo);

        let cp1x = x1 + (x2 - x1) * diverge_point + dx_bow + rng.offset(mo);
        let cp1y = y1 + (y2 - y1) * diverge_point + dy_bow + rng.offset(mo);
        let cp2x = x1 + (x2 - x1) * (1.0 - diverge_point) + dx_bow + rng.offset(mo);
        let cp2y = y1 + (y2 - y1) * (1.0 - diverge_point) + dy_bow + rng.offset(mo);

        let _ = write!(
            out,
            "M {} {} C {} {}, {} {}, {} {} ",
            fmt_num(sx),
            fmt_num(sy),
            fmt_num(cp1x),
            fmt_num(cp1y),
            fmt_num(cp2x),
            fmt_num(cp2y),
            fmt_num(ex),
            fmt_num(ey),
        );
    }
}

fn rough_polyline_path(points: &[Point], roughness: f64, bowing: f64, rng: &mut Rng) -> String {
    let mut out = String::new();
    let style = RoughStyle { roughness, bowing };
    for seg in points.windows(2) {
        push_rough_line(&mut out, seg[0], seg[1], style, rng);
    }
    out
}

fn rough_polygon_stroke(points: &[Point], roughness: f64, bowing: f64, rng: &mut Rng) -> String {
    let mut out = String::new();
    let style = RoughStyle { roughness, bowing };
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        push_rough_line(&mut out, a, b, style, rng);
    }
    out
}

fn rounded_rect_fill_path(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let rad = r.min(w / 2.0).min(h / 2.0);
    format!(
        "M {} {} L {} {} Q {} {} {} {} L {} {} Q {} {} {} {} L {} {} Q {} {} {} {} L {} {} Q {} {} {} {} Z",
        fmt_num(x + rad), fmt_num(y),
        fmt_num(x + w - rad), fmt_num(y),
        fmt_num(x + w), fmt_num(y), fmt_num(x + w), fmt_num(y + rad),
        fmt_num(x + w), fmt_num(y + h - rad),
        fmt_num(x + w), fmt_num(y + h), fmt_num(x + w - rad), fmt_num(y + h),
        fmt_num(x + rad), fmt_num(y + h),
        fmt_num(x), fmt_num(y + h), fmt_num(x), fmt_num(y + h - rad),
        fmt_num(x), fmt_num(y + rad),
        fmt_num(x), fmt_num(y), fmt_num(x + rad), fmt_num(y),
    )
}

fn rough_rounded_rect_stroke(rect: Rect, r: f64, style: RoughStyle, rng: &mut Rng) -> String {
    let Rect {
        x,
        y,
        width: w,
        height: h,
    } = rect;
    let rad = r.min(w / 2.0).min(h / 2.0);
    let mut out = String::new();
    // Four straight edges with hand-drawn double strokes
    push_rough_line(&mut out, Point { x: x + rad, y }, Point { x: x + w - rad, y }, style, rng);
    push_rough_line(&mut out, Point { x: x + w, y: y + rad }, Point { x: x + w, y: y + h - rad }, style, rng);
    push_rough_line(&mut out, Point { x: x + w - rad, y: y + h }, Point { x: x + rad, y: y + h }, style, rng);
    push_rough_line(&mut out, Point { x, y: y + h - rad }, Point { x, y: y + rad }, style, rng);

    // Four rounded corners (drawn twice with subtle jitter to match the double-stroke edges)
    let corners = [
        (x + w - rad, y, x + w, y, x + w, y + rad),
        (x + w, y + h - rad, x + w, y + h, x + w - rad, y + h),
        (x + rad, y + h, x, y + h, x, y + h - rad),
        (x, y + rad, x, y, x + rad, y),
    ];
    for (sx, sy, cx, cy, ex, ey) in corners {
        for pass in 0..2 {
            let j = style.roughness * if pass == 0 { 0.85 } else { 0.55 };
            let _ = write!(
                out,
                "M {} {} Q {} {} {} {} ",
                fmt_num(sx + rng.offset(j)),
                fmt_num(sy + rng.offset(j)),
                fmt_num(cx + rng.offset(j)),
                fmt_num(cy + rng.offset(j)),
                fmt_num(ex + rng.offset(j)),
                fmt_num(ey + rng.offset(j)),
            );
        }
    }
    out
}

fn arrow_wing_points(from: Point, to: Point) -> (Point, Point) {
    let angle = (to.y - from.y).atan2(to.x - from.x);
    let len = 12.0;
    let spread = std::f64::consts::PI / 6.5;
    (
        Point {
            x: to.x - len * (angle - spread).cos(),
            y: to.y - len * (angle - spread).sin(),
        },
        Point {
            x: to.x - len * (angle + spread).cos(),
            y: to.y - len * (angle + spread).sin(),
        },
    )
}

fn escape_xml(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Hand-drawn SVG renderer (pure Rust)
// ---------------------------------------------------------------------------

/// Renders a laid-out diagram to a self-contained hand-drawn Excalidraw-style SVG string.
pub fn render_svg(layout: &LayoutDiagram) -> String {
    let mut svg = String::with_capacity(16 * 1024);
    let w = fmt_num(layout.width);
    let h = fmt_num(layout.height);
    let aria = escape_xml(&layout.diagram.title);

    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"diagram-svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" role=\"img\" aria-label=\"{aria}\">",
    );

    // Background
    let _ = write!(
        svg,
        "<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" rx=\"10\" fill=\"{COLOR_BG}\" class=\"dg-canvas-bg\"/>",
    );

    // Title
    for (i, line) in layout.title_lines.iter().enumerate() {
        let _ = write!(
            svg,
            "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" fill=\"{COLOR_INK}\" font-family=\"&quot;Patrick Hand&quot;, &quot;Be Vietnam Pro&quot;, cursive, sans-serif\" font-size=\"24\" font-weight=\"400\" class=\"dg-title\">{}</text>",
            fmt_num(layout.width / 2.0),
            fmt_num(PAD_TOP + 20.0 + i as f64 * TITLE_LINE_HEIGHT),
            escape_xml(line),
        );
    }

    // Hand-drawn edges + open-V arrowheads
    for edge in &layout.edges {
        let mut rng = Rng::from_key(&format!("edge-{}-{}-{}", edge.index, edge.from, edge.to));
        let path_d = rough_polyline_path(&edge.points, 1.25, 1.0, &mut rng);
        let dash = if edge.is_back_edge {
            " stroke-dasharray=\"7 5\""
        } else {
            ""
        };
        let _ = write!(
            svg,
            "<path d=\"{path_d}\" fill=\"none\" stroke=\"{COLOR_INK}\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}/>",
        );

        if edge.points.len() >= 2 {
            let p1 = edge.points[edge.points.len() - 2];
            let p2 = edge.points[edge.points.len() - 1];
            let (w1, w2) = arrow_wing_points(p1, p2);
            let mut arrow_rng = Rng::from_key(&format!("arrow-{}-{}-{}", edge.index, edge.from, edge.to));
            let arrow_d = rough_polyline_path(&[w1, p2, w2], 0.9, 0.8, &mut arrow_rng);
            let _ = write!(
                svg,
                "<path d=\"{arrow_d}\" fill=\"none\" stroke=\"{COLOR_INK}\" stroke-width=\"1.9\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
            );
        }
    }

    // Hand-drawn nodes (pastel solid fill + sketchy double-ink outline + text)
    for node in &layout.nodes {
        let mut rng = Rng::from_key(&format!("node-{}", node.id));
        let fill = node_fill_color(node.kind);
        let kind_cls = match node.kind {
            NodeKind::Start => "start",
            NodeKind::Step => "step",
            NodeKind::Decision => "decision",
            NodeKind::End => "end",
        };
        let _ = write!(svg, "<g class=\"dg-node dg-node-{kind_cls}\">");

        match node.kind {
            NodeKind::Decision => {
                let cx = node.x + node.width / 2.0;
                let cy = node.y + node.height / 2.0;
                let pts = [
                    Point { x: cx, y: node.y },
                    Point { x: node.x + node.width, y: cy },
                    Point { x: cx, y: node.y + node.height },
                    Point { x: node.x, y: cy },
                ];
                let poly_str = format!(
                    "{},{} {},{} {},{} {},{}",
                    fmt_num(pts[0].x), fmt_num(pts[0].y),
                    fmt_num(pts[1].x), fmt_num(pts[1].y),
                    fmt_num(pts[2].x), fmt_num(pts[2].y),
                    fmt_num(pts[3].x), fmt_num(pts[3].y),
                );
                let stroke_d = rough_polygon_stroke(&pts, 1.65, 1.15, &mut rng);
                let _ = write!(
                    svg,
                    "<polygon points=\"{poly_str}\" fill=\"{fill}\" stroke=\"none\"/>\
                     <path d=\"{stroke_d}\" fill=\"none\" stroke=\"{COLOR_INK}\" stroke-width=\"1.85\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                );
            }
            NodeKind::Start | NodeKind::End => {
                let rad = 28.0_f64.min(node.height / 2.0);
                let fill_d = rounded_rect_fill_path(node.x, node.y, node.width, node.height, rad);
                let stroke_d = rough_rounded_rect_stroke(
                    node.rect(),
                    rad,
                    RoughStyle { roughness: 1.65, bowing: 1.15 },
                    &mut rng,
                );
                let _ = write!(
                    svg,
                    "<path d=\"{fill_d}\" fill=\"{fill}\" stroke=\"none\"/>\
                     <path d=\"{stroke_d}\" fill=\"none\" stroke=\"{COLOR_INK}\" stroke-width=\"1.85\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                );
            }
            NodeKind::Step => {
                let fill_d = rounded_rect_fill_path(node.x, node.y, node.width, node.height, 12.0);
                let stroke_d = rough_rounded_rect_stroke(
                    node.rect(),
                    12.0,
                    RoughStyle { roughness: 1.65, bowing: 1.15 },
                    &mut rng,
                );
                let _ = write!(
                    svg,
                    "<path d=\"{fill_d}\" fill=\"{fill}\" stroke=\"none\"/>\
                     <path d=\"{stroke_d}\" fill=\"none\" stroke=\"{COLOR_INK}\" stroke-width=\"1.85\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                );
            }
        }

        let total_text_h = node.label_lines.len() as f64 * LABEL_LINE_HEIGHT
            + if node.role_lines.is_empty() {
                0.0
            } else {
                node.role_lines.len() as f64 * ROLE_LINE_HEIGHT + 6.0
            };
        let mut text_y = node.y + (node.height - total_text_h) / 2.0;
        let cx = fmt_num(node.x + node.width / 2.0);

        if !node.role_lines.is_empty() {
            for (i, line) in node.role_lines.iter().enumerate() {
                let _ = write!(
                    svg,
                    "<text x=\"{cx}\" y=\"{}\" text-anchor=\"middle\" fill=\"{COLOR_ACCENT}\" font-family=\"&quot;Be Vietnam Pro&quot;, -apple-system, sans-serif\" font-size=\"11\" font-weight=\"600\" class=\"dg-role\">{}</text>",
                    fmt_num(text_y + 12.0 + i as f64 * ROLE_LINE_HEIGHT),
                    escape_xml(line),
                );
            }
            text_y += node.role_lines.len() as f64 * ROLE_LINE_HEIGHT + 6.0;
        }

        for (i, line) in node.label_lines.iter().enumerate() {
            let _ = write!(
                svg,
                "<text x=\"{cx}\" y=\"{}\" text-anchor=\"middle\" fill=\"{COLOR_INK}\" font-family=\"&quot;Patrick Hand&quot;, &quot;Be Vietnam Pro&quot;, cursive, sans-serif\" font-size=\"16.5\" font-weight=\"400\" class=\"dg-label\">{}</text>",
                fmt_num(text_y + 15.0 + i as f64 * LABEL_LINE_HEIGHT),
                escape_xml(line),
            );
        }

        svg.push_str("</g>");
    }

    // Hand-drawn edge label pills on top
    for edge in &layout.edges {
        let Some(box_rect) = edge.label_box else {
            continue;
        };
        if edge.label_lines.is_empty() {
            continue;
        }
        let mut rng = Rng::from_key(&format!("label-{}-{}", edge.index, edge.label));
        let fill_d =
            rounded_rect_fill_path(box_rect.x, box_rect.y, box_rect.width, box_rect.height, 7.0);
        let stroke_d = rough_rounded_rect_stroke(
            box_rect,
            7.0,
            RoughStyle { roughness: 1.1, bowing: 0.9 },
            &mut rng,
        );
        let _ = write!(
            svg,
            "<path d=\"{fill_d}\" fill=\"{COLOR_LABEL_FILL}\" stroke=\"none\"/>\
             <path d=\"{stroke_d}\" fill=\"none\" stroke=\"{COLOR_LABEL_BORDER}\" stroke-width=\"1.15\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
        );

        let cx = fmt_num(box_rect.x + box_rect.width / 2.0);
        for (i, line) in edge.label_lines.iter().enumerate() {
            let _ = write!(
                svg,
                "<text x=\"{cx}\" y=\"{}\" text-anchor=\"middle\" fill=\"{COLOR_INK}\" font-family=\"&quot;Patrick Hand&quot;, &quot;Be Vietnam Pro&quot;, cursive, sans-serif\" font-size=\"14.5\" font-weight=\"400\" class=\"dg-edge-label\">{}</text>",
                fmt_num(box_rect.y + 17.0 + i as f64 * EDGE_LINE_HEIGHT),
                escape_xml(line),
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

// ---------------------------------------------------------------------------
// Editable Excalidraw v2 JSON exporter (pure Rust)
// ---------------------------------------------------------------------------

fn excalidraw_index(seq: usize) -> String {
    // Base-36 padded to 4 chars: a0001, a0002, ...
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut n = seq;
    let mut buf = [b'0'; 4];
    for slot in buf.iter_mut().rev() {
        *slot = DIGITS[n % 36];
        n /= 36;
    }
    format!("a{}", std::str::from_utf8(&buf).unwrap_or("0000"))
}

/// Builds a valid Excalidraw v2 `.excalidraw` JSON string with editable shapes, arrows, and Vietnamese text.
pub fn build_excalidraw_json(layout: &LayoutDiagram) -> String {
    let mut elements: Vec<Value> = Vec::new();
    let mut seq = 1usize;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1_700_000_000_000);

    let mut next_base = |id: String, ty: &str, x: f64, y: f64, width: f64, height: f64| -> Value {
        let seed = 100_000 + seq * 7919;
        let idx = excalidraw_index(seq);
        seq += 1;
        json!({
            "id": id,
            "type": ty,
            "x": x.round() as i64,
            "y": y.round() as i64,
            "width": width.round() as i64,
            "height": height.round() as i64,
            "angle": 0,
            "strokeColor": COLOR_INK,
            "backgroundColor": "transparent",
            "fillStyle": "solid",
            "strokeWidth": 2,
            "strokeStyle": "solid",
            "roughness": 2,
            "opacity": 100,
            "groupIds": [],
            "frameId": null,
            "index": idx,
            "roundness": null,
            "seed": seed,
            "version": 1,
            "versionNonce": seed + 17,
            "isDeleted": false,
            "boundElements": [],
            "updated": now,
            "link": null,
            "locked": false
        })
    };

    // Title
    let title_text = layout.title_lines.join("\n");
    let title_w = (max_line_width(&layout.title_lines, TITLE_FONT_SIZE, 600) + 16.0)
        .ceil()
        .max(240.0);
    let title_h = layout.title_lines.len() as f64 * TITLE_LINE_HEIGHT;
    let mut title_el = next_base(
        "dg-title".into(),
        "text",
        (layout.width - title_w) / 2.0,
        PAD_TOP,
        title_w,
        title_h,
    );
    if let Some(obj) = title_el.as_object_mut() {
        obj.insert("text".into(), json!(title_text));
        obj.insert("originalText".into(), json!(title_text));
        obj.insert("fontSize".into(), json!(20));
        obj.insert("fontFamily".into(), json!(5));
        obj.insert("textAlign".into(), json!("center"));
        obj.insert("verticalAlign".into(), json!("top"));
        obj.insert("containerId".into(), Value::Null);
        obj.insert("autoResize".into(), json!(true));
        obj.insert("lineHeight".into(), json!(1.25));
    }
    elements.push(title_el);

    // Pre-collect bound arrows per node id so shape elements can embed them directly.
    let mut bound_arrows: HashMap<&str, Vec<Value>> = HashMap::new();
    for edge in &layout.edges {
        let arrow_id = format!("edge-{}", edge.index);
        bound_arrows
            .entry(edge.from.as_str())
            .or_default()
            .push(json!({ "id": arrow_id, "type": "arrow" }));
        bound_arrows
            .entry(edge.to.as_str())
            .or_default()
            .push(json!({ "id": arrow_id, "type": "arrow" }));
    }

    for node in &layout.nodes {
        let shape_id = format!("node-{}", node.id);
        let text_id = format!("text-{}", node.id);
        let ex_type = match node.kind {
            NodeKind::Decision => "diamond",
            NodeKind::Start | NodeKind::End => "ellipse",
            NodeKind::Step => "rectangle",
        };
        let mut bound = vec![json!({ "id": text_id, "type": "text" })];
        if let Some(arrows) = bound_arrows.get(node.id.as_str()) {
            bound.extend(arrows.iter().cloned());
        }

        let mut shape = next_base(
            shape_id.clone(),
            ex_type,
            node.x,
            node.y,
            node.width,
            node.height,
        );
        if let Some(obj) = shape.as_object_mut() {
            obj.insert("strokeColor".into(), json!(COLOR_INK));
            obj.insert("backgroundColor".into(), json!(node_fill_color(node.kind)));
            if ex_type == "rectangle" {
                obj.insert("roundness".into(), json!({ "type": 3 }));
            }
            obj.insert("boundElements".into(), json!(bound));
        }
        elements.push(shape);

        let mut combined_lines: Vec<String> = Vec::new();
        if !node.role_lines.is_empty() {
            combined_lines.push(format!("[{}]", node.role_lines.join(" ")));
        }
        combined_lines.extend(node.label_lines.iter().cloned());
        let body_text = combined_lines.join("\n");
        let text_w = (max_line_width(&combined_lines, 15.0, 500) + 8.0)
            .ceil()
            .min(node.width - 24.0)
            .max(80.0);
        let text_h = combined_lines.len() as f64 * 20.0;

        let mut text_el = next_base(
            text_id,
            "text",
            node.x + (node.width - text_w) / 2.0,
            node.y + (node.height - text_h) / 2.0,
            text_w,
            text_h,
        );
        if let Some(obj) = text_el.as_object_mut() {
            obj.insert("text".into(), json!(body_text));
            obj.insert("originalText".into(), json!(body_text));
            obj.insert("fontSize".into(), json!(15));
            obj.insert("fontFamily".into(), json!(5));
            obj.insert("textAlign".into(), json!("center"));
            obj.insert("verticalAlign".into(), json!("middle"));
            obj.insert("containerId".into(), json!(shape_id));
            obj.insert("autoResize".into(), json!(true));
            obj.insert("lineHeight".into(), json!(1.25));
        }
        elements.push(text_el);
    }

    for edge in &layout.edges {
        let Some(&start) = edge.points.first() else {
            continue;
        };
        let arrow_id = format!("edge-{}", edge.index);
        let rel_points: Vec<[i64; 2]> = edge
            .points
            .iter()
            .map(|p| [(p.x - start.x).round() as i64, (p.y - start.y).round() as i64])
            .collect();
        let (mut min_dx, mut max_dx, mut min_dy, mut max_dy) = (0i64, 0i64, 0i64, 0i64);
        for [dx, dy] in &rel_points {
            min_dx = min_dx.min(*dx);
            max_dx = max_dx.max(*dx);
            min_dy = min_dy.min(*dy);
            max_dy = max_dy.max(*dy);
        }

        let label_id = if edge.label_lines.is_empty() {
            None
        } else {
            Some(format!("edge-label-{}", edge.index))
        };

        let mut arrow = next_base(
            arrow_id.clone(),
            "arrow",
            start.x,
            start.y,
            (max_dx - min_dx).max(1) as f64,
            (max_dy - min_dy).max(1) as f64,
        );
        if let Some(obj) = arrow.as_object_mut() {
            obj.insert("strokeColor".into(), json!("#475569"));
            obj.insert(
                "strokeStyle".into(),
                json!(if edge.is_back_edge { "dashed" } else { "solid" }),
            );
            obj.insert("points".into(), json!(rel_points));
            obj.insert("lastCommittedPoint".into(), Value::Null);
            obj.insert(
                "startBinding".into(),
                json!({ "elementId": format!("node-{}", edge.from), "focus": 0, "gap": 4 }),
            );
            obj.insert(
                "endBinding".into(),
                json!({ "elementId": format!("node-{}", edge.to), "focus": 0, "gap": 4 }),
            );
            obj.insert("startArrowhead".into(), Value::Null);
            obj.insert("endArrowhead".into(), json!("arrow"));
            obj.insert(
                "boundElements".into(),
                match &label_id {
                    Some(lid) => json!([{ "id": lid, "type": "text" }]),
                    None => json!([]),
                },
            );
        }
        elements.push(arrow);

        if let (Some(lid), Some(lb)) = (label_id, edge.label_box) {
            let label_text = edge.label_lines.join("\n");
            let mut label_el = next_base(
                lid,
                "text",
                lb.x + 6.0,
                lb.y + 4.0,
                (lb.width - 12.0).max(40.0),
                (lb.height - 8.0).max(20.0),
            );
            if let Some(obj) = label_el.as_object_mut() {
                obj.insert("strokeColor".into(), json!("#334155"));
                obj.insert("text".into(), json!(label_text));
                obj.insert("originalText".into(), json!(label_text));
                obj.insert("fontSize".into(), json!(13));
                obj.insert("fontFamily".into(), json!(5));
                obj.insert("textAlign".into(), json!("center"));
                obj.insert("verticalAlign".into(), json!("middle"));
                obj.insert("containerId".into(), json!(arrow_id));
                obj.insert("autoResize".into(), json!(true));
                obj.insert("lineHeight".into(), json!(1.25));
            }
            elements.push(label_el);
        }
    }

    serde_json::to_string_pretty(&json!({
        "type": "excalidraw",
        "version": 2,
        "source": "https://excalidraw.com",
        "elements": elements,
        "appState": {
            "gridSize": null,
            "viewBackgroundColor": COLOR_BG
        },
        "files": {}
    }))
    .unwrap_or_else(|_| "{}".into())
}

fn wrap_uncertainty_line(prefix: &str, text: &str, max_chars: usize) -> Vec<String> {
    let full = format!("{prefix}{text}");
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in full.split_whitespace() {
        let cur_len = current.chars().count();
        let word_len = word.chars().count();
        if current.is_empty() {
            current.push_str(word);
        } else if cur_len + 1 + word_len <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = format!("   {word}");
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn append_uncertainty_footer_card(
    diagram: &ProcessDiagram,
    icons: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<Vec<serde_json::Value>> {
    let mode = diagram.uncertainty_mode.as_deref()?;
    if !matches!(mode, "footer" | "badges-footer") || diagram.uncertainties.is_empty() {
        return None;
    }
    let (min_x, _min_y, max_x, max_y) = crate::ve_engine::spec_bounds(&diagram.ve_elements, icons).ok()?;
    let card_w = (max_x - min_x).clamp(520.0, 920.0);
    let max_chars = ((card_w - 36.0) / 8.6).floor().max(48.0) as usize;
    let numbered = mode == "badges-footer";
    let mut body_lines = Vec::new();
    for (idx, u) in diagram.uncertainties.iter().enumerate() {
        let prefix = if numbered {
            format!("?{}. ", idx + 1)
        } else {
            "• ".to_owned()
        };
        let content = if u.reason.is_empty() {
            u.item.clone()
        } else if u.item.is_empty() {
            u.reason.clone()
        } else {
            format!("{} — {}", u.item, u.reason)
        };
        body_lines.extend(wrap_uncertainty_line(&prefix, &content, max_chars));
    }
    let body = body_lines.join("\n");
    let mut els = diagram.ve_elements.clone();
    els.push(json!({
        "t": "card",
        "id": "uncertainties_footer",
        "x": min_x,
        "y": max_y + 28.0,
        "w": card_w,
        "title": format!("Cần xác nhận ({})", diagram.uncertainties.len()),
        "text": body,
        "size": 15.0,
        "color": "warn",
        "fill": "warn",
        "dashed": true,
        "textColor": "ink"
    }));
    Some(els)
}

/// One-shot entry point for the UI: dispatches rich `ve_elements` to `ve_engine` or falls back to
/// the layered flowchart layout + hand-drawn SVG + Excalidraw v2 JSON renderer.
pub fn render_diagram(diagram: &ProcessDiagram) -> Result<RenderedDiagram, String> {
    if !diagram.ve_elements.is_empty() {
        let icons = crate::ve_engine::builtin_icons();
        let augmented = append_uncertainty_footer_card(diagram, icons);
        let spec_els = augmented.as_deref().unwrap_or(&diagram.ve_elements);
        let out = crate::ve_engine::render_spec_elements(spec_els, icons)?;
        return Ok(RenderedDiagram {
            width: out.width,
            height: out.height,
            svg: out.svg,
            excalidraw_json: out.excalidraw_json,
            drawio_xml: out.drawio_xml,
        });
    }
    let layout = layout_diagram(diagram)?;
    let svg = render_svg(&layout);
    let excalidraw_json = build_excalidraw_json(&layout);
    let drawio_xml = build_legacy_drawio_xml(&layout);
    Ok(RenderedDiagram {
        width: layout.width,
        height: layout.height,
        svg,
        excalidraw_json,
        drawio_xml,
    })
}

fn build_legacy_drawio_xml(layout: &LayoutDiagram) -> String {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mxfile host=\"A-Sir\"><diagram id=\"d1\" name=\"Diagram\"><mxGraphModel pageWidth=\"{}\" pageHeight=\"{}\"><root><mxCell id=\"0\"/><mxCell id=\"1\" parent=\"0\"/>",
        layout.width.ceil() as i64,
        layout.height.ceil() as i64,
    );
    for (i, n) in layout.nodes.iter().enumerate() {
        let shape = match n.kind {
            NodeKind::Decision => "rhombus;whiteSpace=wrap;",
            NodeKind::Start | NodeKind::End => "rounded=1;arcSize=50;whiteSpace=wrap;",
            NodeKind::Step => "rounded=1;whiteSpace=wrap;",
        };
        let fill = node_fill_color(n.kind);
        let label = n.label.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        xml.push_str(&format!(
            "<mxCell id=\"n{i}\" value=\"{label}\" style=\"{shape}fillColor={fill};strokeColor=#1D2939;\" vertex=\"1\" parent=\"1\"><mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\"/></mxCell>",
            n.x.round(),
            n.y.round(),
            n.width.round(),
            n.height.round(),
        ));
    }
    for (i, e) in layout.edges.iter().enumerate() {
        if e.points.len() < 2 {
            continue;
        }
        let p0 = e.points[0];
        let pn = e.points[e.points.len() - 1];
        let label = e.label.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        xml.push_str(&format!(
            "<mxCell id=\"e{i}\" value=\"{label}\" style=\"endArrow=classic;rounded=1;strokeColor=#475467;\" edge=\"1\" parent=\"1\"><mxGeometry relative=\"1\" as=\"geometry\"><mxPoint x=\"{}\" y=\"{}\" as=\"sourcePoint\"/><mxPoint x=\"{}\" y=\"{}\" as=\"targetPoint\"/></mxGeometry></mxCell>",
            p0.x.round(),
            p0.y.round(),
            pn.x.round(),
            pn.y.round(),
        ));
    }
    xml.push_str("</root></mxGraphModel></diagram></mxfile>");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::DiagramEdge;

    fn node(id: &str, kind: NodeKind, label: &str, role: &str) -> DiagramNode {
        DiagramNode {
            id: id.into(),
            kind,
            label: label.into(),
            role: role.into(),
        }
    }

    fn edge(from: &str, to: &str, label: &str) -> DiagramEdge {
        DiagramEdge {
            from: from.into(),
            to: to.into(),
            label: label.into(),
        }
    }

    /// Strict geometric invariant: zero overlaps between any nodes, any label and any node, or any two labels.
    fn assert_zero_overlaps(layout: &LayoutDiagram) {
        // 1. No two nodes overlap
        for i in 0..layout.nodes.len() {
            for j in (i + 1)..layout.nodes.len() {
                let a = layout.nodes[i].rect();
                let b = layout.nodes[j].rect();
                assert!(
                    !a.intersects(b),
                    "Nodes {} and {} overlap: {:?} vs {:?}",
                    layout.nodes[i].id,
                    layout.nodes[j].id,
                    a,
                    b
                );
            }
        }

        // 2. No edge label overlaps any node (rect or diamond)
        for e in &layout.edges {
            let Some(lb) = e.label_box else { continue };
            for n in &layout.nodes {
                assert!(
                    !rect_intersects_node(lb, n, 4.0),
                    "Edge label '{}' ({}->{}) overlaps node '{}' ({:?} vs {:?})",
                    e.label,
                    e.from,
                    e.to,
                    n.id,
                    lb,
                    n.rect()
                );
            }
        }

        // 3. No two edge labels overlap each other
        for i in 0..layout.edges.len() {
            let Some(a) = layout.edges[i].label_box else {
                continue;
            };
            for j in (i + 1)..layout.edges.len() {
                let Some(b) = layout.edges[j].label_box else {
                    continue;
                };
                assert!(
                    !a.intersects(b.padded(4.0)),
                    "Edge labels '{}' and '{}' overlap: {:?} vs {:?}",
                    layout.edges[i].label,
                    layout.edges[j].label,
                    a,
                    b
                );
            }
        }
    }

    fn news_feed_diagram() -> ProcessDiagram {
        ProcessDiagram {
            title: "Kiến trúc News Feed (Đường ghi & Đường đọc)".into(),
            summary: "So sánh đường ghi (fan-out) và đường đọc (fan-in tác giả lớn).".into(),
            nodes: vec![
                node("w1", NodeKind::Start, "POST /posts", "Client (Đường ghi)"),
                node("w2", NodeKind::Step, "Lưu bài viết vào Posts DB", "Post Service"),
                node("w3", NodeKind::Step, "Đẩy sự kiện PostCreated", "Kafka"),
                node("w4", NodeKind::Decision, "Tác giả > 1 triệu follower?", "Fan-out Worker"),
                node("w5", NodeKind::End, "Chèn post_id vào Redis (tối đa 800 / user)", "Feed Cache (Redis)"),
                node("w6", NodeKind::End, "Bỏ qua fan-out (chờ kéo lúc đọc)", "Fan-out Worker"),
                node("r1", NodeKind::Start, "GET /feed", "Client (Đường đọc)"),
                node("r2", NodeKind::Step, "Lấy danh sách post_id từ Redis", "Feed Service"),
                node("r3", NodeKind::Decision, "Có theo dõi tác giả > 1 triệu follower?", "Feed Service"),
                node("r4", NodeKind::Step, "Kéo bài viết mới từ Posts DB", "Posts DB"),
                node("r5", NodeKind::End, "Trộn & trả danh sách News Feed", "Feed Service"),
            ],
            edges: vec![
                edge("w1", "w2", ""),
                edge("w2", "w3", "PostCreated"),
                edge("w3", "w4", ""),
                edge("w4", "w5", "<= 1 triệu follower"),
                edge("w4", "w6", "> 1 triệu follower"),
                edge("r1", "r2", ""),
                edge("r2", "r3", ""),
                edge("r3", "r4", "Có tác giả lớn"),
                edge("r3", "r5", "Không có"),
                edge("r4", "r5", "Trộn với cache"),
            ],
            uncertainties: vec![],
            ve_elements: vec![],
            provider: "Rust".into(),
            model: "local".into(),
            strategy: None,
            uncertainty_mode: None,
        }
    }

    #[test]
    fn news_feed_has_zero_overlaps_and_clean_bypass_lane() {
        let diagram = news_feed_diagram();
        let layout = layout_diagram(&diagram).unwrap();
        assert_zero_overlaps(&layout);

        // Verify the bypass edge r3 -> r5 does not intersect intermediate node r4 (Posts DB)
        let bypass = layout.edges.iter().find(|e| e.from == "r3" && e.to == "r5").unwrap();
        let posts_db = layout.nodes.iter().find(|n| n.id == "r4").unwrap();
        for seg in bypass.points.windows(2) {
            assert!(
                !ortho_segment_intersects_rect(seg[0], seg[1], posts_db.rect().padded(16.0)),
                "Bypass edge r3->r5 should stay well clear of Posts DB: {:?} vs {:?}",
                seg,
                posts_db.rect()
            );
        }
    }

    #[test]
    fn payment_approval_with_cycle_and_diamond_has_zero_overlaps() {
        let diagram = ProcessDiagram {
            title: "Quy trình duyệt thanh toán".into(),
            summary: String::new(),
            nodes: vec![
                node("n1", NodeKind::Start, "Tiếp nhận hồ sơ", "Nhân viên"),
                node("n2", NodeKind::Decision, "Đủ hoá đơn?", "Kế toán"),
                node("n3", NodeKind::Step, "Trả lại hồ sơ bổ sung", "Kế toán"),
                node("n4", NodeKind::Decision, "User thanh toán trong 5 đến 10 phút?", "User"),
                node("n5", NodeKind::End, "Đơn coi như bị huỷ", ""),
                node("n6", NodeKind::Step, "Thanh toán thành công, bắt đầu giao hàng (ship)", ""),
            ],
            edges: vec![
                edge("n1", "n2", ""),
                edge("n2", "n3", "Thiếu hoá đơn"),
                edge("n3", "n1", "Nộp lại hồ sơ"),
                edge("n2", "n4", "Đủ hoá đơn"),
                edge("n4", "n5", "Quá hạn"),
                edge("n4", "n6", "Thanh toán thành công"),
            ],
            uncertainties: vec![],
            ve_elements: vec![],
            provider: String::new(),
            model: String::new(),
            strategy: None,
            uncertainty_mode: None,
        };
        let layout = layout_diagram(&diagram).unwrap();
        assert_zero_overlaps(&layout);
    }

    #[test]
    fn dense_four_way_decision_fanout_has_zero_label_overlaps() {
        let diagram = ProcessDiagram {
            title: "Phân loại yêu cầu hỗ trợ".into(),
            summary: String::new(),
            nodes: vec![
                node("s", NodeKind::Start, "Nhận ticket", "CSKH"),
                node("d", NodeKind::Decision, "Mức độ ưu tiên của ticket?", "Hệ thống"),
                node("a", NodeKind::End, "Xử lý ngay trong 15 phút", "On-call"),
                node("b", NodeKind::End, "Chuyển đội kỹ thuật cấp 2", "Tier 2"),
                node("c", NodeKind::End, "Trả lời theo mẫu hướng dẫn", "CSKH"),
                node("e", NodeKind::End, "Đóng ticket trùng lặp", "Bot"),
            ],
            edges: vec![
                edge("s", "d", ""),
                edge("d", "a", "Khẩn cấp P0"),
                edge("d", "b", "Lỗi hệ thống P1"),
                edge("d", "c", "Hỏi đáp thông thường P2"),
                edge("d", "e", "Trùng lặp / Spam"),
            ],
            uncertainties: vec![],
            ve_elements: vec![],
            provider: String::new(),
            model: String::new(),
            strategy: None,
            uncertainty_mode: None,
        };
        let layout = layout_diagram(&diagram).unwrap();
        assert_zero_overlaps(&layout);
    }

    #[test]
    fn render_diagram_outputs_valid_svg_and_excalidraw_v2() {
        let diagram = news_feed_diagram();
        let rendered = render_diagram(&diagram).unwrap();
        if let Ok(path) = std::env::var("ASR_DUMP_DIAGRAM_SVG") {
            let _ = std::fs::write(path, &rendered.svg);
        }
        assert!(rendered.width >= 560.0);
        assert!(rendered.height >= 260.0);
        assert!(rendered.svg.starts_with("<svg "));
        assert!(rendered.svg.ends_with("</svg>"));
        assert!(rendered.svg.contains("Kiến trúc News Feed"));
        assert!(rendered.svg.contains("Không có"));

        let parsed: Value = serde_json::from_str(&rendered.excalidraw_json).unwrap();
        assert_eq!(parsed["type"], "excalidraw");
        assert_eq!(parsed["version"], 2);
        assert!(parsed["elements"].as_array().unwrap().len() > 20);
    }
}
