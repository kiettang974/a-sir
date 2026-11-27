//! Pure-Rust port of `drawing-excalidraw-figures` (`scripts/ve.py` + `engine/theme.py`).
//!
//! Compiles a concise JSON figure spec (supporting all 35 macros: `box`, `ellipse`, `diamond`,
//! `arrow`, `line`, `text`, `frame`, `cells`, `grid`, `table`, `entity`, `code`, `axis`, `span`,
//! `plot`, `sequence`, `tree`, `ring`, `lanes`, `swimlane`, `start`, `end`, `stack`, `cross`,
//! `check`, `badge`, `note`, `legend`, `db`, `queue`, `pipe`, `steps`, `card`, `person`, `cloud`,
//! `doc`, `window`, `group`, `row`, `col`, `icon`, plus `extends`/`dimBase`/`remove`) into:
//! - Hand-drawn Excalidraw-style SVG (Rough.js double-stroke curves, solid/hachure/cross-hatch
//!   fills, ER crow's-foot + UML arrowheads, arrow label masks, Patrick Hand + IBM Plex Mono)
//! - Valid `.excalidraw` v2 JSON
//! - Full geometric lint diagnostics (`errors` and `reminders`)

use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// Theme palette (1:1 port of engine/theme.py)
// ---------------------------------------------------------------------------

const CANH_NEN: &str = "#0B1021";
const CANH_CHU: &str = "#DDE7F7";
const PRE_BG_LIGHT: &str = "#111A2B";
const PRE_FG_LIGHT: &str = "#DCE6F5";
const LIT_DARK: &str = "#24272D";
const LIT_LIGHT: &str = "#F4EFE4";
const HACH_HEP: f64 = 30.0;

fn token_light(name: &str) -> Option<&'static str> {
    match name {
        "paper" => Some("#EDF0F4"),
        "card" => Some("#FFFFFF"),
        "ink" => Some("#111A2B"),
        "muted" => Some("#5C6B82"),
        "line" => Some("#C9D4E3"),
        "fill-soft" => Some("#F2F6FB"),
        "l2" => Some("#64748B"),
        "l3" => Some("#008F79"),
        "l4" => Some("#1E5AE8"),
        "l6" => Some("#8F74BE"),
        "l7" => Some("#4D3399"),
        "err" => Some("#D6294B"),
        "warn" => Some("#E8930C"),
        "node" => Some("#14621C"),
        "heap" => Some("#96470C"),
        "hot" => Some("#BE1D72"),
        _ => None,
    }
}

fn fill_light(name: &str) -> Option<&'static str> {
    match name {
        "soft" | "fill-soft" => Some("#F2F6FB"),
        "l2" | "fill-l2" => Some("#D4D8DF"),
        "l3" | "fill-l3" => Some("#A0F2E3"),
        "l4" | "fill-l4" => Some("#C0D1F9"),
        "l6" | "fill-l6" => Some("#E7D5FF"),
        "l7" | "fill-l7" => Some("#EDDAFF"),
        "err" | "fill-err" => Some("#F4C3CD"),
        "warn" | "fill-warn" => Some("#F9E1BB"),
        "node" | "fill-node" => Some("#C5EDBF"),
        "heap" | "fill-heap" => Some("#F5E3C6"),
        "hot" | "fill-hot" => Some("#F9D8EA"),
        _ => None,
    }
}

fn is_accent(name: &str) -> bool {
    matches!(
        name,
        "l3" | "l4" | "l6" | "l7" | "err" | "warn" | "node" | "heap" | "hot"
    )
}

fn resolve_color(v: &str) -> Result<String, String> {
    match v {
        "code-fg" => Ok(CANH_CHU.into()),
        "lit-dark" => Ok(LIT_DARK.into()),
        "lit-light" => Ok(LIT_LIGHT.into()),
        other => token_light(other)
            .map(str::to_owned)
            .ok_or_else(|| format!("color '{other}' is not a valid token")),
    }
}

fn resolve_fill(v: Option<&str>) -> Result<String, String> {
    let Some(v) = v else {
        return Ok("transparent".into());
    };
    if v.is_empty() {
        return Ok("transparent".into());
    }
    match v {
        "code" => Ok(CANH_NEN.into()),
        "ink" => Ok(token_light("ink").unwrap().into()),
        "lit-dark" => Ok(LIT_DARK.into()),
        "lit-light" => Ok(LIT_LIGHT.into()),
        other => fill_light(other)
            .map(str::to_owned)
            .ok_or_else(|| format!("fill '{other}' is not a valid fill token")),
    }
}

fn is_shape_t(t: &str) -> bool {
    matches!(t, "box" | "ellipse" | "diamond")
}

fn resolve_head(name: &str) -> Result<Option<&'static str>, String> {
    match name {
        "arrow" => Ok(Some("arrow")),
        "triangle" => Ok(Some("triangle")),
        "dot" => Ok(Some("circle")),
        "bar" => Ok(Some("bar")),
        "none" => Ok(None),
        "one" => Ok(Some("crowfoot_one")),
        "many" => Ok(Some("crowfoot_many")),
        "one-many" => Ok(Some("crowfoot_one_or_many")),
        "zero-one" => Ok(Some("crowfoot_one")),
        "zero-many" => Ok(Some("crowfoot_many")),
        "inherit" => Ok(Some("triangle_outline")),
        "compose" => Ok(Some("diamond")),
        "aggregate" => Ok(Some("diamond_outline")),
        "circle" => Ok(Some("circle_outline")),
        other => Err(format!("unknown arrow head/tail '{other}'")),
    }
}

// ---------------------------------------------------------------------------
// Public output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VeOutput {
    pub width: f64,
    pub height: f64,
    pub element_count: usize,
    pub svg: String,
    pub excalidraw_json: String,
    pub errors: Vec<String>,
    pub reminders: Vec<String>,
}

// ---------------------------------------------------------------------------
// JSON helper accessors
// ---------------------------------------------------------------------------

fn f64_at(v: &Value, k: &str, default: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(default)
}

fn req_f64(v: &Value, k: &str, ctx: &str) -> Result<f64, String> {
    v.get(k)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{ctx}: missing numeric property '{k}'"))
}

fn str_at<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn bool_at(v: &Value, k: &str, default: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(default)
}

fn opt_val_str(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Text sizing & geometry helpers (1:1 port of ve.py)
// ---------------------------------------------------------------------------

fn rong_chu(s: &str, size: f64) -> f64 {
    s.split('\n')
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0) as f64
        * size
        * 0.52
}

fn cao_chu(s: &str, size: f64) -> f64 {
    (s.matches('\n').count() + 1) as f64 * size * 1.25
}

fn tam(x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
    (x + w / 2.0, y + h / 2.0)
}

fn mep(x: f64, y: f64, w: f64, h: f64, side: &str, at: f64) -> Result<(f64, f64), String> {
    match side {
        "top" => Ok((x + w * at, y)),
        "bottom" => Ok((x + w * at, y + h)),
        "left" => Ok((x, y + h * at)),
        "right" => Ok((x + w, y + h * at)),
        other => Err(format!("unknown side '{other}' (expected top/bottom/left/right)")),
    }
}

fn cat(x: f64, y: f64, w: f64, h: f64, shape: &str, tx: f64, ty: f64) -> (f64, f64) {
    let (cx, cy) = tam(x, y, w, h);
    let dx = tx - cx;
    let dy = ty - cy;
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return (cx, cy);
    }
    let hw = (w / 2.0).max(1e-6);
    let hh = (h / 2.0).max(1e-6);
    let k = match shape {
        "ellipse" => 1.0 / (dx / hw).hypot(dy / hh),
        "diamond" => 1.0 / (dx.abs() / hw + dy.abs() / hh),
        _ => {
            let kx = if dx.abs() > 1e-9 { hw / dx.abs() } else { 1e9 };
            let ky = if dy.abs() > 1e-9 { hh / dy.abs() } else { 1e9 };
            kx.min(ky)
        }
    };
    (cx + dx * k, cy + dy * k)
}

fn lui(p: (f64, f64), q: (f64, f64), d: f64) -> (f64, f64) {
    let l = (q.0 - p.0).hypot(q.1 - p.1);
    let l = if l < 1e-9 { 1.0 } else { l };
    (p.0 + (q.0 - p.0) * d / l, p.1 + (q.1 - p.1) * d / l)
}

// ---------------------------------------------------------------------------
// Spec loader with `extends`, `dimBase`, `remove`
// ---------------------------------------------------------------------------

pub fn load_spec_file(path: &Path) -> Result<Vec<Value>, String> {
    let mut seen = HashSet::new();
    nap_path(path, &mut seen)
}

fn nap_path(path: &Path, seen: &mut HashSet<PathBuf>) -> Result<Vec<Value>, String> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canon.clone()) {
        return Err(format!("circular extends at {}", canon.display()));
    }
    let text = std::fs::read_to_string(&canon)
        .map_err(|e| format!("cannot read spec {}: {e}", canon.display()))?;
    let spec: Value = serde_json::from_str(&text)
        .map_err(|e| format!("invalid JSON in {}: {e}", canon.display()))?;
    let mut els = spec
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if let Some(ext) = spec.get("extends").and_then(Value::as_str) {
        let base_path = canon.parent().unwrap_or_else(|| Path::new(".")).join(ext);
        let goc = nap_path(&base_path, seen)?;
        let remove_ids: HashSet<String> = spec
            .get("remove")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut base: Vec<Value> = goc
            .into_iter()
            .filter(|x| {
                x.get("id")
                    .and_then(Value::as_str)
                    .map(|id| !remove_ids.contains(id))
                    .unwrap_or(true)
            })
            .collect();
        if bool_at(&spec, "dimBase", true) {
            base = vec![json!({
                "t": "group",
                "dim": true,
                "elements": base
            })];
        }
        base.extend(els);
        els = base;
    }
    Ok(els)
}

// ---------------------------------------------------------------------------
// Macro expansion (all 35 macros from scripts/ve.py)
// ---------------------------------------------------------------------------

fn ghost_box(x: f64, y: f64, w: f64, h: f64, text: &str, size: f64, color: &str) -> Value {
    json!({
        "t": "box",
        "ghost": true,
        "x": x,
        "y": y,
        "w": w,
        "h": h,
        "text": text,
        "size": size,
        "color": color
    })
}

fn expand_all(els: &[Value], icons: &HashMap<String, Value>) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for (i, e) in els.iter().enumerate() {
        let t = str_at(e, "t").unwrap_or("");
        match t {
            "box" | "ellipse" | "diamond" | "arrow" | "line" | "text" | "anchor" | "raw" => {
                out.push(e.clone());
            }
            "frame" => out.extend(expand_all(&m_frame(e)?, icons)?),
            "cells" => out.extend(expand_all(&m_cells(e)?, icons)?),
            "grid" => out.extend(expand_all(&m_grid(e)?, icons)?),
            "table" => out.extend(expand_all(&m_table(e)?, icons)?),
            "entity" => out.extend(expand_all(&m_entity(e)?, icons)?),
            "code" => out.extend(expand_all(&m_code(e)?, icons)?),
            "axis" => out.extend(expand_all(&m_axis(e)?, icons)?),
            "span" => out.extend(expand_all(&m_span(e)?, icons)?),
            "plot" => out.extend(expand_all(&m_plot(e)?, icons)?),
            "sequence" => out.extend(expand_all(&m_sequence(e)?, icons)?),
            "tree" => out.extend(expand_all(&m_tree(e)?, icons)?),
            "ring" => out.extend(expand_all(&m_ring(e)?, icons)?),
            "lanes" => out.extend(expand_all(&m_lanes(e)?, icons)?),
            "swimlane" => out.extend(expand_all(&m_swimlane(e)?, icons)?),
            "start" => out.extend(expand_all(&m_start(e)?, icons)?),
            "end" => out.extend(expand_all(&m_end(e)?, icons)?),
            "stack" => out.extend(expand_all(&m_stack(e)?, icons)?),
            "cross" => out.extend(expand_all(&m_cross(e)?, icons)?),
            "check" => out.extend(expand_all(&m_check(e)?, icons)?),
            "badge" => out.extend(expand_all(&m_badge(e)?, icons)?),
            "note" => out.extend(expand_all(&m_note(e)?, icons)?),
            "legend" => out.extend(expand_all(&m_legend(e)?, icons)?),
            "db" => out.extend(expand_all(&m_db(e)?, icons)?),
            "pipe" => out.extend(expand_all(&m_pipe(e)?, icons)?),
            "steps" => out.extend(expand_all(&m_steps(e)?, icons)?),
            "card" => out.extend(expand_all(&m_card(e)?, icons)?),
            "queue" => out.extend(expand_all(&m_queue(e)?, icons)?),
            "person" => out.extend(expand_all(&m_person(e)?, icons)?),
            "cloud" => out.extend(expand_all(&m_cloud(e)?, icons)?),
            "doc" => out.extend(expand_all(&m_doc(e)?, icons)?),
            "window" => out.extend(expand_all(&m_window(e)?, icons)?),
            "group" => out.extend(m_group(e, icons)?),
            "row" | "col" => out.extend(m_row_col(e, icons)?),
            "icon" => out.extend(expand_all(&m_icon(e, icons)?, icons)?),
            other => return Err(format!("element {i}: unsupported t='{other}'")),
        }
    }
    Ok(out)
}

fn m_frame(e: &Value) -> Result<Vec<Value>, String> {
    let mut b = e.clone();
    if let Some(obj) = b.as_object_mut() {
        obj.insert("t".into(), json!("box"));
        obj.remove("text");
        if !obj.contains_key("sharp") {
            obj.insert("sharp".into(), json!(false));
        }
    }
    let mut out = vec![b];
    if let Some(title) = str_at(e, "title") {
        let x = req_f64(e, "x", "frame")?;
        let y = req_f64(e, "y", "frame")?;
        let s = f64_at(e, "size", 16.0);
        let tc = str_at(e, "titleColor")
            .or_else(|| str_at(e, "color"))
            .unwrap_or("muted");
        out.push(json!({
            "t": "text", "x": x + 12.0, "y": y + 8.0, "text": title, "size": s, "color": tc
        }));
    }
    Ok(out)
}

fn m_cells(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("cells: missing 'id'")?;
    let x = req_f64(e, "x", "cells")?;
    let y = req_f64(e, "y", "cells")?;
    let cw = req_f64(e, "cw", "cells")?;
    let ch = req_f64(e, "ch", "cells")?;
    let col = str_at(e, "dir") == Some("col");
    let labs = e
        .get("labels")
        .and_then(Value::as_array)
        .ok_or("cells: missing 'labels'")?;
    let ws: Option<Vec<f64>> = e.get("widths").and_then(Value::as_array).map(|arr| {
        arr.iter().filter_map(Value::as_f64).collect()
    });
    if let Some(ref wv) = ws {
        if wv.len() != labs.len() {
            return Err(format!(
                "cells '{id}': widths has {} items, labels has {}",
                wv.len(),
                labs.len()
            ));
        }
    }
    let marks = e.get("marks").and_then(Value::as_object);
    let size = f64_at(e, "size", 18.0);
    let mut out = Vec::new();
    let mut chay = 0.0;

    for (i, lab_v) in labs.iter().enumerate() {
        let lab = lab_v.as_str().unwrap_or("");
        let m = marks.and_then(|mk| mk.get(&i.to_string()));
        let buoc = ws
            .as_ref()
            .map(|wv| wv[i])
            .unwrap_or(if col { ch } else { cw });
        let bx = x + if col { 0.0 } else { chay };
        let by = y + if col { chay } else { 0.0 };
        let bw = if col { cw } else { buoc };
        let bh = if col { buoc } else { ch };
        let mut b = json!({
            "t": "box",
            "id": format!("{id}.{i}"),
            "sharp": true,
            "x": bx,
            "y": by,
            "w": bw,
            "h": bh,
            "text": lab,
            "size": size
        });
        for k in ["color", "fill", "fillStyle", "weight", "dashed"] {
            if let Some(val) = m.and_then(|mv| mv.get(k)).or_else(|| e.get(k)) {
                if !val.is_null() {
                    b.as_object_mut().unwrap().insert(k.into(), val.clone());
                }
            }
        }
        out.push(b);

        if let Some(idx_v) = e.get("index") {
            let ix_opt = match idx_v {
                Value::Bool(true) => Some(i.to_string()),
                Value::Array(arr) => arr.get(i).and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                }),
                _ => None,
            };
            if let Some(ix) = ix_opt {
                if col {
                    out.push(json!({
                        "t": "text",
                        "x": bx - 12.0 - rong_chu(&ix, 15.0),
                        "y": by + bh / 2.0 - 9.0,
                        "text": ix,
                        "size": 15.0,
                        "color": "muted"
                    }));
                } else {
                    out.push(json!({
                        "t": "text",
                        "x": bx + bw / 2.0 - rong_chu(&ix, 15.0) / 2.0,
                        "y": by - 22.0,
                        "text": ix,
                        "size": 15.0,
                        "color": "muted"
                    }));
                }
            }
        }
        chay += buoc;
    }

    let tot_w = if col { cw } else { chay };
    let tot_h = if col { chay } else { ch };
    out.push(json!({
        "t": "anchor", "id": id, "x": x, "y": y, "w": tot_w, "h": tot_h
    }));
    if let Some(title) = str_at(e, "title") {
        let has_idx = e.get("index").map(|v| !matches!(v, Value::Bool(false) | Value::Null)).unwrap_or(false);
        let dy = if has_idx && !col { 46.0 } else { 28.0 };
        out.push(json!({
            "t": "text",
            "x": x,
            "y": y - dy,
            "text": title,
            "size": 16.0,
            "color": str_at(e, "titleColor").unwrap_or("muted")
        }));
    }
    Ok(out)
}

fn m_grid(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("grid: missing 'id'")?;
    let x = req_f64(e, "x", "grid")?;
    let y = req_f64(e, "y", "grid")?;
    let cw = req_f64(e, "cw", "grid")?;
    let ch = req_f64(e, "ch", "grid")?;
    let rows = e
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("grid: missing 'rows'")?;
    let marks = e.get("marks").and_then(Value::as_object);
    let size = f64_at(e, "size", 16.0);
    let mut out = Vec::new();
    let mut max_cols = 0usize;

    for (r, row_v) in rows.iter().enumerate() {
        let row = row_v.as_array().ok_or("grid: row must be array")?;
        max_cols = max_cols.max(row.len());
        for (c, lab_v) in row.iter().enumerate() {
            let lab = lab_v.as_str().unwrap_or("");
            let m = marks.and_then(|mk| mk.get(&format!("{r},{c}")));
            let mut b = json!({
                "t": "box",
                "id": format!("{id}.{r}.{c}"),
                "sharp": true,
                "x": x + c as f64 * cw,
                "y": y + r as f64 * ch,
                "w": cw,
                "h": ch,
                "text": lab,
                "size": size
            });
            for k in ["color", "fill", "fillStyle", "weight"] {
                if let Some(val) = m.and_then(|mv| mv.get(k)).or_else(|| e.get(k)) {
                    if !val.is_null() {
                        b.as_object_mut().unwrap().insert(k.into(), val.clone());
                    }
                }
            }
            out.push(b);
        }
    }
    out.push(json!({
        "t": "anchor",
        "id": id,
        "x": x,
        "y": y,
        "w": cw * max_cols as f64,
        "h": ch * rows.len() as f64
    }));
    Ok(out)
}

fn m_table(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("table: missing 'id'")?;
    let x0 = req_f64(e, "x", "table")?;
    let mut y = req_f64(e, "y", "table")?;
    let y0 = y;
    let cols: Vec<f64> = e
        .get("cols")
        .and_then(Value::as_array)
        .ok_or("table: missing 'cols'")?
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    let rh = f64_at(e, "rh", 38.0);
    let size = f64_at(e, "size", 16.0);
    let mut out = Vec::new();

    if let Some(title) = str_at(e, "title") {
        out.push(json!({
            "t": "text", "x": x0, "y": y - 28.0, "text": title, "size": 16.0, "color": "muted"
        }));
    }

    let head = e.get("head").and_then(Value::as_array);
    let rows = e
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("table: missing 'rows'")?;
    let marks = e.get("marks").and_then(Value::as_object);
    let total_w: f64 = cols.iter().sum();

    let mut all_rows: Vec<(bool, usize, &Vec<Value>)> = Vec::new();
    if let Some(h) = head {
        all_rows.push((true, 0, h));
    }
    for (ri, r) in rows.iter().enumerate() {
        if let Some(arr) = r.as_array() {
            all_rows.push((false, ri, arr));
        }
    }

    for (la_head, ri, row) in all_rows {
        let m = if la_head {
            None
        } else {
            marks.and_then(|mk| mk.get(&ri.to_string()))
        };
        let mut x = x0;
        for (c, lab_v) in row.iter().enumerate() {
            let w = *cols.get(c).unwrap_or(&80.0);
            let mc = if la_head {
                None
            } else {
                marks.and_then(|mk| mk.get(&format!("{ri},{c}")))
            };
            let lab = match lab_v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let color = mc
                .and_then(|v| str_at(v, "color"))
                .or_else(|| m.and_then(|v| str_at(v, "color")))
                .or_else(|| str_at(e, "color"))
                .unwrap_or("ink");
            let fill = mc
                .and_then(|v| str_at(v, "fill"))
                .or_else(|| m.and_then(|v| str_at(v, "fill")))
                .or_else(|| if la_head { Some("soft") } else { str_at(e, "fill") });
            let mut b = json!({
                "t": "box",
                "id": format!("{id}.{}.{c}", if la_head { "h".to_owned() } else { ri.to_string() }),
                "sharp": true,
                "x": x,
                "y": y,
                "w": w,
                "h": rh,
                "text": lab,
                "size": size,
                "color": color
            });
            if let Some(f) = fill {
                b.as_object_mut().unwrap().insert("fill".into(), json!(f));
            }
            if let Some(fs) = mc
                .and_then(|v| str_at(v, "fillStyle"))
                .or_else(|| m.and_then(|v| str_at(v, "fillStyle")))
            {
                b.as_object_mut().unwrap().insert("fillStyle".into(), json!(fs));
            }
            if mc.map(|v| bool_at(v, "dashed", false)).unwrap_or(false)
                || m.map(|v| bool_at(v, "dashed", false)).unwrap_or(false)
            {
                b.as_object_mut().unwrap().insert("dashed".into(), json!(true));
            }
            out.push(b);
            x += w;
        }
        if !la_head {
            out.push(json!({
                "t": "anchor", "id": format!("{id}.{ri}"), "x": x0, "y": y, "w": total_w, "h": rh
            }));
        }
        y += rh;
    }
    out.push(json!({
        "t": "anchor", "id": id, "x": x0, "y": y0, "w": total_w, "h": y - y0
    }));
    Ok(out)
}

fn m_entity(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("entity: missing 'id'")?;
    let x = req_f64(e, "x", "entity")?;
    let y = req_f64(e, "y", "entity")?;
    let w = req_f64(e, "w", "entity")?;
    let name = str_at(e, "name").ok_or("entity: missing 'name'")?;
    let fields: Vec<String> = e
        .get("fields")
        .and_then(Value::as_array)
        .ok_or("entity: missing 'fields'")?
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_owned())
        .collect();
    let fs = f64_at(e, "size", 16.0);
    let hh = 36.0;
    let bh = 24.0 * fields.len() as f64 + 14.0;
    let c = str_at(e, "color").unwrap_or("ink");
    let fill = str_at(e, "fill").unwrap_or("soft");
    Ok(vec![
        json!({
            "t": "box", "x": x, "y": y, "w": w, "h": hh, "sharp": true,
            "text": name, "size": 18.0, "color": c, "fill": fill
        }),
        json!({
            "t": "box", "x": x, "y": y + hh, "w": w, "h": bh, "sharp": true,
            "color": c, "text": fields.join("\n"), "size": fs, "align": "left", "valign": "top"
        }),
        json!({
            "t": "anchor", "id": id, "x": x, "y": y, "w": w, "h": hh + bh
        }),
    ])
}

fn m_code(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("code: missing 'id'")?;
    let x = req_f64(e, "x", "code")?;
    let y = req_f64(e, "y", "code")?;
    let text = str_at(e, "text").ok_or("code: missing 'text'")?;
    let fs = f64_at(e, "size", 15.0);
    let ts = fs.min(15.0);
    let lines: Vec<&str> = text.split('\n').collect();
    let title = str_at(e, "title");
    let mut can = lines
        .iter()
        .map(|l| rong_chu(l, fs) * 1.17)
        .fold(0.0_f64, f64::max);
    if let Some(ti) = title {
        can = can.max(rong_chu(ti, ts) * 1.17);
    }
    can += 32.0;

    if let Some(ew) = e.get("w").and_then(Value::as_f64) {
        if ew < can - 8.0 {
            return Err(format!(
                "code '{id}': w {ew:.0} is narrower than its longest line or title needs (about {can:.0})"
            ));
        }
    }
    let w = e.get("w").and_then(Value::as_f64).unwrap_or(can);
    let th = if title.is_some() { 26.0 } else { 0.0 };
    let h = lines.len() as f64 * fs * 1.3 + 24.0 + th;
    let color = str_at(e, "color").unwrap_or("line");
    let weight = if e.get("color").is_some() { 2.0 } else { 1.0 };

    let mut out = vec![
        json!({
            "t": "box", "id": id, "x": x, "y": y, "w": w, "h": h,
            "color": color, "fill": "code", "weight": weight
        }),
        json!({
            "t": "text", "x": x + 16.0, "y": y + 12.0 + th, "text": text,
            "size": fs, "color": "code-fg", "mono": true
        }),
    ];
    if let Some(ti) = title {
        out.push(json!({
            "t": "text", "x": x + 16.0, "y": y + 8.0, "text": ti,
            "size": ts, "color": "code-fg", "mono": true, "opacity": 60
        }));
    }
    Ok(out)
}

fn m_axis(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "axis")?;
    let y = req_f64(e, "y", "axis")?;
    let len = req_f64(e, "length", "axis")?;
    let c = str_at(e, "color").unwrap_or("ink");
    let doc = str_at(e, "dir") == Some("y");
    let end = if doc { [x, y - len] } else { [x + len, y] };
    let mut out = vec![json!({
        "t": "arrow", "points": [[x, y], end], "color": c
    })];
    let ticks = e.get("ticks").and_then(Value::as_array);
    let gap = e.get("gap").and_then(Value::as_f64);
    let s = f64_at(e, "size", 15.0);
    if let Some(tks) = ticks {
        let denom = tks.len().saturating_sub(1).max(1) as f64;
        for (i, lab_v) in tks.iter().enumerate() {
            let lab = lab_v.as_str().unwrap_or("");
            let d = gap.map(|g| g * i as f64).unwrap_or((len - 40.0) * i as f64 / denom);
            if doc {
                out.push(json!({ "t": "line", "points": [[x - 6.0, y - d], [x + 6.0, y - d]], "color": c }));
                out.push(json!({
                    "t": "text", "x": x - 14.0 - rong_chu(lab, s), "y": y - d - s * 0.6,
                    "text": lab, "size": s, "color": "muted"
                }));
            } else {
                out.push(json!({ "t": "line", "points": [[x + d, y - 6.0], [x + d, y + 6.0]], "color": c }));
                out.push(json!({
                    "t": "text", "x": x + d - rong_chu(lab, s) / 2.0, "y": y + 12.0,
                    "text": lab, "size": s, "color": "muted"
                }));
            }
        }
    }
    if let Some(lab) = str_at(e, "label") {
        let lx = if doc { x - rong_chu(lab, 17.0) / 2.0 } else { x + len + 10.0 };
        let ly = if doc { y - len - 30.0 } else { y - 12.0 };
        out.push(json!({
            "t": "text", "x": lx, "y": ly, "text": lab, "size": 17.0, "color": c
        }));
    }
    Ok(out)
}

fn m_span(e: &Value) -> Result<Vec<Value>, String> {
    let x1 = req_f64(e, "x1", "span")?;
    let x2 = req_f64(e, "x2", "span")?;
    let y = req_f64(e, "y", "span")?;
    let c = str_at(e, "color").unwrap_or("muted");
    let s = f64_at(e, "size", 16.0);
    let duoi = str_at(e, "side").unwrap_or("below") == "below";
    let mut out = vec![
        json!({ "t": "line", "points": [[x1, y], [x2, y]], "color": c, "weight": f64_at(e, "weight", 2.0) }),
        json!({ "t": "line", "points": [[x1, y - 8.0], [x1, y + 8.0]], "color": c }),
        json!({ "t": "line", "points": [[x2, y - 8.0], [x2, y + 8.0]], "color": c }),
    ];
    if let Some(txt) = str_at(e, "text") {
        out.push(json!({
            "t": "text",
            "x": (x1 + x2) / 2.0 - rong_chu(txt, s) / 2.0,
            "y": if duoi { y + 12.0 } else { y - 12.0 - cao_chu(txt, s) },
            "text": txt,
            "size": s,
            "color": c
        }));
    }
    Ok(out)
}

fn m_plot(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "plot")?;
    let y = req_f64(e, "y", "plot")?;
    let w = req_f64(e, "w", "plot")?;
    let h = req_f64(e, "h", "plot")?;
    let mut out = vec![
        json!({ "t": "arrow", "points": [[x, y], [x + w + 20.0, y]] }),
        json!({ "t": "arrow", "points": [[x, y], [x, y - h - 20.0]] }),
    ];
    if let Some(xl) = str_at(e, "xlabel") {
        out.push(json!({ "t": "text", "x": x + w + 28.0, "y": y - 12.0, "text": xl, "size": 16.0, "color": "muted" }));
    }
    if let Some(yl) = str_at(e, "ylabel") {
        out.push(json!({ "t": "text", "x": x - 10.0, "y": y - h - 50.0, "text": yl, "size": 16.0, "color": "muted" }));
    }

    let bars = e.get("bars").and_then(Value::as_array);
    let lines = e.get("lines").and_then(Value::as_array);
    let thresholds = e.get("thresholds").and_then(Value::as_array);

    let mut vmax = e.get("max").and_then(Value::as_f64).unwrap_or(0.0);
    if vmax <= 0.0 {
        vmax = 1e-9;
        if let Some(bs) = bars {
            for b in bs {
                vmax = vmax.max(f64_at(b, "value", 0.0));
            }
        }
        if let Some(ls) = lines {
            for s in ls {
                if let Some(pts) = s.get("points").and_then(Value::as_array) {
                    for p in pts {
                        if let Some(arr) = p.as_array() {
                            if let Some(py) = arr.get(1).and_then(Value::as_f64) {
                                vmax = vmax.max(py);
                            }
                        }
                    }
                }
            }
        }
        if let Some(ts) = thresholds {
            for t in ts {
                vmax = vmax.max(f64_at(t, "value", 0.0));
            }
        }
    }

    let pid = str_at(e, "id").unwrap_or("plot");
    if let Some(bs) = bars {
        if !bs.is_empty() {
            let slot = w / bs.len() as f64;
            let bw = (slot * 0.62).min(70.0);
            for (i, b) in bs.iter().enumerate() {
                let bh = (h * f64_at(b, "value", 0.0) / vmax).max(2.0);
                let bx = x + slot * i as f64 + (slot - bw) / 2.0;
                let bc = str_at(b, "color").unwrap_or("l4");
                let bf = str_at(b, "fill").unwrap_or(bc);
                let bfs = str_at(b, "fillStyle").unwrap_or("hachure");
                out.push(json!({
                    "t": "box", "id": format!("{pid}.{i}"), "sharp": true,
                    "x": bx, "y": y - bh, "w": bw, "h": bh,
                    "color": bc, "fill": bf, "fillStyle": bfs
                }));
                if let Some(lab) = str_at(b, "label") {
                    out.push(json!({
                        "t": "text", "x": bx + bw / 2.0 - rong_chu(lab, 15.0) / 2.0,
                        "y": y + 10.0, "text": lab, "size": 15.0, "color": "muted"
                    }));
                }
                if let Some(txt) = str_at(b, "text") {
                    out.push(json!({
                        "t": "text", "x": bx + bw / 2.0 - rong_chu(txt, 15.0) / 2.0,
                        "y": y - bh - 24.0, "text": txt, "size": 15.0, "color": bc
                    }));
                }
            }
        }
    }

    if let Some(ls) = lines {
        for s in ls {
            let raw_pts = s.get("points").and_then(Value::as_array).cloned().unwrap_or_default();
            let xm = e.get("xmax").and_then(Value::as_f64).unwrap_or_else(|| {
                raw_pts
                    .iter()
                    .filter_map(|p| p.as_array()?.first()?.as_f64())
                    .fold(1e-9_f64, f64::max)
            });
            let pts: Vec<[f64; 2]> = raw_pts
                .iter()
                .filter_map(|p| {
                    let arr = p.as_array()?;
                    let px = arr.first()?.as_f64()?;
                    let py = arr.get(1)?.as_f64()?;
                    Some([x + w * px / xm, y - h * py / vmax])
                })
                .collect();
            let lc = str_at(s, "color").unwrap_or("l4");
            out.push(json!({
                "t": "line", "points": pts, "color": lc, "weight": 3.0,
                "smooth": bool_at(s, "smooth", true)
            }));
            if let (Some(lab), Some(last)) = (str_at(s, "label"), pts.last()) {
                out.push(json!({
                    "t": "text", "x": last[0] + 8.0, "y": last[1] - 12.0,
                    "text": lab, "size": 15.0, "color": lc
                }));
            }
        }
    }

    if let Some(ts) = thresholds {
        for t in ts {
            let ty = y - h * f64_at(t, "value", 0.0) / vmax;
            let tc = str_at(t, "color").unwrap_or("err");
            out.push(json!({
                "t": "line", "points": [[x, ty], [x + w, ty]], "color": tc, "dashed": true
            }));
            if let Some(lab) = str_at(t, "label") {
                out.push(json!({
                    "t": "text", "x": x + w + 8.0, "y": ty - 12.0,
                    "text": lab, "size": 15.0, "color": tc
                }));
            }
        }
    }

    if let Some(xts) = e.get("xticks").and_then(Value::as_array) {
        for xv in xts {
            let tx = x + w * f64_at(xv, "at", 0.0);
            out.push(json!({ "t": "line", "points": [[tx, y - 5.0], [tx, y + 5.0]] }));
            if let Some(lab) = str_at(xv, "label") {
                out.push(json!({
                    "t": "text", "x": tx - rong_chu(lab, 15.0) / 2.0, "y": y + 10.0,
                    "text": lab, "size": 15.0, "color": "muted"
                }));
            }
        }
    }
    Ok(out)
}

const FRAGS: [&str; 7] = ["alt", "opt", "loop", "par", "group", "break", "critical"];

fn m_sequence(e: &Value) -> Result<Vec<Value>, String> {
    let ex = req_f64(e, "x", "sequence")?;
    let ey = req_f64(e, "y", "sequence")?;
    let acts = e
        .get("actors")
        .and_then(Value::as_array)
        .ok_or("sequence: missing 'actors'")?;
    let msgs = e
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("sequence: missing 'messages'")?;
    let mut gap = f64_at(e, "gap", 190.0);
    let step = f64_at(e, "step", 54.0);

    let mut aw = f64_at(e, "aw", 130.0);
    let mut max_lines = 1usize;
    for a in acts {
        let lab = str_at(a, "label").unwrap_or("");
        aw = aw.max(rong_chu(lab, 17.0) + 30.0);
        max_lines = max_lines.max(lab.matches('\n').count() + 1);
    }
    let ah = (max_lines as f64 * 17.0 * 1.35 + 11.0).round().max(44.0);
    gap = gap.max(aw + 30.0);

    let mut pos: HashMap<String, f64> = HashMap::new();
    for (i, a) in acts.iter().enumerate() {
        let aid = str_at(a, "id").ok_or("sequence: actor missing 'id'")?;
        pos.insert(aid.to_owned(), ex + i as f64 * gap + aw / 2.0);
    }
    let xa = pos.values().copied().fold(f64::INFINITY, f64::min);
    let xb = pos.values().copied().fold(f64::NEG_INFINITY, f64::max);
    let sid = str_at(e, "id")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("seq@{ex},{ey}"));

    struct SeqFrame {
        kind: String,
        label: String,
        top: f64,
        x0: f64,
        x1: f64,
        seps: Vec<(f64, String)>,
        color: String,
        over: Option<Vec<String>>,
    }

    let mut out = Vec::new();
    let mut y = ey + ah + 34.0;
    let mut stack: Vec<SeqFrame> = Vec::new();

    let cham = |stack: &mut Vec<SeqFrame>, x1: f64, x2: f64| {
        for f in stack.iter_mut() {
            f.x0 = f.x0.min(x1);
            f.x1 = f.x1.max(x2);
        }
    };

    let mut prev_kind: Option<&str> = None;
    for m in msgs {
        let c = str_at(m, "color").unwrap_or("ink");
        let kind = FRAGS.iter().copied().find(|&k| m.get(k).is_some());
        let open_before = prev_kind;
        prev_kind = kind;

        if kind.is_some() && y - 34.0 < ey + ah + 14.0 {
            y += 14.0;
        }
        if let Some(k) = kind {
            let over = m.get("over").and_then(Value::as_array).map(|arr| {
                arr.iter().filter_map(Value::as_str).map(str::to_owned).collect()
            });
            stack.push(SeqFrame {
                kind: k.to_owned(),
                label: str_at(m, k).unwrap_or("").to_owned(),
                top: y - 34.0,
                x0: 1e9,
                x1: -1e9,
                seps: Vec::new(),
                color: str_at(m, "color").unwrap_or("muted").to_owned(),
                over,
            });
            y += 30.0;
            continue;
        }
        if let Some(el_v) = m.get("else") {
            let top_f = stack
                .last_mut()
                .ok_or("sequence: 'else' is outside an alt frame")?;
            top_f
                .seps
                .push((y - 30.0, el_v.as_str().unwrap_or("").to_owned()));
            y += 28.0;
            continue;
        }
        if bool_at(m, "end", false) {
            let mut f = stack
                .pop()
                .ok_or("sequence: 'end' has no open frame")?;
            if let Some(ref ov) = f.over {
                f.x0 = ov.iter().filter_map(|k| pos.get(k)).copied().fold(f64::INFINITY, f64::min);
                f.x1 = ov.iter().filter_map(|k| pos.get(k)).copied().fold(f64::NEG_INFINITY, f64::max);
            } else {
                f.x0 = xa - 6.0 + 12.0 * stack.len() as f64;
                f.x1 = (xb + 6.0 - 12.0 * stack.len() as f64).max(f.x1);
                for (k, par) in stack.iter().rev().enumerate() {
                    if let Some(ref pov) = par.over {
                        f.x0 = pov.iter().filter_map(|id| pos.get(id)).copied().fold(f64::INFINITY, f64::min)
                            + 12.0 * (k + 1) as f64;
                        f.x1 = pov.iter().filter_map(|id| pos.get(id)).copied().fold(f64::NEG_INFINITY, f64::max)
                            - 12.0 * (k + 1) as f64;
                        break;
                    }
                }
            }
            let x0 = f.x0 - 34.0;
            let x1 = f.x1 + 34.0;
            let top = f.top;
            let bot = y - 28.0;
            let tw = rong_chu(&f.kind, 15.0) + 26.0;
            out.push(json!({
                "t": "box", "x": x0, "y": top, "w": x1 - x0, "h": bot - top,
                "sharp": true, "color": f.color, "weight": 1.0, "under": true
            }));
            out.push(json!({
                "t": "box", "x": x0, "y": top, "w": tw, "h": 30.0,
                "sharp": true, "color": f.color, "fill": "soft", "weight": 1.0,
                "text": f.kind, "size": 15.0
            }));
            if !f.label.is_empty() {
                out.push(json!({
                    "t": "text", "x": x0 + tw + 8.0, "y": top + 5.0,
                    "text": format!("[{}]", f.label), "size": 15.0, "color": f.color
                }));
            }
            for (sy, lab) in &f.seps {
                out.push(json!({
                    "t": "line", "points": [[x0, sy], [x1, sy]], "color": f.color, "dashed": true, "weight": 1.0
                }));
                if !lab.is_empty() {
                    out.push(json!({
                        "t": "text", "x": x0 + 10.0, "y": sy + 4.0,
                        "text": format!("[{lab}]"), "size": 15.0, "color": f.color
                    }));
                }
            }
            cham(&mut stack, f.x0 - 12.0, f.x1 + 12.0);
            y += 14.0;
            continue;
        }
        if (m.get("divider").is_some() || m.get("delay").is_some()) && y - 34.0 < ey + ah + 14.0 {
            y += 14.0;
        }
        if let Some(div) = str_at(m, "divider") {
            let w = rong_chu(div, 15.0) + 28.0;
            out.push(json!({
                "t": "line", "points": [[xa - aw / 2.0, y - 12.0], [xb + aw / 2.0, y - 12.0]],
                "color": "muted", "weight": 1.0
            }));
            out.push(json!({
                "t": "box", "x": (xa + xb) / 2.0 - w / 2.0, "y": y - 28.0, "w": w, "h": 32.0,
                "text": div, "size": 15.0, "color": "muted", "fill": "soft", "weight": 1.0, "sharp": true
            }));
            y += 44.0;
            continue;
        }
        if let Some(del) = str_at(m, "delay") {
            out.push(json!({
                "t": "text", "x": (xa + xb) / 2.0, "y": y - 22.0, "text": del,
                "size": 15.0, "color": "muted", "align": "center"
            }));
            y += 40.0;
            continue;
        }
        if let Some(note) = str_at(m, "note") {
            let ids: Vec<String> = m
                .get("over")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_else(|| {
                    vec![
                        str_at(&acts[0], "id").unwrap_or("").to_owned(),
                        str_at(acts.last().unwrap(), "id").unwrap_or("").to_owned(),
                    ]
                });
            let pad = if m.get("over").is_some() || stack.is_empty() {
                40.0
            } else {
                28.0 - 12.0 * stack.len() as f64
            };
            let x1 = ids.iter().filter_map(|k| pos.get(k)).copied().fold(f64::INFINITY, f64::min) - pad;
            let x2 = ids.iter().filter_map(|k| pos.get(k)).copied().fold(f64::NEG_INFINITY, f64::max) + pad;
            let nh = ((note.matches('\n').count() + 1) as f64 * 15.0 * 1.35 + 11.0).round().max(34.0);
            out.push(json!({
                "t": "box", "x": x1, "y": y - 16.0, "w": x2 - x1, "h": nh,
                "text": note, "size": 15.0,
                "color": str_at(m, "color").unwrap_or("muted"),
                "fill": str_at(m, "fill").unwrap_or("soft"),
                "dashed": true, "weight": 1.0
            }));
            cham(&mut stack, x1 + pad, x2 - pad);
            y += step * 1.5 + nh - 34.0;
            continue;
        }

        let n_prefix = opt_val_str(m, "n").map(|n| format!("{n}. ")).unwrap_or_default();
        let nhan = format!("{n_prefix}{}", str_at(m, "text").unwrap_or(""));
        if nhan.contains('\n') {
            return Err(format!("sequence: message text '{}' must be one line", &nhan));
        }

        let from_id = str_at(m, "from").ok_or("sequence: message missing 'from'")?;
        let to_id = str_at(m, "to");
        if bool_at(m, "self", false) || to_id == Some(from_id) {
            if open_before.is_some() {
                y -= 20.0;
            }
            let x = *pos.get(from_id).ok_or_else(|| format!("sequence: unknown actor '{from_id}'"))?;
            out.push(json!({
                "t": "arrow",
                "points": [[x, y], [x + 46.0, y], [x + 46.0, y + 26.0], [x + 4.0, y + 26.0]],
                "color": c,
                "dashed": bool_at(m, "dashed", false),
                "sharp": true
            }));
            let lab_id = format!("{sid}~lab~{}", out.len());
            out.push(json!({
                "t": "text", "id": lab_id, "x": x + 56.0, "y": y,
                "text": nhan, "size": 15.0, "color": c
            }));
            cham(&mut stack, x, x + 30.0 + rong_chu(&nhan, 15.0));
            y += step + 12.0;
            continue;
        }

        let to_id = to_id.ok_or("sequence: message missing 'to'")?;
        let x1 = *pos.get(from_id).ok_or_else(|| format!("sequence: unknown actor '{from_id}'"))?;
        let mut x2 = *pos.get(to_id).ok_or_else(|| format!("sequence: unknown actor '{to_id}'"))?;
        let d = if x2 > x1 { 1.0 } else { -1.0 };
        if bool_at(m, "lost", false) {
            x2 = x1 + (x2 - x1) * 0.72;
            out.push(json!({
                "t": "cross", "x": x2 + 2.0 * d - 8.0, "y": y - 8.0, "size": 16.0
            }));
        }
        out.push(json!({
            "t": "arrow",
            "points": [[x1 + 3.0 * d, y], [x2 - 4.0 * d, y]],
            "color": c,
            "dashed": bool_at(m, "dashed", false),
            "weight": f64_at(m, "weight", 2.0),
            "head": str_at(m, "head").unwrap_or("arrow")
        }));
        let mut giua: Vec<f64> = pos
            .values()
            .copied()
            .filter(|&p| p > x1.min(x2) && p < x1.max(x2))
            .collect();
        giua.sort_by(|a, b| (a - x1).abs().partial_cmp(&(b - x1).abs()).unwrap_or(std::cmp::Ordering::Equal));
        let cx = if let Some(&g) = giua.first() {
            (x1 + g) / 2.0
        } else {
            (x1 + x2) / 2.0
        };
        if !nhan.is_empty() {
            let lab_id = format!("{sid}~lab~{}", out.len());
            out.push(json!({
                "t": "text", "id": lab_id, "x": cx, "y": y - 24.0,
                "text": nhan, "size": 15.0, "color": c, "align": "center"
            }));
        }
        if bool_at(m, "fail", false) {
            out.push(json!({
                "t": "cross", "x": cx + rong_chu(&nhan, 15.0) * 0.42 + 8.0, "y": y - 26.0, "size": 16.0
            }));
        }
        cham(&mut stack, x1.min(x2), x1.max(x2));
        y += step;
    }

    if let Some(f) = stack.last() {
        return Err(format!("sequence: frame '{}' is not closed with {{\"end\":true}}", f.kind));
    }

    for a in acts {
        let aid = str_at(a, "id").unwrap();
        let px = pos[aid];
        out.push(json!({
            "t": "box", "id": aid, "x": px - aw / 2.0, "y": ey, "w": aw, "h": ah,
            "text": str_at(a, "label").unwrap_or(""), "size": 17.0,
            "color": str_at(a, "color").unwrap_or("ink"),
            "fill": str_at(a, "fill").unwrap_or("soft")
        }));
        out.push(json!({
            "t": "line", "id": format!("{sid}~life~{aid}"),
            "points": [[px, ey + ah], [px, y - 10.0]],
            "color": "line", "dashed": true, "weight": 1.0, "under": true
        }));
    }
    Ok(out)
}

fn m_tree(e: &Value) -> Result<Vec<Value>, String> {
    let ex = req_f64(e, "x", "tree")?;
    let ey = req_f64(e, "y", "tree")?;
    let root = e.get("root").ok_or("tree: missing 'root'")?;
    let nw = f64_at(e, "nw", 110.0);
    let nh = f64_at(e, "nh", 44.0);
    let dx = f64_at(e, "dx", 26.0);
    let dy = f64_at(e, "dy", 64.0);
    let ngang = str_at(e, "dir") == Some("right");
    let lines_only = bool_at(e, "lines", false);
    let base_id = str_at(e, "id").unwrap_or("tree");
    let def_size = f64_at(e, "size", 17.0);
    let mut out = Vec::new();
    let mut dem = 0usize;

    fn leaf_count(n: &Value) -> usize {
        let ch = n.get("children").and_then(Value::as_array);
        match ch {
            Some(arr) if !arr.is_empty() => arr.iter().map(leaf_count).sum(),
            _ => 1,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dat(
        n: &Value,
        lvl: usize,
        start: f64,
        e: &Value,
        ex: f64,
        ey: f64,
        nw: f64,
        nh: f64,
        dx: f64,
        dy: f64,
        ngang: bool,
        lines_only: bool,
        base_id: &str,
        def_size: f64,
        dem: &mut usize,
        out: &mut Vec<Value>,
    ) -> String {
        let ch = n.get("children").and_then(Value::as_array);
        let o = (if ngang { nh } else { nw }) + dx;
        let span = leaf_count(n) as f64 * o;
        let ctr = start + span / 2.0;
        let nid = str_at(n, "id")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let id = format!("{base_id}.{dem}");
                *dem += 1;
                id
            });
        let w = f64_at(n, "w", nw);
        let h = f64_at(n, "h", nh);
        let (x, y) = if ngang {
            (ex + lvl as f64 * (nw + dy), ey + ctr - h / 2.0)
        } else {
            (ex + ctr - w / 2.0, ey + lvl as f64 * (nh + dy))
        };
        let mut b = json!({
            "t": str_at(n, "shape").unwrap_or("box"),
            "id": nid,
            "x": x,
            "y": y,
            "w": w,
            "h": h,
            "text": str_at(n, "text").unwrap_or(""),
            "size": f64_at(n, "size", def_size)
        });
        for k in ["color", "fill", "fillStyle", "dashed", "sharp", "weight"] {
            if let Some(v) = n.get(k).or_else(|| e.get(k)) {
                if !v.is_null() {
                    b.as_object_mut().unwrap().insert(k.into(), v.clone());
                }
            }
        }
        out.push(b);

        if let Some(children) = ch {
            let mut s = start;
            for c in children {
                let cid = dat(
                    c, lvl + 1, s, e, ex, ey, nw, nh, dx, dy, ngang, lines_only, base_id, def_size,
                    dem, out,
                );
                s += leaf_count(c) as f64 * o;
                let mut a = json!({
                    "t": "arrow",
                    "from": nid,
                    "to": cid,
                    "color": str_at(c, "edgeColor").or_else(|| str_at(e, "edgeColor")).unwrap_or("ink"),
                    "fromSide": if ngang { "right" } else { "bottom" },
                    "toSide": if ngang { "left" } else { "top" },
                    "head": if lines_only { "none" } else { "arrow" },
                    "dashed": bool_at(c, "edgeDashed", false)
                });
                if let Some(edge_txt) = str_at(c, "edge") {
                    a.as_object_mut().unwrap().insert("text".into(), json!(edge_txt));
                }
                out.push(a);
            }
        }
        nid
    }

    dat(
        root, 0, 0.0, e, ex, ey, nw, nh, dx, dy, ngang, lines_only, base_id, def_size, &mut dem,
        &mut out,
    );
    Ok(out)
}

fn m_ring(e: &Value) -> Result<Vec<Value>, String> {
    let cx = req_f64(e, "cx", "ring")?;
    let cy = req_f64(e, "cy", "ring")?;
    let r = req_f64(e, "r", "ring")?;
    let mut out = vec![json!({
        "t": "ellipse",
        "id": str_at(e, "id").unwrap_or("ring"),
        "x": cx - r, "y": cy - r, "w": 2.0 * r, "h": 2.0 * r,
        "color": str_at(e, "color").unwrap_or("l2")
    })];
    let pt = |a: f64, rr: f64| -> (f64, f64) {
        let t = (a - 90.0).to_radians();
        (cx + rr * t.cos(), cy + rr * t.sin())
    };
    let nodes = e.get("nodes").and_then(Value::as_array);

    if let Some(arcs) = e.get("arcs").and_then(Value::as_array) {
        for a in arcs {
            let t1 = f64_at(a, "from", 0.0);
            let mut t2 = f64_at(a, "to", 90.0);
            if t2 < t1 {
                t2 += 360.0;
            }
            let n = (((t2 - t1) / 6.0) as usize).max(2);
            let pts: Vec<[f64; 2]> = (0..=n)
                .map(|i| {
                    let (px, py) = pt(t1 + (t2 - t1) * i as f64 / n as f64, r);
                    [px, py]
                })
                .collect();
            let ac = str_at(a, "color").unwrap_or("warn");
            out.push(json!({
                "t": "line", "points": pts, "color": ac,
                "weight": f64_at(a, "weight", 8.0), "smooth": true, "opacity": 70
            }));
            if let Some(lab) = str_at(a, "label") {
                let (lx, ly) = pt((t1 + t2) / 2.0, r - 34.0);
                out.push(json!({
                    "t": "text", "x": lx - rong_chu(lab, 15.0) / 2.0, "y": ly - 10.0,
                    "text": lab, "size": 15.0, "color": ac
                }));
            }
        }
    }

    if let Some(ticks) = e.get("ticks").and_then(Value::as_array) {
        for k in ticks {
            let ang = f64_at(k, "angle", 0.0);
            let (px, py) = pt(ang, r - 6.0);
            let (qx, qy) = pt(ang, r + 6.0);
            out.push(json!({ "t": "line", "points": [[px, py], [qx, qy]], "color": "muted" }));
            let near_d = nodes
                .map(|ns| {
                    ns.iter()
                        .filter(|n| {
                            let diff = ((f64_at(n, "angle", 0.0) - ang + 180.0).rem_euclid(360.0)) - 180.0;
                            diff.abs() < 12.0
                        })
                        .map(|n| f64_at(n, "d", 26.0))
                        .fold(0.0_f64, f64::max)
                })
                .unwrap_or(0.0);
            let inset = if near_d > 0.0 { near_d / 2.0 + 2.0 } else { 0.0 };
            let (lx, ly) = pt(ang, r - 22.0 - inset);
            let lab = str_at(k, "label").unwrap_or("");
            out.push(json!({
                "t": "text", "x": lx - rong_chu(lab, 15.0) / 2.0, "y": ly - 9.0,
                "text": lab, "size": 15.0, "color": "muted"
            }));
        }
    }

    if let Some(ns) = nodes {
        for n in ns {
            let d = f64_at(n, "d", 26.0);
            let ang = f64_at(n, "angle", 0.0);
            let (px, py) = pt(ang, r);
            let c = str_at(n, "color").unwrap_or("l4");
            let mut el = json!({
                "t": "ellipse",
                "id": str_at(n, "id").unwrap_or(""),
                "x": px - d / 2.0, "y": py - d / 2.0, "w": d, "h": d,
                "color": c,
                "fill": str_at(n, "fill").unwrap_or(c),
                "fillStyle": str_at(n, "fillStyle").unwrap_or("solid")
            });
            if bool_at(n, "dashed", false) {
                el.as_object_mut().unwrap().insert("dashed".into(), json!(true));
            }
            out.push(el);
            if let Some(lab) = str_at(n, "label") {
                let s = f64_at(n, "size", 16.0);
                let (lx, ly) = pt(ang, r + d / 2.0 + 14.0);
                let rad = (ang - 90.0).to_radians();
                let (ca, sa) = (rad.cos(), rad.sin());
                let (tw, th) = (rong_chu(lab, s), cao_chu(lab, s));
                out.push(json!({
                    "t": "text",
                    "x": lx - tw / 2.0 + ca * tw / 2.0,
                    "y": ly - th / 2.0 + sa * th / 2.0,
                    "text": lab, "size": s, "color": c
                }));
            }
        }
    }
    Ok(out)
}

fn m_lanes(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "lanes")?;
    let mut y = req_f64(e, "y", "lanes")?;
    let y0 = y;
    let w = req_f64(e, "w", "lanes")?;
    let lw = f64_at(e, "labelW", 110.0);
    let lanes = e
        .get("lanes")
        .and_then(Value::as_array)
        .ok_or("lanes: missing 'lanes'")?;
    let mut out = Vec::new();
    for ln in lanes {
        let h = f64_at(ln, "h", 90.0);
        let mut b = json!({
            "t": "box", "id": str_at(ln, "id").unwrap_or(""),
            "x": x, "y": y, "w": w, "h": h, "sharp": true,
            "color": "line", "weight": 1.0, "under": true
        });
        if let Some(f) = str_at(ln, "fill") {
            b.as_object_mut().unwrap().insert("fill".into(), json!(f));
        }
        out.push(b);
        out.push(ghost_box(
            x,
            y,
            lw,
            h,
            str_at(ln, "label").unwrap_or(""),
            16.0,
            str_at(ln, "color").unwrap_or("muted"),
        ));
        y += h;
    }
    out.push(json!({
        "t": "line", "points": [[x + lw, y0], [x + lw, y]], "color": "line", "weight": 1.0
    }));
    Ok(out)
}

fn m_start(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "start")?;
    let y = req_f64(e, "y", "start")?;
    let d = f64_at(e, "d", 22.0);
    let mut o = json!({
        "t": "ellipse", "x": x, "y": y, "w": d, "h": d,
        "color": str_at(e, "color").unwrap_or("ink"), "fill": "ink", "rough": 0
    });
    if let Some(id) = str_at(e, "id") {
        o.as_object_mut().unwrap().insert("id".into(), json!(id));
    }
    Ok(vec![o])
}

fn m_end(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "end")?;
    let y = req_f64(e, "y", "end")?;
    let d = f64_at(e, "d", 26.0);
    let c = str_at(e, "color").unwrap_or("ink");
    let mut outer = json!({
        "t": "ellipse", "x": x, "y": y, "w": d, "h": d, "color": c
    });
    if let Some(id) = str_at(e, "id") {
        outer.as_object_mut().unwrap().insert("id".into(), json!(id));
    }
    Ok(vec![
        outer,
        json!({
            "t": "ellipse", "x": x + 6.0, "y": y + 6.0, "w": d - 12.0, "h": d - 12.0,
            "color": c, "fill": "ink", "rough": 0
        }),
    ])
}

fn m_swimlane(e: &Value) -> Result<Vec<Value>, String> {
    let ex = req_f64(e, "x", "swimlane")?;
    let ey = req_f64(e, "y", "swimlane")?;
    let lanes = e
        .get("lanes")
        .and_then(Value::as_array)
        .ok_or("swimlane: missing 'lanes'")?;
    let nodes = e
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or("swimlane: missing 'nodes'")?;
    let rh = f64_at(e, "rowH", 110.0);
    let hh = f64_at(e, "headH", 40.0);
    let nw = f64_at(e, "nw", 150.0);
    let nh = f64_at(e, "nh", 56.0);
    let size = f64_at(e, "size", 16.0);
    let lane_w_def = f64_at(e, "laneW", 190.0);

    let co = |n: &Value| -> (f64, f64) {
        let k = str_at(n, "kind").unwrap_or("step");
        let sz = f64_at(n, "size", size);
        let tx = str_at(n, "text").unwrap_or("");
        if matches!(k, "start" | "end" | "merge") {
            return (26.0, 26.0);
        }
        if k == "decision" {
            let w = n
                .get("w")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| (nw * 1.1).max(rong_chu(tx, sz) * 2.0 + 44.0));
            let dong: f64 = tx
                .split('\n')
                .map(|l| ((rong_chu(l, sz) + 8.0) / (w / 2.0 - 8.0).max(1.0)).ceil().max(1.0))
                .sum();
            let h = n
                .get("h")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| (nh * 1.5).max(dong * sz * 1.25 * 2.0 + 34.0));
            return (w, h);
        }
        let w = n
            .get("w")
            .and_then(Value::as_f64)
            .unwrap_or_else(|| nw.max(rong_chu(tx, sz) + 30.0));
        let h = n
            .get("h")
            .and_then(Value::as_f64)
            .unwrap_or_else(|| nh.max(cao_chu(tx, sz) + 26.0));
        (w, h)
    };

    let mut day = ey + hh;
    for n in nodes {
        let (_, h) = co(n);
        let row = f64_at(n, "row", 0.0);
        let dy = f64_at(n, "dy", 0.0);
        day = day.max(ey + hh + 8.0 + row * rh + rh / 2.0 + dy + h / 2.0);
    }
    let total_h = day - ey + 26.0;

    let mut out = Vec::new();
    let mut x = ex;
    let mut cx: HashMap<String, f64> = HashMap::new();
    let mut li: HashMap<String, usize> = HashMap::new();

    for (k, ln) in lanes.iter().enumerate() {
        let w = f64_at(ln, "w", lane_w_def);
        let lid = str_at(ln, "id").ok_or("swimlane: lane missing 'id'")?;
        cx.insert(lid.to_owned(), x + w / 2.0);
        li.insert(lid.to_owned(), k);
        out.push(ghost_box(
            x,
            ey,
            w,
            hh,
            str_at(ln, "label").unwrap_or(""),
            17.0,
            str_at(ln, "color").unwrap_or("ink"),
        ));
        if let Some(fill) = str_at(ln, "fill") {
            out.push(json!({
                "t": "box", "x": x, "y": ey, "w": w, "h": total_h,
                "sharp": true, "color": "line", "fill": fill, "weight": 1.0, "under": true
            }));
        }
        if k > 0 {
            out.push(json!({
                "t": "line", "points": [[x, ey], [x, ey + total_h]],
                "color": "line", "weight": 1.0, "under": true
            }));
        }
        x += w;
    }
    out.push(json!({
        "t": "box", "x": ex, "y": ey, "w": x - ex, "h": total_h,
        "sharp": true, "color": "line", "weight": 1.0, "under": true
    }));
    out.push(json!({
        "t": "line", "points": [[ex, ey + hh], [x, ey + hh]],
        "color": "line", "weight": 1.0, "under": true
    }));

    struct NodeMeta {
        row: f64,
        kind: String,
        mx: f64,
        w: f64,
    }
    let mut info: HashMap<String, NodeMeta> = HashMap::new();
    for n in nodes {
        let nid = str_at(n, "id").ok_or("swimlane: node missing 'id'")?;
        let lane = str_at(n, "lane").ok_or("swimlane: node missing 'lane'")?;
        let &lane_cx = cx
            .get(lane)
            .ok_or_else(|| format!("swimlane: node '{nid}' uses unknown lane '{lane}'"))?;
        let kind = str_at(n, "kind").unwrap_or("step");
        let row = f64_at(n, "row", 0.0);
        let mx = lane_cx + f64_at(n, "dx", 0.0);
        let my = ey + hh + 8.0 + row * rh + rh / 2.0 + f64_at(n, "dy", 0.0);
        let (w, h) = co(n);
        info.insert(
            nid.to_owned(),
            NodeMeta {
                row,
                kind: kind.to_owned(),
                mx,
                w,
            },
        );
        if kind == "start" || kind == "end" {
            let d = if kind == "start" { 22.0 } else { 26.0 };
            out.push(json!({
                "t": kind, "id": nid, "x": mx - d / 2.0, "y": my - d / 2.0, "d": d
            }));
            continue;
        }
        let mut b = json!({
            "t": if kind == "decision" || kind == "merge" { "diamond" } else { "box" },
            "id": nid,
            "x": mx - w / 2.0,
            "y": my - h / 2.0,
            "w": w,
            "h": h,
            "size": f64_at(n, "size", size),
            "color": str_at(n, "color").unwrap_or("ink")
        });
        if let Some(txt) = str_at(n, "text") {
            b.as_object_mut().unwrap().insert("text".into(), json!(txt));
        }
        for k in ["fill", "fillStyle", "dashed", "weight"] {
            if let Some(v) = n.get(k) {
                b.as_object_mut().unwrap().insert(k.into(), v.clone());
            }
        }
        out.push(b);
    }

    if let Some(edges) = e.get("edges").and_then(Value::as_array) {
        for g in edges {
            let from = str_at(g, "from").ok_or("swimlane: edge missing 'from'")?;
            let to = str_at(g, "to").ok_or("swimlane: edge missing 'to'")?;
            let ia = info
                .get(from)
                .ok_or_else(|| format!("swimlane: edge from='{from}' matches no node"))?;
            let ib = info
                .get(to)
                .ok_or_else(|| format!("swimlane: edge to='{to}' matches no node"))?;
            let mut a = g.clone();
            let obj = a.as_object_mut().unwrap();
            obj.insert("t".into(), json!("arrow"));
            if !obj.contains_key("size") {
                obj.insert("size".into(), json!(15.0));
            }
            if !obj.contains_key("fromSide")
                && !obj.contains_key("toSide")
                && !obj.contains_key("route")
                && !obj.contains_key("bend")
                && from != to
            {
                if (ia.mx - ib.mx).abs() < 1.0 {
                    let (fs, ts) = if ib.row >= ia.row {
                        ("bottom", "top")
                    } else {
                        ("top", "bottom")
                    };
                    obj.insert("fromSide".into(), json!(fs));
                    obj.insert("toSide".into(), json!(ts));
                } else if (ia.row - ib.row).abs() < 1e-6 {
                    let (fs, ts) = if ib.mx > ia.mx {
                        ("right", "left")
                    } else {
                        ("left", "right")
                    };
                    obj.insert("fromSide".into(), json!(fs));
                    obj.insert("toSide".into(), json!(ts));
                } else if (ia.kind == "decision" || ia.kind == "merge")
                    && (ib.mx - ia.mx).abs() >= ia.w / 2.0 + 12.0
                {
                    obj.insert(
                        "fromSide".into(),
                        json!(if ib.mx > ia.mx { "right" } else { "left" }),
                    );
                    obj.insert(
                        "toSide".into(),
                        json!(if ib.row > ia.row { "top" } else { "bottom" }),
                    );
                    obj.insert("route".into(), json!("hv"));
                } else {
                    let (fs, ts) = if ib.row > ia.row {
                        ("bottom", "top")
                    } else {
                        ("top", "bottom")
                    };
                    obj.insert("fromSide".into(), json!(fs));
                    obj.insert("toSide".into(), json!(ts));
                    obj.insert("route".into(), json!("vhv"));
                }
            }
            if obj.get("text").and_then(Value::as_str).is_some()
                && ia.kind == "decision"
                && !obj.contains_key("labelAt")
                && !obj.contains_key("labelDx")
                && !obj.contains_key("labelDy")
            {
                obj.insert("labelAt".into(), json!(30.0));
            }
            out.push(a);
        }
    }
    Ok(out)
}

fn m_stack(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("stack: missing 'id'")?;
    let x = req_f64(e, "x", "stack")?;
    let y = req_f64(e, "y", "stack")?;
    let n = f64_at(e, "n", 3.0) as usize;
    let mut out = Vec::new();
    for k in (0..n).rev() {
        let mut b = e.clone();
        let obj = b.as_object_mut().unwrap();
        obj.insert("t".into(), json!("box"));
        obj.insert("x".into(), json!(x + k as f64 * 8.0));
        obj.insert("y".into(), json!(y - k as f64 * 8.0));
        if !obj.contains_key("fill") {
            obj.insert("fill".into(), json!("soft"));
        }
        if k > 0 {
            obj.insert("id".into(), json!(format!("{id}~{k}")));
            obj.remove("text");
        }
        out.push(b);
    }
    Ok(out)
}

fn m_cross(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "cross")?;
    let y = req_f64(e, "y", "cross")?;
    let s = f64_at(e, "size", 22.0);
    let c = str_at(e, "color").unwrap_or("err");
    Ok(vec![
        json!({ "t": "line", "points": [[x, y], [x + s, y + s]], "color": c, "weight": 4.0 }),
        json!({ "t": "line", "points": [[x + s, y], [x, y + s]], "color": c, "weight": 4.0 }),
    ])
}

fn m_check(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "check")?;
    let y = req_f64(e, "y", "check")?;
    let s = f64_at(e, "size", 22.0);
    let c = str_at(e, "color").unwrap_or("node");
    Ok(vec![json!({
        "t": "line",
        "points": [[x, y + s * 0.55], [x + s * 0.38, y + s], [x + s, y]],
        "color": c, "weight": 4.0
    })])
}

fn m_badge(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "badge")?;
    let y = req_f64(e, "y", "badge")?;
    let d = f64_at(e, "d", 30.0);
    let c = str_at(e, "color").unwrap_or("l4");
    let n = opt_val_str(e, "n").ok_or("badge: missing 'n'")?;
    let mut ell = json!({
        "t": "ellipse", "x": x, "y": y, "w": d, "h": d, "color": c, "fill": c
    });
    if let Some(id) = str_at(e, "id") {
        ell.as_object_mut().unwrap().insert("id".into(), json!(id));
    }
    Ok(vec![
        ell,
        ghost_box(x - 6.0, y, d + 12.0, d, &n, f64_at(e, "size", 16.0), c),
    ])
}

fn m_note(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "note")?;
    let y = req_f64(e, "y", "note")?;
    let txt = str_at(e, "text").ok_or("note: missing 'text'")?;
    let s = f64_at(e, "size", 16.0);
    let c = str_at(e, "color").unwrap_or("muted");
    let w = rong_chu(txt, s) + 8.0;
    let h = cao_chu(txt, s);
    let nid = str_at(e, "id")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("note@{x},{y}"));
    let mut out = vec![
        json!({ "t": "text", "x": x, "y": y, "text": txt, "size": s, "color": c }),
        json!({ "t": "anchor", "id": nid, "x": x - 4.0, "y": y - 2.0, "w": w, "h": h + 4.0 }),
    ];
    if let Some(to) = str_at(e, "to") {
        let mut a = json!({
            "t": "arrow", "from": nid, "to": to, "color": c,
            "dashed": true, "weight": 1.0, "bend": f64_at(e, "bend", 0.0)
        });
        if let Some(ts) = str_at(e, "toSide") {
            a.as_object_mut().unwrap().insert("toSide".into(), json!(ts));
        }
        if let Some(fs) = str_at(e, "fromSide") {
            a.as_object_mut().unwrap().insert("fromSide".into(), json!(fs));
        }
        out.push(a);
    }
    Ok(out)
}

fn m_legend(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "legend")?;
    let mut y = req_f64(e, "y", "legend")?;
    let items = e
        .get("items")
        .and_then(Value::as_array)
        .ok_or("legend: missing 'items'")?;
    let mut out = Vec::new();
    for it in items {
        let c = str_at(it, "color").unwrap_or("ink");
        if bool_at(it, "line", false) {
            out.push(json!({
                "t": "arrow", "points": [[x, y + 10.0], [x + 34.0, y + 10.0]],
                "color": c, "dashed": bool_at(it, "dashed", false),
                "head": str_at(it, "head").unwrap_or("arrow")
            }));
        } else {
            let mut b = json!({
                "t": str_at(it, "shape").unwrap_or("box"),
                "x": x + 6.0, "y": y, "w": 22.0, "h": 20.0,
                "color": c, "fillStyle": str_at(it, "fillStyle").unwrap_or("solid"),
                "dashed": bool_at(it, "dashed", false)
            });
            if let Some(f) = str_at(it, "fill") {
                b.as_object_mut().unwrap().insert("fill".into(), json!(f));
            }
            out.push(b);
        }
        out.push(json!({
            "t": "text", "x": x + 44.0, "y": y - 1.0,
            "text": str_at(it, "text").unwrap_or(""), "size": 15.0, "color": "muted"
        }));
        y += 30.0;
    }
    Ok(out)
}

fn m_db(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("db: missing 'id'")?;
    let x = req_f64(e, "x", "db")?;
    let y = req_f64(e, "y", "db")?;
    let w = req_f64(e, "w", "db")?;
    let h = req_f64(e, "h", "db")?;
    let ry = (h * 0.16).min(16.0);
    let c = str_at(e, "color").unwrap_or("ink");
    let fill = str_at(e, "fill").unwrap_or("soft");
    let fs = str_at(e, "fillStyle").unwrap_or("solid");
    let weight = f64_at(e, "weight", 2.0);
    let dashed = bool_at(e, "dashed", false);

    let mut than: Vec<[f64; 2]> = vec![[x + w, y + ry]];
    for a in (0..=180).step_by(15) {
        let rad = (a as f64).to_radians();
        than.push([x + w / 2.0 + (w / 2.0) * rad.cos(), y + h - ry + ry * rad.sin()]);
    }
    than.push([x, y + ry]);

    let mut out = vec![
        json!({
            "t": "line", "points": than, "closed": true, "fill": fill, "color": c,
            "fillStyle": fs, "weight": weight, "dashed": dashed, "smooth": false
        }),
        json!({
            "t": "ellipse", "x": x, "y": y, "w": w, "h": ry * 2.0, "color": c,
            "fill": fill, "fillStyle": fs, "weight": weight, "dashed": dashed
        }),
        json!({ "t": "anchor", "id": id, "x": x, "y": y, "w": w, "h": h }),
    ];

    let kind = str_at(e, "kind");
    let by0 = y + ry * 2.0;
    let bh = h - ry * 3.0;
    match kind {
        Some("sql") => {
            for k in [1.0, 2.0] {
                let yy = y + ry + (h - 2.0 * ry) * k / 3.0;
                let pts: Vec<[f64; 2]> = (0..=180)
                    .step_by(20)
                    .map(|a| {
                        let rad = (a as f64).to_radians();
                        [x + w / 2.0 + (w / 2.0) * rad.cos(), yy + ry * rad.sin()]
                    })
                    .collect();
                out.push(json!({ "t": "line", "color": c, "weight": 1.0, "smooth": true, "points": pts }));
            }
        }
        Some("col") => {
            for k in 1..5 {
                let xx = x + w * k as f64 / 5.0;
                out.push(json!({ "t": "line", "color": c, "weight": 1.0, "points": [[xx, by0 + 4.0], [xx, by0 + bh - 2.0]] }));
            }
        }
        Some("ts") => {
            let pts: Vec<[f64; 2]> = [(0.15, 0.7), (0.32, 0.35), (0.5, 0.6), (0.68, 0.2), (0.85, 0.5)]
                .iter()
                .map(|&(px, py)| [x + w * px, by0 + bh * py])
                .collect();
            out.push(json!({ "t": "line", "color": c, "weight": 2.0, "smooth": false, "points": pts }));
        }
        Some(k @ ("doc" | "kv" | "blob" | "search" | "vector" | "graph")) => {
            let g = match k {
                "doc" => "{ }",
                "kv" => "k → v",
                "blob" => "▲ ◆ ●",
                "search" => "a-z ?",
                "vector" => "[.1 .7]",
                _ => "o-o",
            };
            out.push(ghost_box(x, by0, w, bh, g, 15.0, c));
        }
        _ => {}
    }

    if let Some(txt) = str_at(e, "text") {
        let s = f64_at(e, "size", 17.0);
        if kind.is_some() || str_at(e, "labelPos") == Some("below") {
            out.push(json!({
                "t": "text", "x": x + w / 2.0 - rong_chu(txt, s) / 2.0, "y": y + h + 6.0,
                "text": txt, "size": s, "color": c
            }));
        } else {
            out.push(ghost_box(x, by0, w, bh, txt, s, c));
        }
    }
    Ok(out)
}

fn m_pipe(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("pipe: missing 'id'")?;
    let x = req_f64(e, "x", "pipe")?;
    let y = req_f64(e, "y", "pipe")?;
    let w = req_f64(e, "w", "pipe")?;
    let h = req_f64(e, "h", "pipe")?;
    let c = str_at(e, "color").unwrap_or("ink");
    let rx = (w * 0.08).min(14.0);
    let f = str_at(e, "fill").unwrap_or("soft");

    let cung = |cx: f64, a0: i32, a1: i32| -> Vec<[f64; 2]> {
        (a0..=a1)
            .step_by(15)
            .map(|a| {
                let rad = (a as f64).to_radians();
                [cx + rx * rad.cos(), y + h / 2.0 + (h / 2.0) * rad.sin()]
            })
            .collect()
    };

    let mut than = vec![[x + rx, y], [x + w - rx, y]];
    than.extend(cung(x + w - rx, -90, 90));
    than.push([x + rx, y + h]);
    than.extend(cung(x + rx, 90, 270));

    let mut out = vec![
        json!({
            "t": "line", "points": than, "closed": true, "smooth": false,
            "color": c, "fill": f, "dashed": bool_at(e, "dashed", false)
        }),
        json!({ "t": "ellipse", "x": x, "y": y, "w": rx * 2.0, "h": h, "color": c, "fill": f }),
        json!({ "t": "anchor", "id": id, "x": x, "y": y, "w": w, "h": h }),
    ];

    let n = f64_at(e, "segments", 0.0) as usize;
    for k in 1..n {
        let xx = x + rx + (w - 2.0 * rx) * k as f64 / n as f64;
        out.push(json!({
            "t": "line", "color": c, "weight": 1.0, "smooth": true, "points": cung(xx, -90, 90)
        }));
    }
    if let Some(txt) = str_at(e, "text") {
        let s = f64_at(e, "size", 16.0);
        if n > 0 || str_at(e, "labelPos") == Some("above") {
            out.push(json!({
                "t": "text", "x": x + 4.0, "y": y - 26.0, "text": txt, "size": s, "color": c
            }));
        } else {
            out.push(ghost_box(x + rx * 2.0, y, w - rx * 3.0, h, txt, s, c));
        }
    }
    Ok(out)
}

fn m_steps(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "steps")?;
    let mut y = req_f64(e, "y", "steps")?;
    let s = f64_at(e, "size", 16.0);
    let c = str_at(e, "color").unwrap_or("ink");
    let nc = str_at(e, "numColor").unwrap_or("muted");
    let start = f64_at(e, "start", 1.0) as i64;
    let items = e
        .get("items")
        .and_then(Value::as_array)
        .ok_or("steps: missing 'items'")?;
    let mut out = Vec::new();
    if let Some(title) = str_at(e, "title") {
        out.push(json!({ "t": "text", "x": x, "y": y, "text": title, "size": s, "color": "muted" }));
        y += s * 1.7;
    }
    for (i, it) in items.iter().enumerate() {
        let (txt, ic) = match it {
            Value::String(st) => (st.as_str(), c),
            Value::Object(_) => (str_at(it, "text").unwrap_or(""), str_at(it, "color").unwrap_or(c)),
            _ => ("", c),
        };
        out.push(json!({
            "t": "text", "x": x, "y": y, "text": format!("{}.", i as i64 + start), "size": s, "color": nc
        }));
        out.push(json!({
            "t": "text", "x": x + s * 1.5, "y": y, "text": txt, "size": s, "color": ic
        }));
        y += cao_chu(txt, s) + s * 0.45;
    }
    Ok(out)
}

fn m_card(e: &Value) -> Result<Vec<Value>, String> {
    let x = req_f64(e, "x", "card")?;
    let y = req_f64(e, "y", "card")?;
    let s = f64_at(e, "size", 15.0);
    let c = str_at(e, "color").unwrap_or("l2");
    let body = str_at(e, "text").unwrap_or("");
    let title = str_at(e, "title");
    let mono = bool_at(e, "mono", false);
    let w = e.get("w").and_then(Value::as_f64).unwrap_or_else(|| {
        rong_chu(title.unwrap_or(""), 17.0)
            .max(rong_chu(body, s) * if mono { 1.17 } else { 1.0 })
            + 28.0
    });
    let h = e.get("h").and_then(Value::as_f64).unwrap_or_else(|| {
        (if title.is_some() { 34.0 } else { 10.0 }) + cao_chu(body, s) + 14.0
    });
    let mut b = json!({
        "t": "box", "x": x, "y": y, "w": w, "h": h, "color": c,
        "weight": 1.0, "dashed": bool_at(e, "dashed", false)
    });
    if let Some(id) = str_at(e, "id") {
        b.as_object_mut().unwrap().insert("id".into(), json!(id));
    }
    if let Some(f) = str_at(e, "fill") {
        b.as_object_mut().unwrap().insert("fill".into(), json!(f));
    }
    let mut out = vec![b];
    let mut yy = y + 8.0;
    if let Some(ti) = title {
        out.push(json!({
            "t": "text", "x": x + 14.0, "y": yy, "text": ti, "size": 17.0,
            "color": str_at(e, "titleColor").unwrap_or(c)
        }));
        out.push(json!({
            "t": "line", "points": [[x + 12.0, yy + 26.0], [x + w - 12.0, yy + 26.0]],
            "color": c, "weight": 1.0
        }));
        yy += 34.0;
    }
    if !body.is_empty() {
        out.push(json!({
            "t": "text", "x": x + 14.0, "y": yy, "text": body, "size": s,
            "color": str_at(e, "textColor").unwrap_or("ink"), "mono": mono
        }));
    }
    Ok(out)
}

fn m_queue(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("queue: missing 'id'")?;
    let x = req_f64(e, "x", "queue")?;
    let y = req_f64(e, "y", "queue")?;
    let w = req_f64(e, "w", "queue")?;
    let h = req_f64(e, "h", "queue")?;
    let c = str_at(e, "color").unwrap_or("ink");
    let mut out = vec![json!({
        "t": "box", "id": id, "x": x, "y": y, "w": w, "h": h,
        "color": c, "fill": str_at(e, "fill").unwrap_or("soft")
    })];
    let items: Vec<Value> = match e.get("items").and_then(Value::as_array) {
        Some(arr) => arr.clone(),
        None => vec![json!({}); f64_at(e, "n", 4.0) as usize],
    };
    let mw = f64_at(e, "mw", 22.0);
    let pad = 8.0;
    let tot = items.len() as f64 * (mw + 6.0);
    let sx = x + w - pad - tot;
    for (i, it) in items.iter().enumerate() {
        if it.is_null() {
            continue;
        }
        let ic = match it {
            Value::String(s) => s.as_str(),
            Value::Object(_) => str_at(it, "color").unwrap_or(c),
            _ => c,
        };
        let mut cell = json!({
            "t": "box", "x": sx + i as f64 * (mw + 6.0), "y": y + 8.0, "w": mw, "h": h - 16.0,
            "color": ic, "fill": if is_accent(ic) { ic } else { "soft" },
            "fillStyle": "hachure", "sharp": true, "weight": 1.0, "size": 15.0
        });
        if let Some(txt) = str_at(it, "text") {
            cell.as_object_mut().unwrap().insert("text".into(), json!(txt));
        }
        out.push(cell);
    }
    if let Some(txt) = str_at(e, "text") {
        if str_at(e, "labelPos") == Some("above") || sx - x < rong_chu(txt, 16.0) + 14.0 {
            out.push(json!({ "t": "text", "x": x + 4.0, "y": y - 26.0, "text": txt, "size": 16.0, "color": c }));
        } else {
            out.push(ghost_box(x + 4.0, y, sx - x - 8.0, h, txt, 16.0, c));
        }
    }
    Ok(out)
}

fn m_person(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("person: missing 'id'")?;
    let x = req_f64(e, "x", "person")?;
    let y = req_f64(e, "y", "person")?;
    let h = f64_at(e, "h", 70.0);
    let c = str_at(e, "color").unwrap_or("ink");
    let w = h * 0.5;
    let r = h * 0.2;
    let cx = x + w / 2.0;
    let mut head = json!({ "t": "ellipse", "x": cx - r, "y": y, "w": 2.0 * r, "h": 2.0 * r, "color": c });
    if let Some(f) = str_at(e, "fill") {
        head.as_object_mut().unwrap().insert("fill".into(), json!(f));
    }
    let mut out = vec![
        head,
        json!({ "t": "line", "points": [[cx, y + 2.0 * r], [cx, y + h * 0.7]], "color": c }),
        json!({ "t": "line", "points": [[x, y + h * 0.5], [x + w, y + h * 0.5]], "color": c }),
        json!({ "t": "line", "points": [[x + 2.0, y + h], [cx, y + h * 0.7], [x + w - 2.0, y + h]], "color": c }),
        json!({ "t": "anchor", "id": id, "x": x - 6.0, "y": y, "w": w + 12.0, "h": h }),
    ];
    if let Some(txt) = str_at(e, "text") {
        let s = f64_at(e, "size", 16.0);
        out.push(json!({
            "t": "text", "x": cx - rong_chu(txt, s) / 2.0, "y": y + h + 6.0,
            "text": txt, "size": s, "color": c
        }));
    }
    Ok(out)
}

fn m_cloud(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("cloud: missing 'id'")?;
    let x = req_f64(e, "x", "cloud")?;
    let y = req_f64(e, "y", "cloud")?;
    let w = req_f64(e, "w", "cloud")?;
    let h = req_f64(e, "h", "cloud")?;
    let c = str_at(e, "color").unwrap_or("ink");
    let pts: Vec<[f64; 2]> = [
        (0.18, 0.82), (0.05, 0.62), (0.12, 0.38), (0.28, 0.32), (0.36, 0.12),
        (0.56, 0.08), (0.68, 0.26), (0.86, 0.28), (0.96, 0.50), (0.88, 0.76),
        (0.70, 0.84), (0.50, 0.92), (0.30, 0.88),
    ]
    .iter()
    .map(|&(px, py)| [x + px * w, y + py * h])
    .collect();
    let mut out = vec![
        json!({
            "t": "line", "points": pts, "closed": true, "smooth": true,
            "color": c, "fill": str_at(e, "fill").unwrap_or("soft"),
            "dashed": bool_at(e, "dashed", false)
        }),
        json!({
            "t": "anchor", "id": id, "x": x + w * 0.05, "y": y + h * 0.1,
            "w": w * 0.9, "h": h * 0.8, "shape": "ellipse"
        }),
    ];
    if let Some(txt) = str_at(e, "text") {
        out.push(ghost_box(
            x + w * 0.15,
            y + h * 0.3,
            w * 0.7,
            h * 0.5,
            txt,
            f64_at(e, "size", 17.0),
            c,
        ));
    }
    Ok(out)
}

fn m_doc(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("doc: missing 'id'")?;
    let x = req_f64(e, "x", "doc")?;
    let y = req_f64(e, "y", "doc")?;
    let w = req_f64(e, "w", "doc")?;
    let h = req_f64(e, "h", "doc")?;
    let c = str_at(e, "color").unwrap_or("ink");
    let k = w.min(h) * 0.26;
    let mut out = vec![
        json!({
            "t": "line",
            "points": [[x, y], [x + w - k, y], [x + w, y + k], [x + w, y + h], [x, y + h]],
            "closed": true, "color": c, "fill": str_at(e, "fill").unwrap_or("soft")
        }),
        json!({
            "t": "line",
            "points": [[x + w - k, y], [x + w - k, y + k], [x + w, y + k]],
            "color": c
        }),
        json!({ "t": "anchor", "id": id, "x": x, "y": y, "w": w, "h": h }),
    ];
    if let Some(txt) = str_at(e, "text") {
        out.push(ghost_box(
            x,
            y + k * 0.6,
            w,
            h - k * 0.6,
            txt,
            f64_at(e, "size", 16.0),
            c,
        ));
    }
    Ok(out)
}

fn m_window(e: &Value) -> Result<Vec<Value>, String> {
    let id = str_at(e, "id").ok_or("window: missing 'id'")?;
    let x = req_f64(e, "x", "window")?;
    let y = req_f64(e, "y", "window")?;
    let w = req_f64(e, "w", "window")?;
    let h = req_f64(e, "h", "window")?;
    let c = str_at(e, "color").unwrap_or("ink");
    let mut b = json!({ "t": "box", "id": id, "x": x, "y": y, "w": w, "h": h, "color": c });
    if let Some(f) = str_at(e, "fill") {
        b.as_object_mut().unwrap().insert("fill".into(), json!(f));
    }
    let mut out = vec![
        b,
        json!({ "t": "line", "points": [[x, y + 24.0], [x + w, y + 24.0]], "color": c, "weight": 1.0 }),
    ];
    for i in 0..3 {
        out.push(json!({
            "t": "ellipse", "x": x + 10.0 + i as f64 * 14.0, "y": y + 8.0,
            "w": 8.0, "h": 8.0, "color": c, "weight": 1.0
        }));
    }
    if let Some(title) = str_at(e, "title") {
        out.push(json!({
            "t": "text", "id": format!("{id}.title"), "x": x + 60.0, "y": y + 3.0,
            "text": title, "size": 15.0, "color": "muted"
        }));
    }
    if let Some(txt) = str_at(e, "text") {
        out.push(ghost_box(
            x,
            y + 24.0,
            w,
            h - 24.0,
            txt,
            f64_at(e, "size", 16.0),
            c,
        ));
    }
    Ok(out)
}

fn m_group(e: &Value, icons: &HashMap<String, Value>) -> Result<Vec<Value>, String> {
    let kids = e
        .get("elements")
        .and_then(Value::as_array)
        .ok_or("group: missing 'elements'")?;
    let dx = f64_at(e, "dx", 0.0);
    let dy = f64_at(e, "dy", 0.0);
    let dim = bool_at(e, "dim", false);
    let prefix = str_at(e, "prefix");
    let mut out = Vec::new();

    for mut c in expand_all(kids, icons)? {
        if let Some(obj) = c.as_object_mut() {
            for k in ["x", "x1", "x2", "cx"] {
                if let Some(v) = obj.get(k).and_then(Value::as_f64) {
                    obj.insert(k.into(), json!(v + dx));
                }
            }
            for k in ["y", "cy"] {
                if let Some(v) = obj.get(k).and_then(Value::as_f64) {
                    obj.insert(k.into(), json!(v + dy));
                }
            }
            if let Some(pts) = obj.get_mut("points").and_then(Value::as_array_mut) {
                for p in pts.iter_mut() {
                    if let Some(arr) = p.as_array_mut() {
                        if let (Some(px), Some(py)) = (
                            arr.first().and_then(Value::as_f64),
                            arr.get(1).and_then(Value::as_f64),
                        ) {
                            arr[0] = json!(px + dx);
                            arr[1] = json!(py + dy);
                        }
                    }
                }
            }
            if obj.get("t").and_then(Value::as_str) == Some("raw") {
                if let Some(raw_els) = obj.get_mut("elements").and_then(Value::as_array_mut) {
                    for r in raw_els.iter_mut() {
                        if let Some(ro) = r.as_object_mut() {
                            let rx = ro.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                            let ry = ro.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                            ro.insert("x".into(), json!(rx + dx));
                            ro.insert("y".into(), json!(ry + dy));
                            if dim {
                                if let Some(sc) = ro.get("strokeColor").and_then(Value::as_str) {
                                    if sc != "transparent" {
                                        ro.insert("strokeColor".into(), json!(token_light("muted").unwrap()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if dim {
                obj.insert("color".into(), json!("muted"));
                if let Some(f) = obj.get("fill").and_then(Value::as_str) {
                    if !f.is_empty() && f != "code" {
                        obj.insert("fill".into(), json!("soft"));
                    }
                }
            }
            if let Some(pre) = prefix {
                for k in ["id", "from", "to"] {
                    if let Some(s) = obj.get(k).and_then(Value::as_str).map(str::to_owned) {
                        if !s.is_empty() {
                            obj.insert(k.into(), json!(format!("{pre}{s}")));
                        }
                    }
                }
            }
        }
        out.push(c);
    }
    Ok(out)
}

fn m_row_col(e: &Value, icons: &HashMap<String, Value>) -> Result<Vec<Value>, String> {
    let col = str_at(e, "t") == Some("col");
    let ex = req_f64(e, "x", "row/col")?;
    let ey = req_f64(e, "y", "row/col")?;
    let gap = f64_at(e, "gap", 60.0);
    let align_center = str_at(e, "align").unwrap_or("center") == "center";
    let kids = e
        .get("elements")
        .and_then(Value::as_array)
        .ok_or("row/col: missing 'elements'")?;
    let lon = kids
        .iter()
        .map(|k| f64_at(k, if col { "w" } else { "h" }, 0.0))
        .fold(0.0_f64, f64::max);
    let mut chay = if col { ey } else { ex };
    let mut positioned = Vec::with_capacity(kids.len());

    for k in kids {
        let mut kc = k.clone();
        let kw = f64_at(k, "w", 0.0);
        let kh = f64_at(k, "h", 0.0);
        let obj = kc.as_object_mut().unwrap();
        if col {
            obj.insert("y".into(), json!(chay));
            obj.insert("x".into(), json!(if align_center { ex + (lon - kw) / 2.0 } else { ex }));
            chay += kh + gap;
        } else {
            obj.insert("x".into(), json!(chay));
            obj.insert("y".into(), json!(if align_center { ey + (lon - kh) / 2.0 } else { ey }));
            chay += kw + gap;
        }
        positioned.push(kc);
    }
    expand_all(&positioned, icons)
}

fn m_icon(e: &Value, icons: &HashMap<String, Value>) -> Result<Vec<Value>, String> {
    let name = str_at(e, "name").ok_or("icon: missing 'name'")?.trim().to_lowercase();
    let ic = icons
        .get(&name)
        .ok_or_else(|| format!("icon '{name}' not found in icon library"))?;
    let ex = req_f64(e, "x", "icon")?;
    let ey = req_f64(e, "y", "icon")?;
    let keep_text = bool_at(e, "keepText", false);
    let src_els = ic
        .get("elements")
        .and_then(Value::as_array)
        .ok_or("icon: missing elements")?;

    let els: Vec<Value> = src_els
        .iter()
        .filter(|x| {
            let ty = str_at(x, "type").unwrap_or("");
            matches!(ty, "rectangle" | "ellipse" | "diamond" | "line" | "arrow")
                || (keep_text && ty == "text")
        })
        .cloned()
        .collect();
    if els.is_empty() {
        return Err(format!("icon '{name}' has no drawable elements"));
    }

    let mut x0 = f64::INFINITY;
    let mut y0 = f64::INFINITY;
    let mut x1 = f64::NEG_INFINITY;
    let mut y1 = f64::NEG_INFINITY;
    for x in &els {
        let bx = f64_at(x, "x", 0.0);
        let by = f64_at(x, "y", 0.0);
        if let Some(pts) = x.get("points").and_then(Value::as_array) {
            for p in pts {
                if let Some(arr) = p.as_array() {
                    let px = bx + arr.first().and_then(Value::as_f64).unwrap_or(0.0);
                    let py = by + arr.get(1).and_then(Value::as_f64).unwrap_or(0.0);
                    x0 = x0.min(px);
                    x1 = x1.max(px);
                    y0 = y0.min(py);
                    y1 = y1.max(py);
                }
            }
        } else {
            let bw = f64_at(x, "width", 0.0).abs();
            let bh = f64_at(x, "height", 0.0).abs();
            x0 = x0.min(bx);
            x1 = x1.max(bx + bw);
            y0 = y0.min(by);
            y1 = y1.max(by + bh);
        }
    }
    let w0 = (x1 - x0).max(1.0);
    let h0 = (y1 - y0).max(1.0);
    let s = if let Some(ew) = e.get("w").and_then(Value::as_f64) {
        ew / w0
    } else {
        f64_at(e, "h", 56.0) / h0
    };
    let w = w0 * s;
    let h = h0 * s;
    let c = str_at(e, "color").unwrap_or("ink");
    let stroke_hex = resolve_color(c)?;
    let fill_hex = resolve_fill(Some(str_at(e, "fill").unwrap_or("soft")))?;

    let mut raw = Vec::with_capacity(els.len());
    for mut x in els {
        if let Some(obj) = x.as_object_mut() {
            let ox = obj.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let oy = obj.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            obj.insert("x".into(), json!(ex + (ox - x0) * s));
            obj.insert("y".into(), json!(ey + (oy - y0) * s));
            let ow = obj.get("width").and_then(Value::as_f64).unwrap_or(0.0);
            let oh = obj.get("height").and_then(Value::as_f64).unwrap_or(0.0);
            obj.insert("width".into(), json!(ow * s));
            obj.insert("height".into(), json!(oh * s));
            if let Some(pts) = obj.get_mut("points").and_then(Value::as_array_mut) {
                for p in pts.iter_mut() {
                    if let Some(arr) = p.as_array_mut() {
                        let px = arr.first().and_then(Value::as_f64).unwrap_or(0.0);
                        let py = arr.get(1).and_then(Value::as_f64).unwrap_or(0.0);
                        arr[0] = json!(px * s);
                        arr[1] = json!(py * s);
                    }
                }
            }
            if let Some(fs) = obj.get("fontSize").and_then(Value::as_f64) {
                obj.insert("fontSize".into(), json!((fs * s).max(10.0)));
            }
            if obj.get("strokeColor").and_then(Value::as_str).map(|sc| sc.to_lowercase()) != Some("transparent".into()) {
                obj.insert("strokeColor".into(), json!(stroke_hex));
            }
            if obj.get("backgroundColor").and_then(Value::as_str).map(|bg| bg.to_lowercase()) != Some("transparent".into()) {
                obj.insert("backgroundColor".into(), json!(fill_hex));
            }
            let sw = obj.get("strokeWidth").and_then(Value::as_f64).unwrap_or(2.0);
            obj.insert("strokeWidth".into(), json!((sw * s.sqrt()).clamp(1.0, 2.0)));
            obj.insert("roughness".into(), json!(1));
            obj.insert("opacity".into(), json!(100));
        }
        raw.push(x);
    }

    let aid = str_at(e, "id")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("icon@{ex},{ey}"));
    let mut out = vec![
        json!({ "t": "raw", "elements": raw }),
        json!({ "t": "anchor", "id": aid, "x": ex, "y": ey, "w": w, "h": h }),
    ];
    if let Some(txt) = str_at(e, "text") {
        let sz = f64_at(e, "size", 16.0);
        out.push(json!({
            "t": "text", "x": ex + w / 2.0, "y": ey + h + 6.0, "align": "center",
            "text": txt, "size": sz, "color": c
        }));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Excalidraw scene element model (compiled from skeleton)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct SceneEl {
    id: String,
    el_type: String, // rectangle | ellipse | diamond | line | arrow | text
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    stroke_color: String,
    background_color: String,
    fill_style: String,   // solid | hachure | cross-hatch
    stroke_width: f64,
    stroke_style: String, // solid | dashed | dotted
    roughness: f64,
    opacity: f64,
    roundness: Option<u8>, // 2 = proportional/smooth, 3 = adaptive rounded rect
    points: Vec<(f64, f64)>,
    closed: bool,
    start_arrowhead: Option<String>,
    end_arrowhead: Option<String>,
    text: Option<String>,
    font_size: f64,
    mono: bool,
    text_align: String,     // left | center | right
    vertical_align: String, // top | middle | bottom
    container_id: Option<String>,
    seed: u32,
}

#[derive(Debug, Clone)]
struct AnchorInfo {
    id: String,
    t: String,
    shape: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    ghost: bool,
    under: bool,
}

#[derive(Debug, Clone)]
struct LintArrow {
    from: Option<String>,
    to: Option<String>,
    pts: Vec<(f64, f64)>,
    head_count: usize,
}

fn compile_scene(
    spec_els: &[Value],
    icons: &HashMap<String, Value>,
) -> Result<(Vec<SceneEl>, HashMap<String, AnchorInfo>, Vec<LintArrow>, Vec<Value>), String> {
    let els = expand_all(spec_els, icons)?;
    let mut hinh: HashMap<String, AnchorInfo> = HashMap::new();
    for e in &els {
        let t = str_at(e, "t").unwrap_or("");
        if (is_shape_t(t) || t == "anchor") && str_at(e, "id").is_some_and(|s| !s.is_empty()) {
            let id = str_at(e, "id").unwrap().to_owned();
            if hinh.contains_key(&id) && t != "anchor" {
                return Err(format!("duplicate id '{id}'"));
            }
            hinh.entry(id.clone()).or_insert_with(|| AnchorInfo {
                id,
                t: t.to_owned(),
                shape: str_at(e, "shape").unwrap_or(t).to_owned(),
                x: f64_at(e, "x", 0.0),
                y: f64_at(e, "y", 0.0),
                w: f64_at(e, "w", 0.0),
                h: f64_at(e, "h", 0.0),
                ghost: bool_at(e, "ghost", false),
                under: bool_at(e, "under", false),
            });
        }
    }

    let mut ordered: Vec<&Value> = Vec::with_capacity(els.len());
    for e in &els {
        if bool_at(e, "under", false) {
            ordered.push(e);
        }
    }
    for e in &els {
        if !bool_at(e, "under", false) {
            ordered.push(e);
        }
    }

    let mut scene: Vec<SceneEl> = Vec::new();
    let mut raw_els: Vec<SceneEl> = Vec::new();
    let mut lint_arrows: Vec<LintArrow> = Vec::new();
    let warn_hex = token_light("warn").unwrap();
    let ink_hex = token_light("ink").unwrap();
    let paper_hex = token_light("paper").unwrap();
    let line_hex = token_light("line").unwrap();

    let safe_text_color = |hex: &str| -> String {
        if hex.eq_ignore_ascii_case(warn_hex) {
            ink_hex.to_owned()
        } else {
            hex.to_owned()
        }
    };

    for (i, e) in ordered.iter().enumerate() {
        let t = str_at(e, "t").unwrap_or("");
        if t == "anchor" {
            continue;
        }
        if t == "raw" {
            if let Some(arr) = e.get("elements").and_then(Value::as_array) {
                for (k, r) in arr.iter().enumerate() {
                    let ty = str_at(r, "type").unwrap_or("rectangle");
                    let pts: Vec<(f64, f64)> = r
                        .get("points")
                        .and_then(Value::as_array)
                        .map(|pa| {
                            pa.iter()
                                .filter_map(|p| {
                                    let a = p.as_array()?;
                                    Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let closed = ty == "line"
                        && pts.len() >= 3
                        && (pts.first().unwrap().0 - pts.last().unwrap().0).abs() < 1e-3
                        && (pts.first().unwrap().1 - pts.last().unwrap().1).abs() < 1e-3;
                    raw_els.push(SceneEl {
                        id: format!("ic{}_{}", raw_els.len(), k),
                        el_type: ty.to_owned(),
                        x: f64_at(r, "x", 0.0),
                        y: f64_at(r, "y", 0.0),
                        width: f64_at(r, "width", 0.0),
                        height: f64_at(r, "height", 0.0),
                        stroke_color: str_at(r, "strokeColor").unwrap_or(ink_hex).to_owned(),
                        background_color: str_at(r, "backgroundColor").unwrap_or("transparent").to_owned(),
                        fill_style: str_at(r, "fillStyle").unwrap_or("solid").to_owned(),
                        stroke_width: f64_at(r, "strokeWidth", 2.0),
                        stroke_style: str_at(r, "strokeStyle").unwrap_or("solid").to_owned(),
                        roughness: f64_at(r, "roughness", 1.0),
                        opacity: f64_at(r, "opacity", 100.0),
                        roundness: r
                            .get("roundness")
                            .and_then(|rn| rn.get("type"))
                            .and_then(Value::as_u64)
                            .map(|v| v as u8),
                        points: pts,
                        closed,
                        start_arrowhead: str_at(r, "startArrowhead").map(str::to_owned),
                        end_arrowhead: if ty == "arrow" {
                            Some(str_at(r, "endArrowhead").unwrap_or("arrow").to_owned())
                        } else {
                            None
                        },
                        text: str_at(r, "text").map(str::to_owned),
                        font_size: f64_at(r, "fontSize", 16.0),
                        mono: false,
                        text_align: str_at(r, "textAlign").unwrap_or("left").to_owned(),
                        vertical_align: "top".into(),
                        container_id: None,
                        seed: 1000 + (raw_els.len() as u32) * 37,
                    });
                }
            }
            continue;
        }

        let color_tok = str_at(e, "color").unwrap_or("ink");
        let c = resolve_color(color_tok)?;
        let ghost = bool_at(e, "ghost", false);
        let stroke_color = if ghost { "transparent".to_owned() } else { c.clone() };
        let stroke_width = f64_at(e, "weight", 2.0);
        let roughness = f64_at(e, "rough", 1.0);
        let opacity = f64_at(e, "opacity", 100.0);
        let stroke_style = if bool_at(e, "dashed", false) {
            "dashed"
        } else if bool_at(e, "dotted", false) {
            "dotted"
        } else {
            "solid"
        }
        .to_owned();

        if is_shape_t(t) {
            let id = str_at(e, "id")
                .map(str::to_owned)
                .unwrap_or_else(|| format!("_s{i}"));
            let x = req_f64(e, "x", t)?;
            let y = req_f64(e, "y", t)?;
            let w = req_f64(e, "w", t)?;
            let h = req_f64(e, "h", t)?;
            let fill_tok = str_at(e, "fill");
            let mut bg = resolve_fill(fill_tok)?;
            let fill_style = str_at(e, "fillStyle").unwrap_or("solid").to_owned();
            let el_type = if t == "box" { "rectangle" } else { t }.to_owned();
            let roundness = if t == "box" && !bool_at(e, "sharp", false) {
                Some(3)
            } else {
                None
            };
            let label_txt = str_at(e, "text").filter(|s| !s.is_empty());

            // _hachure_ro port: visible hachure on soft / narrow shapes
            if matches!(fill_style.as_str(), "hachure" | "cross-hatch") && !ghost {
                if let Some(ft) = fill_tok {
                    if ft == "soft" {
                        bg = line_hex.to_owned();
                    } else if w.min(h) < HACH_HEP && label_txt.is_none() {
                        if let Some(sat) = token_light(ft) {
                            scene.push(SceneEl {
                                id: format!("{id}~hach"),
                                el_type: el_type.clone(),
                                x,
                                y,
                                width: w,
                                height: h,
                                stroke_color: "transparent".into(),
                                background_color: sat.to_owned(),
                                fill_style: fill_style.clone(),
                                stroke_width: 1.5,
                                stroke_style: stroke_style.clone(),
                                roughness,
                                opacity: 55.0,
                                roundness,
                                points: Vec::new(),
                                closed: false,
                                start_arrowhead: None,
                                end_arrowhead: None,
                                text: None,
                                font_size: 20.0,
                                mono: false,
                                text_align: "center".into(),
                                vertical_align: "middle".into(),
                                container_id: None,
                                seed: 1000 + scene.len() as u32 * 37,
                            });
                            bg = "transparent".into();
                        }
                    }
                }
            }

            scene.push(SceneEl {
                id: id.clone(),
                el_type,
                x,
                y,
                width: w,
                height: h,
                stroke_color,
                background_color: bg,
                fill_style,
                stroke_width,
                stroke_style,
                roughness,
                opacity,
                roundness,
                points: Vec::new(),
                closed: false,
                start_arrowhead: None,
                end_arrowhead: None,
                text: None,
                font_size: 20.0,
                mono: false,
                text_align: "center".into(),
                vertical_align: "middle".into(),
                container_id: None,
                seed: 1000 + scene.len() as u32 * 37,
            });

            if let Some(txt) = label_txt {
                let fs = f64_at(e, "size", 20.0);
                let tw = rong_chu(txt, fs);
                let th = cao_chu(txt, fs);
                let align = str_at(e, "align").unwrap_or("center").to_owned();
                let valign = str_at(e, "valign").unwrap_or("middle").to_owned();
                let tx = match align.as_str() {
                    "left" => x + 10.0,
                    "right" => x + w - 10.0 - tw,
                    _ => x + (w - tw) / 2.0,
                };
                let ty = match valign.as_str() {
                    "top" => y + 8.0,
                    "bottom" => y + h - 8.0 - th,
                    _ => y + (h - th) / 2.0,
                };
                scene.push(SceneEl {
                    id: format!("{id}~txt"),
                    el_type: "text".into(),
                    x: tx,
                    y: ty,
                    width: tw,
                    height: th,
                    stroke_color: safe_text_color(&c),
                    background_color: "transparent".into(),
                    fill_style: "solid".into(),
                    stroke_width: 1.0,
                    stroke_style: "solid".into(),
                    roughness: 0.0,
                    opacity,
                    roundness: None,
                    points: Vec::new(),
                    closed: false,
                    start_arrowhead: None,
                    end_arrowhead: None,
                    text: Some(txt.to_owned()),
                    font_size: fs,
                    mono: false,
                    text_align: align,
                    vertical_align: valign,
                    container_id: Some(id),
                    seed: 1000 + scene.len() as u32 * 37,
                });
            }
        } else if t == "arrow" || t == "line" {
            let head_str = str_at(e, "head").unwrap_or("arrow");
            let tail_str = str_at(e, "tail").unwrap_or(if bool_at(e, "both", false) {
                "arrow"
            } else {
                "none"
            });
            let head_count = usize::from(head_str != "none") + usize::from(tail_str != "none");

            let mut force_sharp = bool_at(e, "sharp", false);
            let pts: Vec<(f64, f64)> = if let Some(arr) = e.get("points").and_then(Value::as_array) {
                let p: Vec<(f64, f64)> = arr
                    .iter()
                    .filter_map(|pt| {
                        let a = pt.as_array()?;
                        Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
                    })
                    .collect();
                if t == "arrow" {
                    lint_arrows.push(LintArrow {
                        from: None,
                        to: None,
                        pts: p.clone(),
                        head_count,
                    });
                }
                p
            } else {
                let from_id = str_at(e, "from").ok_or_else(|| format!("arrow {i}: missing 'from'"))?;
                let to_id = str_at(e, "to").ok_or_else(|| format!("arrow {i}: missing 'to'"))?;
                let a = hinh
                    .get(from_id)
                    .ok_or_else(|| format!("arrow {i}: from='{from_id}' matches no shape id"))?
                    .clone();
                let b = hinh
                    .get(to_id)
                    .ok_or_else(|| format!("arrow {i}: to='{to_id}' matches no shape id"))?
                    .clone();
                if from_id == to_id {
                    vec![
                        (a.x + a.w * 0.62, a.y - 4.0),
                        (a.x + a.w * 0.70, a.y - 34.0),
                        (a.x + a.w + 30.0, a.y - 30.0),
                        (a.x + a.w + 34.0, a.y + 6.0),
                        (a.x + a.w + 6.0, a.y + a.h * 0.3),
                    ]
                } else {
                    let ca = tam(a.x, a.y, a.w, a.h);
                    let cb = tam(b.x, b.y, b.w, b.h);
                    let mut p = match str_at(e, "fromSide") {
                        Some(fs) => Some(mep(a.x, a.y, a.w, a.h, fs, f64_at(e, "fromAt", 0.5))?),
                        None => None,
                    };
                    let mut q = match str_at(e, "toSide") {
                        Some(ts) => Some(mep(b.x, b.y, b.w, b.h, ts, f64_at(e, "toAt", 0.5))?),
                        None => None,
                    };
                    if p.is_none() {
                        let target = q.unwrap_or(cb);
                        p = Some(cat(a.x, a.y, a.w, a.h, &a.shape, target.0, target.1));
                    }
                    if q.is_none() {
                        let target = if str_at(e, "fromSide").is_some() {
                            p.unwrap()
                        } else {
                            ca
                        };
                        q = Some(cat(b.x, b.y, b.w, b.h, &b.shape, target.0, target.1));
                    }
                    let (mut p, mut q) = (p.unwrap(), q.unwrap());
                    let route = str_at(e, "route");
                    let routed = if let Some(r @ ("hv" | "vh" | "hvh" | "vhv")) = route {
                        let shift = f64_at(e, "shift", 0.0);
                        let mid = match r {
                            "hv" => vec![(q.0, p.1)],
                            "vh" => vec![(p.0, q.1)],
                            "hvh" => {
                                let mx = (p.0 + q.0) / 2.0 + shift;
                                vec![(mx, p.1), (mx, q.1)]
                            }
                            _ => {
                                let my = (p.1 + q.1) / 2.0 + shift;
                                vec![(p.0, my), (q.0, my)]
                            }
                        };
                        p = lui(p, mid[0], 6.0);
                        q = lui(q, *mid.last().unwrap(), 6.0);
                        let mut v = vec![p];
                        v.extend(mid);
                        v.push(q);
                        force_sharp = true;
                        v
                    } else {
                        p = lui(p, q, 6.0);
                        q = lui(q, p, 6.0);
                        let bend = f64_at(e, "bend", 0.0);
                        if bend.abs() > 1e-6 {
                            let l = (q.0 - p.0).hypot(q.1 - p.1).max(1.0);
                            let (nx, ny) = (-(q.1 - p.1) / l, (q.0 - p.0) / l);
                            vec![p, ((p.0 + q.0) / 2.0 + nx * bend, (p.1 + q.1) / 2.0 + ny * bend), q]
                        } else {
                            vec![p, q]
                        }
                    };
                    lint_arrows.push(LintArrow {
                        from: Some(from_id.to_owned()),
                        to: Some(to_id.to_owned()),
                        pts: routed.clone(),
                        head_count,
                    });
                    routed
                }
            };

            if pts.len() < 2 {
                continue;
            }
            if t == "arrow" {
                if let Some(aid) = str_at(e, "id") {
                    let (mx, my) = if pts.len() > 2 {
                        pts[pts.len() / 2]
                    } else {
                        ((pts[0].0 + pts[1].0) / 2.0, (pts[0].1 + pts[1].1) / 2.0)
                    };
                    hinh.insert(
                        aid.to_owned(),
                        AnchorInfo {
                            id: aid.to_owned(),
                            t: "anchor".into(),
                            shape: "box".into(),
                            x: mx - 8.0,
                            y: my - 8.0,
                            w: 16.0,
                            h: 16.0,
                            ghost: true,
                            under: false,
                        },
                    );
                }
            }

            let (x0, y0) = pts[0];
            let mut rel: Vec<(f64, f64)> = pts.iter().map(|&(px, py)| (px - x0, py - y0)).collect();
            let closed = t == "line" && bool_at(e, "closed", false);
            if closed {
                rel.push((0.0, 0.0));
            }
            let smooth = e
                .get("smooth")
                .and_then(Value::as_bool)
                .unwrap_or(pts.len() > 2 && !force_sharp && !closed);
            let roundness = if smooth { Some(2) } else { None };
            let bg = if closed {
                resolve_fill(str_at(e, "fill"))?
            } else {
                "transparent".into()
            };
            let fill_style = str_at(e, "fillStyle").unwrap_or("solid").to_owned();

            let mut end_arrow = if t == "arrow" {
                resolve_head(head_str)?.map(str::to_owned)
            } else {
                None
            };
            let mut start_arrow = if t == "arrow" {
                resolve_head(tail_str)?.map(str::to_owned)
            } else {
                None
            };

            // Solid ER end segments on dashed/dotted arrows
            let is_er_end = |s: &str| matches!(s, "one" | "many" | "one-many" | "zero-one" | "zero-many");
            let mut extra_er: Vec<SceneEl> = Vec::new();
            if t == "arrow" && stroke_style != "solid" {
                for (is_head, end_name, tip, prev) in [
                    (true, head_str, *pts.last().unwrap(), pts[pts.len() - 2]),
                    (false, tail_str, pts[0], pts[1]),
                ] {
                    if is_er_end(end_name) {
                        let l = (tip.0 - prev.0).hypot(tip.1 - prev.1).max(1.0);
                        let s = l.min(40.0);
                        let p = (tip.0 + (prev.0 - tip.0) / l * s, tip.1 + (prev.1 - tip.1) / l * s);
                        let (a0, a1) = if is_head { (p, tip) } else { (tip, p) };
                        let resolved = resolve_head(end_name)?.map(str::to_owned);
                        if is_head {
                            end_arrow = None;
                        } else {
                            start_arrow = None;
                        }
                        extra_er.push(SceneEl {
                            id: format!("_erend{i}{}", if is_head { "head" } else { "tail" }),
                            el_type: "arrow".into(),
                            x: a0.0,
                            y: a0.1,
                            width: (a1.0 - a0.0).abs(),
                            height: (a1.1 - a0.1).abs(),
                            stroke_color: stroke_color.clone(),
                            background_color: "transparent".into(),
                            fill_style: "solid".into(),
                            stroke_width,
                            stroke_style: "solid".into(),
                            roughness,
                            opacity,
                            roundness: None,
                            points: vec![(0.0, 0.0), (a1.0 - a0.0, a1.1 - a0.1)],
                            closed: false,
                            start_arrowhead: if is_head { None } else { resolved.clone() },
                            end_arrowhead: if is_head { resolved } else { None },
                            text: None,
                            font_size: 16.0,
                            mono: false,
                            text_align: "center".into(),
                            vertical_align: "middle".into(),
                            container_id: None,
                            seed: 1000 + scene.len() as u32 * 37 + 5,
                        });
                    }
                }
            }

            let min_dx = rel.iter().map(|p| p.0).fold(0.0_f64, f64::min);
            let max_dx = rel.iter().map(|p| p.0).fold(0.0_f64, f64::max);
            let min_dy = rel.iter().map(|p| p.1).fold(0.0_f64, f64::min);
            let max_dy = rel.iter().map(|p| p.1).fold(0.0_f64, f64::max);

            let arrow_id = str_at(e, "id")
                .map(str::to_owned)
                .unwrap_or_else(|| format!("_{t}{i}"));

            scene.push(SceneEl {
                id: arrow_id.clone(),
                el_type: t.to_owned(),
                x: x0,
                y: y0,
                width: (max_dx - min_dx).max(0.0),
                height: (max_dy - min_dy).max(0.0),
                stroke_color: stroke_color.clone(),
                background_color: bg,
                fill_style,
                stroke_width,
                stroke_style: stroke_style.clone(),
                roughness,
                opacity,
                roundness,
                points: rel,
                closed,
                start_arrowhead: start_arrow,
                end_arrowhead: end_arrow,
                text: None,
                font_size: 16.0,
                mono: false,
                text_align: "center".into(),
                vertical_align: "middle".into(),
                container_id: None,
                seed: 1000 + scene.len() as u32 * 37,
            });
            scene.extend(extra_er);

            // Hollow ring for zero-one / zero-many ER ends
            if t == "arrow" {
                for (is_head, end_name, tip, prev) in [
                    (true, head_str, *pts.last().unwrap(), pts[pts.len() - 2]),
                    (false, tail_str, pts[0], pts[1]),
                ] {
                    if end_name == "zero-one" || end_name == "zero-many" {
                        let l = (tip.0 - prev.0).hypot(tip.1 - prev.1).max(1.0);
                        let d = (l * 0.5).min(20.0) + if end_name == "zero-one" { 14.0 } else { 10.0 };
                        let cx = tip.0 + (prev.0 - tip.0) / l * d;
                        let cy = tip.1 + (prev.1 - tip.1) / l * d;
                        scene.push(SceneEl {
                            id: format!("_zero{i}{}", if is_head { "head" } else { "tail" }),
                            el_type: "ellipse".into(),
                            x: cx - 6.0,
                            y: cy - 6.0,
                            width: 12.0,
                            height: 12.0,
                            stroke_color: stroke_color.clone(),
                            background_color: paper_hex.to_owned(),
                            fill_style: "solid".into(),
                            stroke_width,
                            stroke_style: "solid".into(),
                            roughness,
                            opacity,
                            roundness: None,
                            points: Vec::new(),
                            closed: false,
                            start_arrowhead: None,
                            end_arrowhead: None,
                            text: None,
                            font_size: 16.0,
                            mono: false,
                            text_align: "center".into(),
                            vertical_align: "middle".into(),
                            container_id: None,
                            seed: 1000 + scene.len() as u32 * 37,
                        });
                    }
                }
            }

            // Arrow label (bound or detached)
            if t == "arrow" {
                if let Some(txt) = str_at(e, "text").filter(|s| !s.is_empty()) {
                    let tu_do = e.get("labelAt").is_some()
                        || e.get("labelDx").is_some()
                        || e.get("labelDy").is_some();
                    let sz = f64_at(e, "size", 16.0);
                    let tw = rong_chu(txt, sz);
                    let th = cao_chu(txt, sz);

                    let segs: Vec<((f64, f64), (f64, f64), f64)> = pts
                        .windows(2)
                        .map(|w| (w[0], w[1], (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)))
                        .collect();
                    let tong: f64 = segs.iter().map(|s| s.2).sum::<f64>().max(1.0);
                    let at = f64_at(e, "labelAt", 0.5);
                    let mut con = if at <= 1.0 { at * tong } else { at.min(tong) };
                    let (mut lx, mut ly, mut ngang) = (pts[0].0, pts[0].1, true);
                    for &(p0, p1, l) in &segs {
                        if con <= l || (p0 == segs.last().unwrap().0 && p1 == segs.last().unwrap().1) {
                            let f = if l > 1e-6 { con / l } else { 0.5 };
                            lx = p0.0 + (p1.0 - p0.0) * f;
                            ly = p0.1 + (p1.1 - p0.1) * f;
                            ngang = (p1.0 - p0.0).abs() >= (p1.1 - p0.1).abs();
                            break;
                        }
                        con -= l;
                    }

                    if tu_do {
                        let dx = e.get("labelDx").and_then(Value::as_f64).unwrap_or(if ngang {
                            0.0
                        } else {
                            tw / 2.0 + 8.0
                        });
                        let dy = e.get("labelDy").and_then(Value::as_f64).unwrap_or(if ngang {
                            -(th / 2.0 + 3.0)
                        } else {
                            0.0
                        });
                        scene.push(SceneEl {
                            id: format!("_lab{i}"),
                            el_type: "text".into(),
                            x: lx + dx - tw / 2.0,
                            y: ly + dy - th / 2.0,
                            width: tw,
                            height: th,
                            stroke_color: safe_text_color(&c),
                            background_color: "transparent".into(),
                            fill_style: "solid".into(),
                            stroke_width: 1.0,
                            stroke_style: "solid".into(),
                            roughness: 0.0,
                            opacity,
                            roundness: None,
                            points: Vec::new(),
                            closed: false,
                            start_arrowhead: None,
                            end_arrowhead: None,
                            text: Some(txt.to_owned()),
                            font_size: sz,
                            mono: false,
                            text_align: "center".into(),
                            vertical_align: "middle".into(),
                            container_id: None,
                            seed: 1000 + scene.len() as u32 * 37,
                        });
                    } else {
                        // Bound arrow label centered at path midpoint
                        let (mx, my) = if smooth && pts.len() == 3 {
                            // Quadratic Bezier midpoint at t=0.5: 0.25*p0 + 0.5*p1 + 0.25*p2
                            (
                                0.25 * pts[0].0 + 0.5 * pts[1].0 + 0.25 * pts[2].0,
                                0.25 * pts[0].1 + 0.5 * pts[1].1 + 0.25 * pts[2].1,
                            )
                        } else {
                            (lx, ly)
                        };
                        scene.push(SceneEl {
                            id: format!("{arrow_id}~lab"),
                            el_type: "text".into(),
                            x: mx - tw / 2.0,
                            y: my - th / 2.0,
                            width: tw,
                            height: th,
                            stroke_color: safe_text_color(&c),
                            background_color: "transparent".into(),
                            fill_style: "solid".into(),
                            stroke_width: 1.0,
                            stroke_style: "solid".into(),
                            roughness: 0.0,
                            opacity,
                            roundness: None,
                            points: Vec::new(),
                            closed: false,
                            start_arrowhead: None,
                            end_arrowhead: None,
                            text: Some(txt.to_owned()),
                            font_size: sz,
                            mono: false,
                            text_align: "center".into(),
                            vertical_align: "middle".into(),
                            container_id: Some(arrow_id),
                            seed: 1000 + scene.len() as u32 * 37,
                        });
                    }
                }
            }
        } else if t == "text" {
            let x = req_f64(e, "x", "text")?;
            let y = req_f64(e, "y", "text")?;
            let txt = str_at(e, "text").unwrap_or("");
            let fs = f64_at(e, "size", 20.0);
            let mono = bool_at(e, "mono", false);
            let tw = rong_chu(txt, fs) * if mono { 1.15 } else { 1.0 };
            let th = cao_chu(txt, fs);
            let align = str_at(e, "align").unwrap_or("left").to_owned();
            let adj_x = match align.as_str() {
                "center" => x - tw / 2.0,
                "right" => x - tw,
                _ => x,
            };
            scene.push(SceneEl {
                id: str_at(e, "id")
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("_text{i}")),
                el_type: "text".into(),
                x: adj_x,
                y,
                width: tw,
                height: th,
                stroke_color: safe_text_color(&c),
                background_color: "transparent".into(),
                fill_style: "solid".into(),
                stroke_width: 1.0,
                stroke_style: "solid".into(),
                roughness: 0.0,
                opacity,
                roundness: None,
                points: Vec::new(),
                closed: false,
                start_arrowhead: None,
                end_arrowhead: None,
                text: Some(txt.to_owned()),
                font_size: fs,
                mono,
                text_align: align,
                vertical_align: "top".into(),
                container_id: None,
                seed: 1000 + scene.len() as u32 * 37,
            });
        }
    }

    scene.extend(raw_els);
    Ok((scene, hinh, lint_arrows, els))
}

// ---------------------------------------------------------------------------
// Geometric Linter (port of `soat()` in ve.py)
// ---------------------------------------------------------------------------

fn seg_intersects_rect_inset(p: (f64, f64), q: (f64, f64), x: f64, y: f64, w: f64, h: f64, co: f64) -> bool {
    let (x0, y0, x1, y1) = (x + co, y + co, x + w - co, y + h - co);
    if x1 <= x0 || y1 <= y0 {
        return false;
    }
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (d, a, b) in [(dx, x0 - p.0, x1 - p.0), (dy, y0 - p.1, y1 - p.1)] {
        if d.abs() < 1e-9 {
            if a > 0.0 || b < 0.0 {
                return false;
            }
        } else {
            let (mut ta, mut tb) = (a / d, b / d);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 {
                return false;
            }
        }
    }
    t1 - t0 > 0.02
}

fn overlap_area(ax: f64, ay: f64, aw: f64, ah: f64, bx: f64, by: f64, bw: f64, bh: f64) -> f64 {
    let w = (ax + aw).min(bx + bw) - ax.max(bx);
    let h = (ay + ah).min(by + bh) - ay.max(by);
    w.max(0.0) * h.max(0.0)
}

fn lint_scene(
    scene: &[SceneEl],
    hinh: &HashMap<String, AnchorInfo>,
    lint_arrows: &[LintArrow],
    goc: &[Value],
) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut reminders = Vec::new();
    let by: HashMap<&str, &SceneEl> = scene.iter().map(|e| (e.id.as_str(), e)).collect();

    // 1. Text overflows containing shape
    for e in scene {
        if e.el_type == "text" {
            if let Some(cid) = e.container_id.as_deref() {
                if let Some(c) = by.get(cid) {
                    if matches!(c.el_type.as_str(), "rectangle" | "ellipse" | "diamond")
                        && c.stroke_color != "transparent"
                    {
                        let he = match c.el_type.as_str() {
                            "ellipse" => 0.78,
                            "diamond" => 0.58,
                            _ => 1.02,
                        };
                        if e.width > c.width * he + 4.0 || e.height > c.height * he + 4.0 {
                            let preview: String = e.text.as_deref().unwrap_or("").chars().take(28).collect();
                            errors.push(format!(
                                "text \"{preview}\" overflows shape '{}': enlarge w/h",
                                c.id
                            ));
                        }
                    }
                }
            }
        }
    }

    // 2. Arrow crosses non-endpoint shape & short arrow check
    let than: Vec<&AnchorInfo> = hinh
        .values()
        .filter(|h| is_shape_t(&h.t) && !h.ghost && !h.under && !h.id.contains('~'))
        .collect();

    for ar in lint_arrows {
        let dai: f64 = ar
            .pts
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum();
        let min_len = if ar.head_count < 2 { 22.0 } else { 64.0 };
        if ar.from != ar.to && dai < min_len {
            reminders.push(format!(
                "arrow {:?} -> {:?} is only {dai:.0} long for {} head(s)",
                ar.from, ar.to, ar.head_count
            ));
        }
        let a_box = ar
            .from
            .as_deref()
            .and_then(|id| hinh.get(id))
            .map(|h| (h.x, h.y, h.w, h.h))
            .unwrap_or((ar.pts[0].0, ar.pts[0].1, 0.0, 0.0));
        let b_rect = ar
            .to
            .as_deref()
            .and_then(|id| hinh.get(id))
            .map(|h| (h.x, h.y, h.w, h.h))
            .unwrap_or_else(|| {
                let p = *ar.pts.last().unwrap();
                (p.0, p.1, 0.0, 0.0)
            });

        for h in &than {
            if ar.from.as_deref() == Some(h.id.as_str()) || ar.to.as_deref() == Some(h.id.as_str()) {
                continue;
            }
            let encloses = |o: (f64, f64, f64, f64)| {
                h.x - 4.0 <= o.0
                    && h.y - 4.0 <= o.1
                    && h.x + h.w + 4.0 >= o.0 + o.2
                    && h.y + h.h + 4.0 >= o.1 + o.3
            };
            if encloses(a_box) || encloses(b_rect) {
                continue;
            }
            if ar
                .pts
                .windows(2)
                .any(|w| seg_intersects_rect_inset(w[0], w[1], h.x, h.y, h.w, h.h, 6.0))
            {
                reminders.push(format!("arrow crosses shape '{}'", h.id));
                break;
            }
        }
    }

    // 3. Free text overlaps another text
    let free: Vec<&SceneEl> = scene
        .iter()
        .filter(|e| e.el_type == "text" && e.container_id.is_none())
        .collect();
    let all_text: Vec<&SceneEl> = scene.iter().filter(|e| e.el_type == "text").collect();
    let mut seen_pairs: HashSet<(&str, &str)> = HashSet::new();
    for f in &free {
        for o in &all_text {
            if f.id == o.id || seen_pairs.contains(&(o.id.as_str(), f.id.as_str())) {
                continue;
            }
            let area = overlap_area(f.x, f.y, f.width, f.height, o.x, o.y, o.width, o.height);
            let min_a = (f.width * f.height).min(o.width * o.height);
            if min_a > 0.0 && area > 0.18 * min_a {
                seen_pairs.insert((f.id.as_str(), o.id.as_str()));
                reminders.push(format!(
                    "text \"{}\" overlaps text \"{}\"",
                    f.text.as_deref().unwrap_or(""),
                    o.text.as_deref().unwrap_or("")
                ));
            }
        }
    }

    // 4. Too many accent colors
    let mut accents: HashSet<&str> = HashSet::new();
    for e in goc {
        if let Some(c) = str_at(e, "color") {
            if is_accent(c) {
                accents.insert(c);
            }
        }
        if let Some(f) = str_at(e, "fill") {
            if is_accent(f) {
                accents.insert(f);
            }
        }
    }
    if accents.len() > 3 {
        reminders.push(format!("{} accent colors used (recommend <= 3)", accents.len()));
    }

    (errors, reminders)
}

// ---------------------------------------------------------------------------
// Pure-Rust Rough.js Hand-Drawn SVG Renderer (solid, hachure, cross-hatch, curves, all arrowheads)
// ---------------------------------------------------------------------------

struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Self(if seed == 0 { 1337 } else { seed })
    }

    fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0
    }

    fn offset(&mut self, max: f64) -> f64 {
        (self.next_f64() * 2.0 - 1.0) * max
    }
}

fn fmt_n(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-6 {
        format!("{}", r.round() as i64)
    } else {
        format!("{r:.2}")
    }
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

fn push_rough_line(out: &mut String, x1: f64, y1: f64, x2: f64, y2: f64, roughness: f64, rng: &mut Rng) {
    let len = (x2 - x1).hypot(y2 - y1);
    if len < 0.4 {
        return;
    }
    if roughness <= 0.05 {
        let _ = write!(out, "M {} {} L {} {} ", fmt_n(x1), fmt_n(y1), fmt_n(x2), fmt_n(y2));
        return;
    }
    let max_off = (roughness * if len < 200.0 { 0.85 } else if len > 500.0 { 0.35 } else { 0.6 })
        .min(len / 14.0);
    let div = 0.2 + rng.next_f64() * 0.2;
    for pass in 0..2 {
        let mo = max_off * if pass == 0 { 1.0 } else { 0.6 };
        let dx_b = rng.offset((mo * 0.55).max(0.3));
        let dy_b = rng.offset((mo * 0.55).max(0.3));
        let _ = write!(
            out,
            "M {} {} C {} {}, {} {}, {} {} ",
            fmt_n(x1 + rng.offset(mo)),
            fmt_n(y1 + rng.offset(mo)),
            fmt_n(x1 + (x2 - x1) * div + dx_b + rng.offset(mo)),
            fmt_n(y1 + (y2 - y1) * div + dy_b + rng.offset(mo)),
            fmt_n(x1 + (x2 - x1) * (1.0 - div) + dx_b + rng.offset(mo)),
            fmt_n(y1 + (y2 - y1) * (1.0 - div) + dy_b + rng.offset(mo)),
            fmt_n(x2 + rng.offset(mo)),
            fmt_n(y2 + rng.offset(mo)),
        );
    }
}

fn rough_smooth_curve(pts: &[(f64, f64)], closed: bool, roughness: f64, rng: &mut Rng) -> String {
    if pts.len() < 2 {
        return String::new();
    }
    let mut out = String::new();
    let passes = if roughness <= 0.05 { 1 } else { 2 };
    for pass in 0..passes {
        let j = roughness * if pass == 0 { 0.7 } else { 0.45 };
        if pts.len() == 2 {
            push_rough_line(&mut out, pts[0].0, pts[0].1, pts[1].0, pts[1].1, roughness, rng);
            break;
        }
        if pts.len() == 3 && !closed {
            let _ = write!(
                out,
                "M {} {} Q {} {} {} {} ",
                fmt_n(pts[0].0 + rng.offset(j)),
                fmt_n(pts[0].1 + rng.offset(j)),
                fmt_n(pts[1].0 + rng.offset(j)),
                fmt_n(pts[1].1 + rng.offset(j)),
                fmt_n(pts[2].0 + rng.offset(j)),
                fmt_n(pts[2].1 + rng.offset(j)),
            );
            continue;
        }
        let n = pts.len();
        let _ = write!(
            out,
            "M {} {} ",
            fmt_n(pts[0].0 + rng.offset(j)),
            fmt_n(pts[0].1 + rng.offset(j))
        );
        for i in 0..n - 1 {
            let p0 = pts[if i == 0 { if closed { n - 2 } else { 0 } } else { i - 1 }];
            let p1 = pts[i];
            let p2 = pts[i + 1];
            let p3 = pts[if i + 2 < n { i + 2 } else if closed { 1 } else { n - 1 }];
            let cp1x = p1.0 + (p2.0 - p0.0) / 6.0 + rng.offset(j);
            let cp1y = p1.1 + (p2.1 - p0.1) / 6.0 + rng.offset(j);
            let cp2x = p2.0 - (p3.0 - p1.0) / 6.0 + rng.offset(j);
            let cp2y = p2.1 - (p3.1 - p1.1) / 6.0 + rng.offset(j);
            let _ = write!(
                out,
                "C {} {}, {} {}, {} {} ",
                fmt_n(cp1x),
                fmt_n(cp1y),
                fmt_n(cp2x),
                fmt_n(cp2y),
                fmt_n(p2.0 + rng.offset(j)),
                fmt_n(p2.1 + rng.offset(j)),
            );
        }
    }
    out
}

fn rounded_rect_path(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let rad = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if rad < 0.5 {
        return format!(
            "M {} {} L {} {} L {} {} L {} {} Z",
            fmt_n(x), fmt_n(y),
            fmt_n(x + w), fmt_n(y),
            fmt_n(x + w), fmt_n(y + h),
            fmt_n(x), fmt_n(y + h),
        );
    }
    format!(
        "M {} {} L {} {} Q {} {} {} {} L {} {} Q {} {} {} {} L {} {} Q {} {} {} {} L {} {} Q {} {} {} {} Z",
        fmt_n(x + rad), fmt_n(y),
        fmt_n(x + w - rad), fmt_n(y),
        fmt_n(x + w), fmt_n(y), fmt_n(x + w), fmt_n(y + rad),
        fmt_n(x + w), fmt_n(y + h - rad),
        fmt_n(x + w), fmt_n(y + h), fmt_n(x + w - rad), fmt_n(y + h),
        fmt_n(x + rad), fmt_n(y + h),
        fmt_n(x), fmt_n(y + h), fmt_n(x), fmt_n(y + h - rad),
        fmt_n(x), fmt_n(y + rad),
        fmt_n(x), fmt_n(y), fmt_n(x + rad), fmt_n(y),
    )
}

fn rough_rect_stroke(x: f64, y: f64, w: f64, h: f64, r: f64, roughness: f64, rng: &mut Rng) -> String {
    let rad = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut out = String::new();
    push_rough_line(&mut out, x + rad, y, x + w - rad, y, roughness, rng);
    push_rough_line(&mut out, x + w, y + rad, x + w, y + h - rad, roughness, rng);
    push_rough_line(&mut out, x + w - rad, y + h, x + rad, y + h, roughness, rng);
    push_rough_line(&mut out, x, y + h - rad, x, y + rad, roughness, rng);
    if rad >= 0.5 {
        let corners = [
            (x + w - rad, y, x + w, y, x + w, y + rad),
            (x + w, y + h - rad, x + w, y + h, x + w - rad, y + h),
            (x + rad, y + h, x, y + h, x, y + h - rad),
            (x, y + rad, x, y, x + rad, y),
        ];
        for (sx, sy, cx, cy, ex, ey) in corners {
            let passes = if roughness <= 0.05 { 1 } else { 2 };
            for pass in 0..passes {
                let j = roughness * if pass == 0 { 0.7 } else { 0.45 };
                let _ = write!(
                    out,
                    "M {} {} Q {} {} {} {} ",
                    fmt_n(sx + rng.offset(j)),
                    fmt_n(sy + rng.offset(j)),
                    fmt_n(cx + rng.offset(j)),
                    fmt_n(cy + rng.offset(j)),
                    fmt_n(ex + rng.offset(j)),
                    fmt_n(ey + rng.offset(j)),
                );
            }
        }
    }
    out
}

fn rough_ellipse_stroke(cx: f64, cy: f64, rx: f64, ry: f64, roughness: f64, rng: &mut Rng) -> String {
    let steps = 28;
    let passes = if roughness <= 0.05 { 1 } else { 2 };
    let mut out = String::new();
    for pass in 0..passes {
        let j = roughness * if pass == 0 { 0.8 } else { 0.5 };
        let pts: Vec<(f64, f64)> = (0..=steps)
            .map(|i| {
                let a = 2.0 * PI * i as f64 / steps as f64;
                (cx + rx * a.cos() + rng.offset(j), cy + ry * a.sin() + rng.offset(j))
            })
            .collect();
        out.push_str(&rough_smooth_curve(&pts, true, 0.0, rng));
    }
    out
}

/// Generates SVG `<g clip-path=...>` for Excalidraw-style hachure / cross-hatch fills.
fn push_hachure_fill(
    defs: &mut String,
    body: &mut String,
    clip_id: &str,
    clip_shape_svg: &str,
    bx: f64,
    by: f64,
    bw: f64,
    bh: f64,
    color: &str,
    stroke_w: f64,
    cross: bool,
    opacity_attr: &str,
    rng: &mut Rng,
) {
    let _ = write!(defs, "<clipPath id=\"{clip_id}\">{clip_shape_svg}</clipPath>");
    let gap = (stroke_w * 4.0).max(5.5);
    let line_w = (stroke_w * 0.55).max(0.8);
    let mut d = String::new();
    // 45-degree diagonal hatch
    let mut offset = -bh;
    while offset < bw + bh {
        let x1 = bx + offset;
        let y1 = by + bh;
        let x2 = bx + offset + bh;
        let y2 = by;
        push_rough_line(&mut d, x1, y1, x2, y2, 0.45, rng);
        offset += gap;
    }
    if cross {
        let mut offset = -bh;
        while offset < bw + bh {
            let x1 = bx + offset;
            let y1 = by;
            let x2 = bx + offset + bh;
            let y2 = by + bh;
            push_rough_line(&mut d, x1, y1, x2, y2, 0.45, rng);
            offset += gap;
        }
    }
    let _ = write!(
        body,
        "<g clip-path=\"url(#{clip_id})\"{opacity_attr}><path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linecap=\"round\"/></g>",
        fmt_n(line_w)
    );
}

fn push_arrowhead(
    out: &mut String,
    kind: &str,
    from: (f64, f64),
    to: (f64, f64),
    color: &str,
    stroke_w: f64,
    paper_color: &str,
    rng: &mut Rng,
) {
    let ang = (to.1 - from.1).atan2(to.0 - from.0);
    let ux = ang.cos();
    let uy = ang.sin();
    let px = -uy;
    let py = ux;

    match kind {
        "arrow" => {
            let len = 13.0;
            let spread = PI / 6.0;
            let w1 = (to.0 - len * (ang - spread).cos(), to.1 - len * (ang - spread).sin());
            let w2 = (to.0 - len * (ang + spread).cos(), to.1 - len * (ang + spread).sin());
            let mut d = String::new();
            push_rough_line(&mut d, w1.0, w1.1, to.0, to.1, 0.6, rng);
            push_rough_line(&mut d, w2.0, w2.1, to.0, to.1, 0.6, rng);
            let _ = write!(
                out,
                "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linecap=\"round\"/>",
                fmt_n(stroke_w)
            );
        }
        "triangle" | "triangle_outline" => {
            let len = 14.0;
            let half = 6.5;
            let b1 = (to.0 - ux * len + px * half, to.1 - uy * len + py * half);
            let b2 = (to.0 - ux * len - px * half, to.1 - uy * len - py * half);
            let fill = if kind == "triangle" { color } else { paper_color };
            let _ = write!(
                out,
                "<polygon points=\"{},{} {},{} {},{}\" fill=\"{fill}\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>",
                fmt_n(to.0), fmt_n(to.1), fmt_n(b1.0), fmt_n(b1.1), fmt_n(b2.0), fmt_n(b2.1), fmt_n(stroke_w)
            );
        }
        "diamond" | "diamond_outline" => {
            let len = 16.0;
            let half = 6.5;
            let mid = (to.0 - ux * len * 0.5, to.1 - uy * len * 0.5);
            let back = (to.0 - ux * len, to.1 - uy * len);
            let s1 = (mid.0 + px * half, mid.1 + py * half);
            let s2 = (mid.0 - px * half, mid.1 - py * half);
            let fill = if kind == "diamond" { color } else { paper_color };
            let _ = write!(
                out,
                "<polygon points=\"{},{} {},{} {},{} {},{}\" fill=\"{fill}\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>",
                fmt_n(to.0), fmt_n(to.1), fmt_n(s1.0), fmt_n(s1.1),
                fmt_n(back.0), fmt_n(back.1), fmt_n(s2.0), fmt_n(s2.1), fmt_n(stroke_w)
            );
        }
        "circle" | "circle_outline" => {
            let r = 5.0;
            let cx = to.0 - ux * r;
            let cy = to.1 - uy * r;
            let fill = if kind == "circle" { color } else { paper_color };
            let _ = write!(
                out,
                "<circle cx=\"{}\" cy=\"{}\" r=\"{r}\" fill=\"{fill}\" stroke=\"{color}\" stroke-width=\"{}\"/>",
                fmt_n(cx), fmt_n(cy), fmt_n(stroke_w)
            );
        }
        "bar" => {
            let half = 7.5;
            let b1 = (to.0 + px * half, to.1 + py * half);
            let b2 = (to.0 - px * half, to.1 - py * half);
            let mut d = String::new();
            push_rough_line(&mut d, b1.0, b1.1, b2.0, b2.1, 0.5, rng);
            let _ = write!(
                out,
                "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linecap=\"round\"/>",
                fmt_n(stroke_w)
            );
        }
        "crowfoot_one" | "crowfoot_many" | "crowfoot_one_or_many" => {
            let mut d = String::new();
            if kind == "crowfoot_one" || kind == "crowfoot_one_or_many" {
                let dist = if kind == "crowfoot_one" { 10.0 } else { 15.0 };
                let c = (to.0 - ux * dist, to.1 - uy * dist);
                let half = 7.5;
                push_rough_line(
                    &mut d,
                    c.0 + px * half,
                    c.1 + py * half,
                    c.0 - px * half,
                    c.1 - py * half,
                    0.45,
                    rng,
                );
            }
            if kind == "crowfoot_many" || kind == "crowfoot_one_or_many" {
                let base = (to.0 - ux * 13.0, to.1 - uy * 13.0);
                let spread = 8.0;
                let t1 = (to.0 + px * spread, to.1 + py * spread);
                let t2 = (to.0 - px * spread, to.1 - py * spread);
                push_rough_line(&mut d, base.0, base.1, t1.0, t1.1, 0.45, rng);
                push_rough_line(&mut d, base.0, base.1, t2.0, t2.1, 0.45, rng);
            }
            let _ = write!(
                out,
                "<path d=\"{d}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" stroke-linecap=\"round\"/>",
                fmt_n(stroke_w)
            );
        }
        _ => {}
    }
}

fn scene_bounds(scene: &[SceneEl]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for e in scene {
        if !e.points.is_empty() {
            for &(dx, dy) in &e.points {
                min_x = min_x.min(e.x + dx);
                min_y = min_y.min(e.y + dy);
                max_x = max_x.max(e.x + dx);
                max_y = max_y.max(e.y + dy);
            }
        } else {
            min_x = min_x.min(e.x);
            min_y = min_y.min(e.y);
            max_x = max_x.max(e.x + e.width);
            max_y = max_y.max(e.y + e.height);
        }
    }
    if !min_x.is_finite() {
        return (0.0, 0.0, 400.0, 200.0);
    }
    (min_x, min_y, max_x, max_y)
}

fn render_scene_svg(scene: &[SceneEl]) -> (f64, f64, String) {
    let (min_x, min_y, max_x, max_y) = scene_bounds(scene);
    let pad = 16.0;
    let ox = pad - min_x;
    let oy = pad - min_y;
    let width = ((max_x - min_x) + pad * 2.0).max(60.0).ceil();
    let height = ((max_y - min_y) + pad * 2.0).max(40.0).ceil();

    // Map arrow_id -> bound label rect for SVG mask gap cutting
    let mut arrow_label_rect: HashMap<&str, (f64, f64, f64, f64)> = HashMap::new();
    for e in scene {
        if e.el_type == "text" {
            if let Some(cid) = e.container_id.as_deref() {
                arrow_label_rect.insert(cid, (e.x + ox - 5.0, e.y + oy - 2.0, e.width + 10.0, e.height + 4.0));
            }
        }
    }

    let mut defs = String::new();
    let mut body = String::with_capacity(16 * 1024);

    let resolve_render_color = |hex: &str| -> String {
        if hex.eq_ignore_ascii_case(CANH_NEN) {
            PRE_BG_LIGHT.to_owned()
        } else if hex.eq_ignore_ascii_case(CANH_CHU) {
            PRE_FG_LIGHT.to_owned()
        } else {
            hex.to_owned()
        }
    };

    for (idx, e) in scene.iter().enumerate() {
        let mut rng = Rng::new(e.seed);
        let stroke = resolve_render_color(&e.stroke_color);
        let bg = resolve_render_color(&e.background_color);
        let op_attr = if e.opacity < 99.5 {
            format!(" opacity=\"{}\"", fmt_n(e.opacity / 100.0))
        } else {
            String::new()
        };
        let dash_attr = match e.stroke_style.as_str() {
            "dashed" => " stroke-dasharray=\"8 6\"",
            "dotted" => " stroke-dasharray=\"2 5\"",
            _ => "",
        };

        let x = e.x + ox;
        let y = e.y + oy;

        match e.el_type.as_str() {
            "rectangle" => {
                let rad = if e.roundness.is_some() {
                    (e.width.min(e.height) * 0.25).min(12.0)
                } else {
                    0.0
                };
                let fill_path = rounded_rect_path(x, y, e.width, e.height, rad);
                if bg != "transparent" {
                    if e.fill_style == "solid" {
                        let _ = write!(body, "<path d=\"{fill_path}\" fill=\"{bg}\" stroke=\"none\"{op_attr}/>");
                    } else {
                        let clip_id = format!("hc{idx}");
                        let clip_svg = format!("<path d=\"{fill_path}\"/>");
                        push_hachure_fill(
                            &mut defs,
                            &mut body,
                            &clip_id,
                            &clip_svg,
                            x,
                            y,
                            e.width,
                            e.height,
                            &bg,
                            e.stroke_width,
                            e.fill_style == "cross-hatch",
                            &op_attr,
                            &mut rng,
                        );
                    }
                }
                if stroke != "transparent" {
                    let d = rough_rect_stroke(x, y, e.width, e.height, rad, e.roughness, &mut rng);
                    let _ = write!(
                        body,
                        "<path d=\"{d}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash_attr}{op_attr}/>",
                        fmt_n(e.stroke_width)
                    );
                }
            }
            "ellipse" => {
                let cx = x + e.width / 2.0;
                let cy = y + e.height / 2.0;
                let rx = (e.width / 2.0).max(0.5);
                let ry = (e.height / 2.0).max(0.5);
                if bg != "transparent" {
                    if e.fill_style == "solid" {
                        let _ = write!(
                            body,
                            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{bg}\" stroke=\"none\"{op_attr}/>",
                            fmt_n(cx), fmt_n(cy), fmt_n(rx), fmt_n(ry)
                        );
                    } else {
                        let clip_id = format!("hc{idx}");
                        let clip_svg = format!(
                            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"/>",
                            fmt_n(cx), fmt_n(cy), fmt_n(rx), fmt_n(ry)
                        );
                        push_hachure_fill(
                            &mut defs,
                            &mut body,
                            &clip_id,
                            &clip_svg,
                            x,
                            y,
                            e.width,
                            e.height,
                            &bg,
                            e.stroke_width,
                            e.fill_style == "cross-hatch",
                            &op_attr,
                            &mut rng,
                        );
                    }
                }
                if stroke != "transparent" {
                    let d = rough_ellipse_stroke(cx, cy, rx, ry, e.roughness, &mut rng);
                    let _ = write!(
                        body,
                        "<path d=\"{d}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash_attr}{op_attr}/>",
                        fmt_n(e.stroke_width)
                    );
                }
            }
            "diamond" => {
                let cx = x + e.width / 2.0;
                let cy = y + e.height / 2.0;
                let pts = [(cx, y), (x + e.width, cy), (cx, y + e.height), (x, cy)];
                let poly = format!(
                    "{},{} {},{} {},{} {},{}",
                    fmt_n(pts[0].0), fmt_n(pts[0].1),
                    fmt_n(pts[1].0), fmt_n(pts[1].1),
                    fmt_n(pts[2].0), fmt_n(pts[2].1),
                    fmt_n(pts[3].0), fmt_n(pts[3].1),
                );
                if bg != "transparent" {
                    if e.fill_style == "solid" {
                        let _ = write!(body, "<polygon points=\"{poly}\" fill=\"{bg}\" stroke=\"none\"{op_attr}/>");
                    } else {
                        let clip_id = format!("hc{idx}");
                        let clip_svg = format!("<polygon points=\"{poly}\"/>");
                        push_hachure_fill(
                            &mut defs,
                            &mut body,
                            &clip_id,
                            &clip_svg,
                            x,
                            y,
                            e.width,
                            e.height,
                            &bg,
                            e.stroke_width,
                            e.fill_style == "cross-hatch",
                            &op_attr,
                            &mut rng,
                        );
                    }
                }
                if stroke != "transparent" {
                    let mut d = String::new();
                    for k in 0..4 {
                        let a = pts[k];
                        let b = pts[(k + 1) % 4];
                        push_rough_line(&mut d, a.0, a.1, b.0, b.1, e.roughness, &mut rng);
                    }
                    let _ = write!(
                        body,
                        "<path d=\"{d}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash_attr}{op_attr}/>",
                        fmt_n(e.stroke_width)
                    );
                }
            }
            "line" | "arrow" => {
                let abs_pts: Vec<(f64, f64)> = e.points.iter().map(|&(dx, dy)| (x + dx, y + dy)).collect();
                if abs_pts.len() < 2 {
                    continue;
                }
                if e.closed && bg != "transparent" {
                    let fill_d = if e.roundness.is_some() {
                        format!("{} Z", rough_smooth_curve(&abs_pts, true, 0.0, &mut rng))
                    } else {
                        let mut p = format!("M {} {} ", fmt_n(abs_pts[0].0), fmt_n(abs_pts[0].1));
                        for pt in &abs_pts[1..] {
                            let _ = write!(p, "L {} {} ", fmt_n(pt.0), fmt_n(pt.1));
                        }
                        p.push('Z');
                        p
                    };
                    if e.fill_style == "solid" {
                        let _ = write!(body, "<path d=\"{fill_d}\" fill=\"{bg}\" stroke=\"none\"{op_attr}/>");
                    } else {
                        let (mut bx0, mut by0, mut bx1, mut by1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
                        for &(px, py) in &abs_pts {
                            bx0 = bx0.min(px);
                            by0 = by0.min(py);
                            bx1 = bx1.max(px);
                            by1 = by1.max(py);
                        }
                        let clip_id = format!("hc{idx}");
                        let clip_svg = format!("<path d=\"{fill_d}\"/>");
                        push_hachure_fill(
                            &mut defs,
                            &mut body,
                            &clip_id,
                            &clip_svg,
                            bx0,
                            by0,
                            bx1 - bx0,
                            by1 - by0,
                            &bg,
                            e.stroke_width,
                            e.fill_style == "cross-hatch",
                            &op_attr,
                            &mut rng,
                        );
                    }
                }

                let mask_attr = if let Some(&(lx, ly, lw, lh)) = arrow_label_rect.get(e.id.as_str()) {
                    let mid = format!("mk{idx}");
                    let _ = write!(
                        defs,
                        "<mask id=\"{mid}\"><path d=\"M0 0h{}v{}h-{}Z\" fill=\"#ffffff\"/><path d=\"M{} {}h{}v{}h-{}Z\" fill=\"#000000\"/></mask>",
                        fmt_n(width), fmt_n(height), fmt_n(width),
                        fmt_n(lx), fmt_n(ly), fmt_n(lw), fmt_n(lh), fmt_n(lw),
                    );
                    format!(" mask=\"url(#{mid})\"")
                } else {
                    String::new()
                };

                let stroke_d = if e.roundness.is_some() {
                    rough_smooth_curve(&abs_pts, e.closed, e.roughness, &mut rng)
                } else {
                    let mut d = String::new();
                    for w in abs_pts.windows(2) {
                        push_rough_line(&mut d, w[0].0, w[0].1, w[1].0, w[1].1, e.roughness, &mut rng);
                    }
                    d
                };
                if stroke != "transparent" {
                    let _ = write!(
                        body,
                        "<path d=\"{stroke_d}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash_attr}{op_attr}{mask_attr}/>",
                        fmt_n(e.stroke_width)
                    );
                }
                if e.el_type == "arrow" {
                    let paper = token_light("paper").unwrap();
                    if let Some(ref end_h) = e.end_arrowhead {
                        let n = abs_pts.len();
                        push_arrowhead(&mut body, end_h, abs_pts[n - 2], abs_pts[n - 1], &stroke, e.stroke_width, paper, &mut rng);
                    }
                    if let Some(ref start_h) = e.start_arrowhead {
                        push_arrowhead(&mut body, start_h, abs_pts[1], abs_pts[0], &stroke, e.stroke_width, paper, &mut rng);
                    }
                }
            }
            "text" => {
                let txt = e.text.as_deref().unwrap_or("");
                if txt.is_empty() {
                    continue;
                }
                let font_family = if e.mono {
                    "'IBM Plex Mono', monospace"
                } else {
                    "'Patrick Hand', 'Be Vietnam Pro', cursive, sans-serif"
                };
                let anchor = match e.text_align.as_str() {
                    "center" => "middle",
                    "right" => "end",
                    _ => "start",
                };
                let tx = match e.text_align.as_str() {
                    "center" => x + e.width / 2.0,
                    "right" => x + e.width,
                    _ => x,
                };
                let lh = e.font_size * 1.25;
                for (li, line) in txt.split('\n').enumerate() {
                    let ty = y + e.font_size * 0.92 + li as f64 * lh;
                    let _ = write!(
                        body,
                        "<text x=\"{}\" y=\"{}\" text-anchor=\"{anchor}\" fill=\"{stroke}\" font-family=\"{font_family}\" font-size=\"{}\"{op_attr}>{}</text>",
                        fmt_n(tx),
                        fmt_n(ty),
                        fmt_n(e.font_size),
                        escape_xml(line),
                    );
                }
            }
            _ => {}
        }
    }

    let mut svg = String::with_capacity(defs.len() + body.len() + 256);
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" class=\"dg\">",
        fmt_n(width), fmt_n(height), fmt_n(width), fmt_n(height),
    );
    if !defs.is_empty() {
        let _ = write!(svg, "<defs>{defs}</defs>");
    }
    svg.push_str(&body);
    svg.push_str("</svg>");
    (width, height, svg)
}

// ---------------------------------------------------------------------------
// Excalidraw v2 JSON export
// ---------------------------------------------------------------------------

fn scene_to_excalidraw_json(scene: &[SceneEl]) -> String {
    let elements: Vec<Value> = scene
        .iter()
        .map(|e| {
            let mut obj = Map::new();
            obj.insert("id".into(), json!(e.id));
            obj.insert("type".into(), json!(e.el_type));
            obj.insert("x".into(), json!(e.x));
            obj.insert("y".into(), json!(e.y));
            obj.insert("width".into(), json!(e.width));
            obj.insert("height".into(), json!(e.height));
            obj.insert("angle".into(), json!(0));
            obj.insert("strokeColor".into(), json!(e.stroke_color.replace(CANH_CHU, PRE_FG_LIGHT)));
            obj.insert("backgroundColor".into(), json!(e.background_color.replace(CANH_NEN, PRE_BG_LIGHT)));
            obj.insert("fillStyle".into(), json!(e.fill_style));
            obj.insert("strokeWidth".into(), json!(e.stroke_width));
            obj.insert("strokeStyle".into(), json!(e.stroke_style));
            obj.insert("roughness".into(), json!(e.roughness));
            obj.insert("opacity".into(), json!(e.opacity));
            obj.insert("groupIds".into(), json!([]));
            obj.insert("frameId".into(), Value::Null);
            obj.insert("roundness".into(), match e.roundness {
                Some(rt) => json!({ "type": rt }),
                None => Value::Null,
            });
            obj.insert("seed".into(), json!(e.seed));
            obj.insert("version".into(), json!(1));
            obj.insert("versionNonce".into(), json!(1));
            obj.insert("isDeleted".into(), json!(false));
            obj.insert("boundElements".into(), Value::Null);
            obj.insert("updated".into(), json!(1));
            obj.insert("link".into(), Value::Null);
            obj.insert("locked".into(), json!(false));

            if e.el_type == "line" || e.el_type == "arrow" {
                let pts: Vec<[f64; 2]> = e.points.iter().map(|&(a, b)| [a, b]).collect();
                obj.insert("points".into(), json!(pts));
                obj.insert("lastCommittedPoint".into(), Value::Null);
                obj.insert("startArrowhead".into(), json!(e.start_arrowhead));
                obj.insert("endArrowhead".into(), json!(e.end_arrowhead));
            }
            if e.el_type == "text" {
                let txt = e.text.as_deref().unwrap_or("");
                obj.insert("text".into(), json!(txt));
                obj.insert("originalText".into(), json!(txt));
                obj.insert("fontSize".into(), json!(e.font_size));
                obj.insert("fontFamily".into(), json!(if e.mono { 3 } else { 5 }));
                obj.insert("textAlign".into(), json!(e.text_align));
                obj.insert("verticalAlign".into(), json!(e.vertical_align));
                obj.insert("containerId".into(), json!(e.container_id));
                obj.insert("lineHeight".into(), json!(1.25));
            }
            Value::Object(obj)
        })
        .collect();

    serde_json::to_string_pretty(&json!({
        "type": "excalidraw",
        "version": 2,
        "source": "asr-studio-ve-rust",
        "elements": elements,
        "appState": {
            "viewBackgroundColor": "#ffffff",
            "gridSize": null
        },
        "files": {}
    }))
    .unwrap_or_else(|_| "{}".into())
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

pub fn load_icons_json(path: &Path) -> Result<HashMap<String, Value>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read icons {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("invalid icons JSON: {e}"))
}

pub fn render_spec_elements(
    spec_els: &[Value],
    icons: &HashMap<String, Value>,
) -> Result<VeOutput, String> {
    let (scene, hinh, lint_arrows, goc) = compile_scene(spec_els, icons)?;
    let (errors, reminders) = lint_scene(&scene, &hinh, &lint_arrows, &goc);
    let (width, height, svg) = render_scene_svg(&scene);
    let excalidraw_json = scene_to_excalidraw_json(&scene);
    Ok(VeOutput {
        width,
        height,
        element_count: scene.len(),
        svg,
        excalidraw_json,
        errors,
        reminders,
    })
}

pub fn render_spec_json(spec_json: &str, icons: &HashMap<String, Value>) -> Result<VeOutput, String> {
    let spec: Value = serde_json::from_str(spec_json).map_err(|e| format!("invalid spec JSON: {e}"))?;
    let els = spec
        .get("elements")
        .and_then(Value::as_array)
        .ok_or("spec missing 'elements' array")?;
    render_spec_elements(els, icons)
}

pub fn render_spec_file(path: &Path, icons: &HashMap<String, Value>) -> Result<VeOutput, String> {
    let els = load_spec_file(path)?;
    render_spec_elements(&els, icons)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_30_official_examples_when_skill_repo_present() {
        let skill_dir = Path::new("/Users/kietta/.claude/skills/drawing-excalidraw-figures");
        if !skill_dir.exists() {
            return;
        }
        let icons = load_icons_json(&skill_dir.join("assets/icons.json")).unwrap();
        let mut count = 0usize;
        let mut entries: Vec<_> = std::fs::read_dir(skill_dir.join("examples"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
            .collect();
        entries.sort();
        for path in entries {
            let out = render_spec_file(&path, &icons)
                .unwrap_or_else(|e| panic!("failed to render {}: {e}", path.display()));
            assert!(out.svg.starts_with("<svg "), "{}", path.display());
            assert!(out.svg.ends_with("</svg>"), "{}", path.display());
            assert!(out.width > 50.0 && out.height > 40.0, "{}", path.display());
            let parsed: Value = serde_json::from_str(&out.excalidraw_json).unwrap();
            assert_eq!(parsed["type"], "excalidraw");
            count += 1;
        }
        assert_eq!(count, 30);
    }
}
