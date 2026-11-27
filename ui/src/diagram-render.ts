//! Self-contained process flowchart layout engine and hand-drawn Excalidraw-style renderers
//! (Rough.js + Patrick Hand SVG preview, 2x HiDPI PNG via Rough.js Canvas2D, and editable `.excalidraw` v2 JSON).

import rough from "roughjs";

export type NodeKind = "start" | "step" | "decision" | "end";

export type DiagramNode = {
  id: string;
  kind: NodeKind;
  label: string;
  role: string;
};

export type DiagramEdge = {
  from: string;
  to: string;
  label: string;
};

export type DiagramUncertainty = {
  item: string;
  reason: string;
};

export type ProcessDiagram = {
  title: string;
  summary: string;
  nodes: DiagramNode[];
  edges: DiagramEdge[];
  uncertainties: DiagramUncertainty[];
  provider?: string;
  model: string;
};

export type Point = { x: number; y: number };

export type LayoutNode = DiagramNode & {
  x: number;
  y: number;
  width: number;
  height: number;
  layer: number;
  labelLines: string[];
  roleLines: string[];
};

export type LayoutEdge = DiagramEdge & {
  index: number;
  isBackEdge: boolean;
  points: Point[];
  labelLines: string[];
  labelBox: { x: number; y: number; width: number; height: number } | null;
};

export type LayoutDiagram = {
  diagram: ProcessDiagram;
  titleLines: string[];
  width: number;
  height: number;
  nodes: LayoutNode[];
  edges: LayoutEdge[];
};

const FONT_FAMILY_UI = '"Be Vietnam Pro", -apple-system, BlinkMacSystemFont, sans-serif';
const FONT_FAMILY_HAND = '"Patrick Hand", "Be Vietnam Pro", cursive, sans-serif';
const TITLE_FONT_SIZE = 20;
const TITLE_LINE_HEIGHT = 28;
const LABEL_FONT_SIZE = 14;
const LABEL_LINE_HEIGHT = 20;
const ROLE_FONT_SIZE = 11.5;
const ROLE_LINE_HEIGHT = 16;
const EDGE_FONT_SIZE = 12;
const EDGE_LINE_HEIGHT = 17;

/** Palette aligned with `drawing-excalidraw-figures` (VÁ 6): solid pastel fills + high-contrast `#111A2B` ink. */
const SKETCH_COLORS = {
  bg: "#ffffff",
  ink: "#111A2B",
  accent: "#1E5AE8",
  startFill: "#DBEAFE",
  stepFill: "#F2F6FB",
  decisionFill: "#FEF3C7",
  labelFill: "#FFFFFF",
  labelBorder: "#5C6B82",
};

function nodeFillColor(kind: NodeKind): string {
  if (kind === "start" || kind === "end") return SKETCH_COLORS.startFill;
  if (kind === "decision") return SKETCH_COLORS.decisionFill;
  return SKETCH_COLORS.stepFill;
}

function deterministicSeed(key: string): number {
  let h = 2166136261;
  for (let i = 0; i < key.length; i++) {
    h ^= key.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % 2_000_000_000 || 1337;
}

function roundedRectSvgPath(x: number, y: number, w: number, h: number, r: number): string {
  const rad = Math.min(r, w / 2, h / 2);
  return [
    `M ${x + rad} ${y}`,
    `L ${x + w - rad} ${y}`,
    `Q ${x + w} ${y} ${x + w} ${y + rad}`,
    `L ${x + w} ${y + h - rad}`,
    `Q ${x + w} ${y + h} ${x + w - rad} ${y + h}`,
    `L ${x + rad} ${y + h}`,
    `Q ${x} ${y + h} ${x} ${y + h - rad}`,
    `L ${x} ${y + rad}`,
    `Q ${x} ${y} ${x + rad} ${y}`,
    "Z",
  ].join(" ");
}

function arrowWingPoints(from: Point, to: Point): [Point, Point] {
  const angle = Math.atan2(to.y - from.y, to.x - from.x);
  const len = 12;
  const spread = Math.PI / 6.5;
  return [
    { x: to.x - len * Math.cos(angle - spread), y: to.y - len * Math.sin(angle - spread) },
    { x: to.x - len * Math.cos(angle + spread), y: to.y - len * Math.sin(angle + spread) },
  ];
}

const PAD_X = 48;
const PAD_TOP = 36;
const PAD_BOTTOM = 44;
const LAYER_GAP_Y = 96;
const NODE_GAP_X = 56;

let measureCtx: CanvasRenderingContext2D | null = null;
function getMeasureContext(): CanvasRenderingContext2D | null {
  if (measureCtx) return measureCtx;
  if (typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  measureCtx = canvas.getContext("2d");
  return measureCtx;
}

function measureTextWidth(text: string, fontSize: number, weight = 500): number {
  const ctx = getMeasureContext();
  if (!ctx) return text.length * fontSize * 0.58;
  ctx.font = `${weight} ${fontSize}px ${FONT_FAMILY_UI}`;
  return ctx.measureText(text).width;
}

function wrapText(text: string, maxWidth: number, fontSize: number, weight = 500): string[] {
  const cleaned = text.trim().replace(/\s+/g, " ");
  if (!cleaned) return [];
  const words = cleaned.split(" ");
  const lines: string[] = [];
  let current = "";

  const pushLongWord = (word: string) => {
    let chunk = "";
    for (const ch of word) {
      const next = chunk + ch;
      if (chunk && measureTextWidth(next, fontSize, weight) > maxWidth) {
        lines.push(chunk);
        chunk = ch;
      } else {
        chunk = next;
      }
    }
    if (chunk) current = chunk;
  };

  for (const word of words) {
    const candidate = current ? `${current} ${word}` : word;
    if (measureTextWidth(candidate, fontSize, weight) <= maxWidth) {
      current = candidate;
    } else if (!current) {
      pushLongWord(word);
    } else {
      lines.push(current);
      current = "";
      if (measureTextWidth(word, fontSize, weight) <= maxWidth) {
        current = word;
      } else {
        pushLongWord(word);
      }
    }
  }
  if (current) lines.push(current);
  return lines;
}

function maxLineWidth(lines: string[], fontSize: number, weight = 500): number {
  let best = 0;
  for (const line of lines) {
    best = Math.max(best, measureTextWidth(line, fontSize, weight));
  }
  return best;
}

function measureNode(node: DiagramNode): {
  width: number;
  height: number;
  labelLines: string[];
  roleLines: string[];
} {
  const maxLabelW = node.kind === "decision" ? 190 : 230;
  const labelLines = wrapText(node.label, maxLabelW, LABEL_FONT_SIZE, 600);
  const roleLines = node.role.trim()
    ? wrapText(node.role, maxLabelW, ROLE_FONT_SIZE, 500)
    : [];

  const textW = Math.max(
    maxLineWidth(labelLines, LABEL_FONT_SIZE, 600),
    maxLineWidth(roleLines, ROLE_FONT_SIZE, 500),
  );
  const labelH = Math.max(1, labelLines.length) * LABEL_LINE_HEIGHT;
  const roleH = roleLines.length > 0 ? roleLines.length * ROLE_LINE_HEIGHT + 6 : 0;
  const totalTextH = labelH + roleH;

  if (node.kind === "decision") {
    // Inscribed rectangle of a diamond is half its width/height; size generously to prevent clipping.
    const width = Math.ceil(Math.max(216, textW * 1.68 + 52));
    const height = Math.ceil(Math.max(114, totalTextH * 1.78 + 44));
    return { width, height, labelLines, roleLines };
  }
  if (node.kind === "start" || node.kind === "end") {
    const width = Math.ceil(Math.max(180, textW + 52));
    const height = Math.ceil(Math.max(58, totalTextH + 28));
    return { width, height, labelLines, roleLines };
  }
  const width = Math.ceil(Math.max(204, textW + 44));
  const height = Math.ceil(Math.max(64, totalTextH + 28));
  return { width, height, labelLines, roleLines };
}

/** Validates diagram structure and computes deterministic flowchart coordinates. */
export function layoutDiagram(diagram: ProcessDiagram): LayoutDiagram {
  if (!diagram || !Array.isArray(diagram.nodes) || diagram.nodes.length < 2) {
    throw new Error("Sơ đồ cần ít nhất 2 bước để dựng hình.");
  }
  if (!Array.isArray(diagram.edges) || diagram.edges.length === 0) {
    throw new Error("Sơ đồ chưa có đường nối giữa các bước.");
  }

  const nodeById = new Map<string, DiagramNode>();
  for (const node of diagram.nodes) {
    if (!node.id || !node.label?.trim()) {
      throw new Error("Dữ liệu bước trong sơ đồ bị thiếu mã hoặc nhãn.");
    }
    if (nodeById.has(node.id)) {
      throw new Error(`Trùng mã bước "${node.id}" trong sơ đồ.`);
    }
    nodeById.set(node.id, node);
  }

  const outgoing = new Map<string, string[]>();
  const inDegree = new Map<string, number>();
  for (const node of diagram.nodes) {
    outgoing.set(node.id, []);
    inDegree.set(node.id, 0);
  }
  for (const edge of diagram.edges) {
    if (!nodeById.has(edge.from) || !nodeById.has(edge.to)) {
      throw new Error(`Đường nối trỏ đến bước không tồn tại (${edge.from} -> ${edge.to}).`);
    }
    outgoing.get(edge.from)!.push(edge.to);
    inDegree.set(edge.to, (inDegree.get(edge.to) ?? 0) + 1);
  }

  // Classify cycle back-edges via DFS starting from start/root nodes.
  const roots = diagram.nodes
    .filter((n) => n.kind === "start" || (inDegree.get(n.id) ?? 0) === 0)
    .map((n) => n.id);
  if (roots.length === 0) roots.push(diagram.nodes[0].id);

  const visited = new Set<string>();
  const onStack = new Set<string>();
  const backEdgePairs = new Set<string>();
  const order: string[] = [];

  const dfs = (id: string) => {
    visited.add(id);
    onStack.add(id);
    order.push(id);
    for (const next of outgoing.get(id) ?? []) {
      if (onStack.has(next)) {
        backEdgePairs.add(`${id}=>${next}`);
      } else if (!visited.has(next)) {
        dfs(next);
      }
    }
    onStack.delete(id);
  };

  for (const r of roots) {
    if (!visited.has(r)) dfs(r);
  }
  for (const n of diagram.nodes) {
    if (!visited.has(n.id)) dfs(n.id);
  }

  // Compute topological longest-path layers on the DAG (ignoring back-edges).
  const layerOf = new Map<string, number>();
  for (const id of order) layerOf.set(id, 0);

  for (let pass = 0; pass < diagram.nodes.length; pass++) {
    let changed = false;
    for (const edge of diagram.edges) {
      if (backEdgePairs.has(`${edge.from}=>${edge.to}`) || edge.from === edge.to) continue;
      const candidate = (layerOf.get(edge.from) ?? 0) + 1;
      if (candidate > (layerOf.get(edge.to) ?? 0)) {
        layerOf.set(edge.to, candidate);
        changed = true;
      }
    }
    if (!changed) break;
  }

  // Group nodes by layer, preserving input order within each layer.
  const layers = new Map<number, DiagramNode[]>();
  let maxLayer = 0;
  for (const node of diagram.nodes) {
    const l = layerOf.get(node.id) ?? 0;
    maxLayer = Math.max(maxLayer, l);
    const bucket = layers.get(l) ?? [];
    bucket.push(node);
    layers.set(l, bucket);
  }

  const measured = new Map<string, ReturnType<typeof measureNode>>();
  for (const node of diagram.nodes) {
    measured.set(node.id, measureNode(node));
  }

  // Compute row widths and heights to center every layer cleanly.
  const rowWidth = new Map<number, number>();
  const rowHeight = new Map<number, number>();
  let maxRowW = 420;
  for (let l = 0; l <= maxLayer; l++) {
    const list = layers.get(l) ?? [];
    let w = 0;
    let h = 0;
    list.forEach((n, idx) => {
      const m = measured.get(n.id)!;
      w += m.width + (idx > 0 ? NODE_GAP_X : 0);
      h = Math.max(h, m.height);
    });
    rowWidth.set(l, w);
    rowHeight.set(l, h);
    maxRowW = Math.max(maxRowW, w);
  }

  // Reserve space for title at top.
  const titleText = diagram.title.trim() || "Sơ đồ quy trình";
  const titleLines = wrapText(titleText, Math.max(maxRowW, 480), TITLE_FONT_SIZE, 600);
  const titleBlockH = titleLines.length * TITLE_LINE_HEIGHT + 28;

  // Position nodes.
  const layoutNodes: LayoutNode[] = [];
  const layoutById = new Map<string, LayoutNode>();
  let cursorY = PAD_TOP + titleBlockH;
  let contentMaxX = PAD_X + maxRowW;

  for (let l = 0; l <= maxLayer; l++) {
    const list = layers.get(l) ?? [];
    const rw = rowWidth.get(l) ?? 0;
    const rh = rowHeight.get(l) ?? 0;
    let cursorX = PAD_X + (maxRowW - rw) / 2;
    for (const node of list) {
      const m = measured.get(node.id)!;
      const ln: LayoutNode = {
        ...node,
        x: Math.round(cursorX),
        y: Math.round(cursorY + (rh - m.height) / 2),
        width: m.width,
        height: m.height,
        layer: l,
        labelLines: m.labelLines,
        roleLines: m.roleLines,
      };
      layoutNodes.push(ln);
      layoutById.set(ln.id, ln);
      contentMaxX = Math.max(contentMaxX, ln.x + ln.width);
      cursorX += m.width + NODE_GAP_X;
    }
    if (list.length > 0) {
      cursorY += rh + LAYER_GAP_Y;
    }
  }

  // Route edges.
  const layoutEdges: LayoutEdge[] = [];
  let backEdgeLane = 0;
  let rightMostX = contentMaxX;

  // Count outgoing forward edges per source to stagger decision anchors cleanly.
  const outForwardBySource = new Map<string, DiagramEdge[]>();
  for (const edge of diagram.edges) {
    const src = layoutById.get(edge.from)!;
    const dst = layoutById.get(edge.to)!;
    const isBack = backEdgePairs.has(`${edge.from}=>${edge.to}`) || dst.layer <= src.layer;
    if (!isBack) {
      const arr = outForwardBySource.get(edge.from) ?? [];
      arr.push(edge);
      outForwardBySource.set(edge.from, arr);
    }
  }

  diagram.edges.forEach((edge, index) => {
    const src = layoutById.get(edge.from)!;
    const dst = layoutById.get(edge.to)!;
    const isBack = backEdgePairs.has(`${edge.from}=>${edge.to}`) || dst.layer <= src.layer;
    const labelLines = edge.label.trim()
      ? wrapText(edge.label, 168, EDGE_FONT_SIZE, 500)
      : [];
    const labelW = labelLines.length > 0 ? Math.ceil(maxLineWidth(labelLines, EDGE_FONT_SIZE, 500) + 18) : 0;
    const labelH = labelLines.length > 0 ? labelLines.length * EDGE_LINE_HEIGHT + 10 : 0;

    let points: Point[];
    let labelAnchor: Point;

    if (isBack) {
      const laneX = Math.round(contentMaxX + 44 + backEdgeLane * 44);
      backEdgeLane += 1;
      const startPt: Point = { x: src.x + src.width, y: Math.round(src.y + src.height / 2) };
      const endPt: Point = { x: dst.x + dst.width, y: Math.round(dst.y + dst.height / 2) };
      points = [
        startPt,
        { x: laneX, y: startPt.y },
        { x: laneX, y: endPt.y },
        endPt,
      ];
      labelAnchor = { x: laneX, y: Math.round((startPt.y + endPt.y) / 2) };
      rightMostX = Math.max(rightMostX, laneX + Math.ceil(labelW / 2) + 20);
    } else {
      const srcCx = Math.round(src.x + src.width / 2);
      const dstCx = Math.round(dst.x + dst.width / 2);
      const siblings = outForwardBySource.get(edge.from) ?? [edge];

      let startPt: Point = { x: srcCx, y: src.y + src.height };
      const endPt: Point = { x: dstCx, y: dst.y };

      // Only anchor to left/right vertex of a decision diamond when the target center lies strictly
      // outside the diamond's horizontal span so the arrow never doubles back across the shape.
      if (src.kind === "decision" && siblings.length >= 2) {
        if (dstCx <= src.x - 16) {
          startPt = { x: src.x, y: Math.round(src.y + src.height / 2) };
        } else if (dstCx >= src.x + src.width + 16) {
          startPt = { x: src.x + src.width, y: Math.round(src.y + src.height / 2) };
        }
      }

      if (Math.abs(startPt.x - endPt.x) <= 4) {
        points = [startPt, endPt];
        labelAnchor = {
          x: startPt.x,
          y: Math.round((startPt.y + endPt.y) / 2),
        };
      } else if (startPt.y < src.y + src.height - 4) {
        // Exited from side of diamond (strictly outside its width): go horizontally to target X,
        // then drop into target top. Place label on the clear vertical drop below the diamond.
        const elbow: Point = { x: endPt.x, y: startPt.y };
        points = [startPt, elbow, endPt];
        const safeTopY = Math.max(startPt.y + Math.ceil(labelH / 2) + 14, src.y + src.height - 8);
        const safeBotY = Math.max(safeTopY, endPt.y - Math.ceil(labelH / 2) - 14);
        labelAnchor = {
          x: endPt.x,
          y: Math.round((safeTopY + safeBotY) / 2),
        };
      } else {
        // Orthogonal Z-bend between layers. Place label on the vertical drop above target so
        // sibling branches from the same node never overlap each other or the source shape.
        const midY = Math.round(startPt.y + (endPt.y - startPt.y) * 0.34);
        points = [
          startPt,
          { x: startPt.x, y: midY },
          { x: endPt.x, y: midY },
          endPt,
        ];
        labelAnchor = {
          x: endPt.x,
          y: Math.round((midY + endPt.y) / 2),
        };
      }
    }

    const labelBox =
      labelLines.length > 0
        ? {
            x: Math.round(labelAnchor.x - labelW / 2),
            y: Math.round(labelAnchor.y - labelH / 2),
            width: labelW,
            height: labelH,
          }
        : null;

    if (labelBox) {
      rightMostX = Math.max(rightMostX, labelBox.x + labelBox.width + 16);
    }

    layoutEdges.push({
      ...edge,
      index,
      isBackEdge: isBack,
      points,
      labelLines,
      labelBox,
    });
  });

  // Ensure no label box bleeds past x = 16 on the left.
  let minX = PAD_X;
  for (const n of layoutNodes) minX = Math.min(minX, n.x);
  for (const e of layoutEdges) {
    if (e.labelBox) minX = Math.min(minX, e.labelBox.x);
  }
  const shiftX = minX < 24 ? Math.ceil(24 - minX) : 0;
  if (shiftX > 0) {
    for (const n of layoutNodes) n.x += shiftX;
    for (const e of layoutEdges) {
      for (const p of e.points) p.x += shiftX;
      if (e.labelBox) e.labelBox.x += shiftX;
    }
    rightMostX += shiftX;
  }

  const totalWidth = Math.ceil(Math.max(rightMostX + PAD_X, 560));
  const totalHeight = Math.ceil(Math.max(cursorY - LAYER_GAP_Y + PAD_BOTTOM, 260));

  return {
    diagram,
    titleLines,
    width: totalWidth,
    height: totalHeight,
    nodes: layoutNodes,
    edges: layoutEdges,
  };
}

/** Renders the laid-out diagram into a hand-drawn Rough.js + Patrick Hand SVG matching Excalidraw. */
export function renderDiagramSvg(layout: LayoutDiagram): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const mk = <K extends keyof SVGElementTagNameMap>(
    tag: K,
    attrs: Record<string, string | number>,
  ): SVGElementTagNameMap[K] => {
    const node = document.createElementNS(ns, tag);
    for (const [k, v] of Object.entries(attrs)) {
      node.setAttribute(k, String(v));
    }
    return node;
  };

  const svg = mk("svg", {
    class: "diagram-svg",
    viewBox: `0 0 ${layout.width} ${layout.height}`,
    width: layout.width,
    height: layout.height,
    role: "img",
    "aria-label": layout.diagram.title,
  });

  const rc = rough.svg(svg);

  // Background
  svg.append(
    mk("rect", {
      x: 0,
      y: 0,
      width: layout.width,
      height: layout.height,
      rx: 10,
      class: "dg-canvas-bg",
    }),
  );

  // Title (Patrick Hand)
  layout.titleLines.forEach((line, i) => {
    const t = mk("text", {
      x: layout.width / 2,
      y: PAD_TOP + 20 + i * TITLE_LINE_HEIGHT,
      "text-anchor": "middle",
      class: "dg-title",
    });
    t.textContent = line;
    svg.append(t);
  });

  // Hand-drawn edges + Excalidraw open-V arrowheads
  for (const edge of layout.edges) {
    const seed = deterministicSeed(`edge-${edge.index}-${edge.from}-${edge.to}`);
    const pts: [number, number][] = edge.points.map((p) => [p.x, p.y]);
    svg.append(
      rc.linearPath(pts, {
        stroke: SKETCH_COLORS.ink,
        strokeWidth: 1.8,
        roughness: 1.25,
        bowing: 1.0,
        seed,
        strokeLineDash: edge.isBackEdge ? [7, 5] : undefined,
      }),
    );
    if (edge.points.length >= 2) {
      const p1 = edge.points[edge.points.length - 2];
      const p2 = edge.points[edge.points.length - 1];
      const [w1, w2] = arrowWingPoints(p1, p2);
      svg.append(
        rc.linearPath(
          [
            [w1.x, w1.y],
            [p2.x, p2.y],
            [w2.x, w2.y],
          ],
          {
            stroke: SKETCH_COLORS.ink,
            strokeWidth: 1.9,
            roughness: 0.9,
            bowing: 0.8,
            seed: seed + 19,
          },
        ),
      );
    }
  }

  // Hand-drawn nodes (solid pastel fills + crisp ink borders)
  for (const node of layout.nodes) {
    const g = mk("g", { class: `dg-node dg-node-${node.kind}` });
    const seed = deterministicSeed(`node-${node.id}`);
    const fill = nodeFillColor(node.kind);
    const shapeOpts = {
      fill,
      fillStyle: "solid" as const,
      stroke: SKETCH_COLORS.ink,
      strokeWidth: 1.85,
      roughness: 1.65,
      bowing: 1.15,
      seed,
    };

    if (node.kind === "decision") {
      const cx = node.x + node.width / 2;
      const cy = node.y + node.height / 2;
      g.append(
        rc.polygon(
          [
            [cx, node.y],
            [node.x + node.width, cy],
            [cx, node.y + node.height],
            [node.x, cy],
          ],
          shapeOpts,
        ),
      );
    } else if (node.kind === "start" || node.kind === "end") {
      g.append(
        rc.path(roundedRectSvgPath(node.x, node.y, node.width, node.height, Math.min(28, node.height / 2)), shapeOpts),
      );
    } else {
      g.append(rc.path(roundedRectSvgPath(node.x, node.y, node.width, node.height, 12), shapeOpts));
    }

    const totalTextH =
      node.labelLines.length * LABEL_LINE_HEIGHT +
      (node.roleLines.length > 0 ? node.roleLines.length * ROLE_LINE_HEIGHT + 6 : 0);
    let textY = node.y + (node.height - totalTextH) / 2;
    const cx = node.x + node.width / 2;

    if (node.roleLines.length > 0) {
      node.roleLines.forEach((line, i) => {
        const t = mk("text", {
          x: cx,
          y: textY + 12 + i * ROLE_LINE_HEIGHT,
          "text-anchor": "middle",
          class: "dg-role",
        });
        t.textContent = line;
        g.append(t);
      });
      textY += node.roleLines.length * ROLE_LINE_HEIGHT + 6;
    }

    node.labelLines.forEach((line, i) => {
      const t = mk("text", {
        x: cx,
        y: textY + 15 + i * LABEL_LINE_HEIGHT,
        "text-anchor": "middle",
        class: "dg-label",
      });
      t.textContent = line;
      g.append(t);
    });

    svg.append(g);
  }

  // Hand-drawn edge label pills on top
  for (const edge of layout.edges) {
    if (!edge.labelBox || edge.labelLines.length === 0) continue;
    const box = edge.labelBox;
    const seed = deterministicSeed(`label-${edge.index}-${edge.label}`);
    svg.append(
      rc.path(roundedRectSvgPath(box.x, box.y, box.width, box.height, 7), {
        fill: SKETCH_COLORS.labelFill,
        fillStyle: "solid",
        stroke: SKETCH_COLORS.labelBorder,
        strokeWidth: 1.15,
        roughness: 1.1,
        bowing: 0.9,
        seed,
      }),
    );
    edge.labelLines.forEach((line, i) => {
      const t = mk("text", {
        x: box.x + box.width / 2,
        y: box.y + 17 + i * EDGE_LINE_HEIGHT,
        "text-anchor": "middle",
        class: "dg-edge-label",
      });
      t.textContent = line;
      svg.append(t);
    });
  }

  return svg;
}

/** Renders a 2x HiDPI hand-drawn PNG via Rough.js Canvas2D using bundled Patrick Hand Vietnamese font. */
export async function renderDiagramPng(layout: LayoutDiagram): Promise<Uint8Array> {
  if (typeof document !== "undefined" && document.fonts?.ready) {
    await document.fonts.ready;
  }
  const scale = 2;
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(layout.width * scale);
  canvas.height = Math.ceil(layout.height * scale);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Không khởi tạo được bộ dựng ảnh PNG.");

  ctx.scale(scale, scale);
  ctx.fillStyle = SKETCH_COLORS.bg;
  ctx.fillRect(0, 0, layout.width, layout.height);

  const rc = rough.canvas(canvas);

  // Title
  ctx.fillStyle = SKETCH_COLORS.ink;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.font = `400 24px ${FONT_FAMILY_HAND}`;
  layout.titleLines.forEach((line, i) => {
    ctx.fillText(line, layout.width / 2, PAD_TOP + 12 + i * TITLE_LINE_HEIGHT);
  });

  // Hand-drawn edges + arrowheads
  for (const edge of layout.edges) {
    const seed = deterministicSeed(`edge-${edge.index}-${edge.from}-${edge.to}`);
    const pts: [number, number][] = edge.points.map((p) => [p.x, p.y]);
    rc.linearPath(pts, {
      stroke: SKETCH_COLORS.ink,
      strokeWidth: 1.8,
      roughness: 1.25,
      bowing: 1.0,
      seed,
      strokeLineDash: edge.isBackEdge ? [7, 5] : undefined,
    });
    if (edge.points.length >= 2) {
      const p1 = edge.points[edge.points.length - 2];
      const p2 = edge.points[edge.points.length - 1];
      const [w1, w2] = arrowWingPoints(p1, p2);
      rc.linearPath(
        [
          [w1.x, w1.y],
          [p2.x, p2.y],
          [w2.x, w2.y],
        ],
        {
          stroke: SKETCH_COLORS.ink,
          strokeWidth: 1.9,
          roughness: 0.9,
          bowing: 0.8,
          seed: seed + 19,
        },
      );
    }
  }

  // Hand-drawn nodes
  for (const node of layout.nodes) {
    const seed = deterministicSeed(`node-${node.id}`);
    const shapeOpts = {
      fill: nodeFillColor(node.kind),
      fillStyle: "solid" as const,
      stroke: SKETCH_COLORS.ink,
      strokeWidth: 1.85,
      roughness: 1.65,
      bowing: 1.15,
      seed,
    };

    if (node.kind === "decision") {
      const cx = node.x + node.width / 2;
      const cy = node.y + node.height / 2;
      rc.polygon(
        [
          [cx, node.y],
          [node.x + node.width, cy],
          [cx, node.y + node.height],
          [node.x, cy],
        ],
        shapeOpts,
      );
    } else if (node.kind === "start" || node.kind === "end") {
      rc.path(roundedRectSvgPath(node.x, node.y, node.width, node.height, Math.min(28, node.height / 2)), shapeOpts);
    } else {
      rc.path(roundedRectSvgPath(node.x, node.y, node.width, node.height, 12), shapeOpts);
    }

    const totalTextH =
      node.labelLines.length * LABEL_LINE_HEIGHT +
      (node.roleLines.length > 0 ? node.roleLines.length * ROLE_LINE_HEIGHT + 6 : 0);
    let textY = node.y + (node.height - totalTextH) / 2;
    const cx = node.x + node.width / 2;

    if (node.roleLines.length > 0) {
      ctx.fillStyle = SKETCH_COLORS.accent;
      ctx.font = `600 11px ${FONT_FAMILY_UI}`;
      node.roleLines.forEach((line, i) => {
        ctx.fillText(line, cx, textY + ROLE_LINE_HEIGHT / 2 + i * ROLE_LINE_HEIGHT);
      });
      textY += node.roleLines.length * ROLE_LINE_HEIGHT + 6;
    }

    ctx.fillStyle = SKETCH_COLORS.ink;
    ctx.font = `400 16.5px ${FONT_FAMILY_HAND}`;
    node.labelLines.forEach((line, i) => {
      ctx.fillText(line, cx, textY + LABEL_LINE_HEIGHT / 2 + i * LABEL_LINE_HEIGHT);
    });
  }

  // Hand-drawn edge labels
  for (const edge of layout.edges) {
    if (!edge.labelBox || edge.labelLines.length === 0) continue;
    const box = edge.labelBox;
    const seed = deterministicSeed(`label-${edge.index}-${edge.label}`);
    rc.path(roundedRectSvgPath(box.x, box.y, box.width, box.height, 7), {
      fill: SKETCH_COLORS.labelFill,
      fillStyle: "solid",
      stroke: SKETCH_COLORS.labelBorder,
      strokeWidth: 1.15,
      roughness: 1.1,
      bowing: 0.9,
      seed,
    });
    ctx.fillStyle = SKETCH_COLORS.ink;
    ctx.font = `400 14.5px ${FONT_FAMILY_HAND}`;
    edge.labelLines.forEach((line, i) => {
      ctx.fillText(line, box.x + box.width / 2, box.y + 5 + EDGE_LINE_HEIGHT / 2 + i * EDGE_LINE_HEIGHT);
    });
  }

  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("Không xuất được dữ liệu ảnh PNG.");
  return new Uint8Array(await blob.arrayBuffer());
}

/** Deterministic fractional-index key for Excalidraw elements. */
function excalidrawIndex(seq: number): string {
  return `a${seq.toString(36).padStart(4, "0")}`;
}

/** Builds a valid Excalidraw v2 `.excalidraw` JSON string with editable shapes, arrows, and Vietnamese text. */
export function buildExcalidrawFile(layout: LayoutDiagram): string {
  const elements: Record<string, unknown>[] = [];
  let seq = 1;
  const nextSeed = () => 100000 + seq * 7919;
  const now = Date.now();

  const baseElement = (id: string, type: string, x: number, y: number, width: number, height: number) => ({
    id,
    type,
    x: Math.round(x),
    y: Math.round(y),
    width: Math.round(width),
    height: Math.round(height),
    angle: 0,
    strokeColor: SKETCH_COLORS.ink,
    backgroundColor: "transparent",
    fillStyle: "solid",
    strokeWidth: 2,
    strokeStyle: "solid",
    roughness: 2,
    opacity: 100,
    groupIds: [] as string[],
    frameId: null,
    index: excalidrawIndex(seq++),
    roundness: null as { type: number } | null,
    seed: nextSeed(),
    version: 1,
    versionNonce: nextSeed() + 17,
    isDeleted: false,
    boundElements: [] as { id: string; type: string }[],
    updated: now,
    link: null,
    locked: false,
  });

  // Title text element (fontFamily 5 = Excalidraw Nunito Sans, full Vietnamese diacritics).
  const titleText = layout.titleLines.join("\n");
  const titleW = Math.max(240, Math.ceil(maxLineWidth(layout.titleLines, TITLE_FONT_SIZE, 600) + 16));
  const titleH = layout.titleLines.length * TITLE_LINE_HEIGHT;
  elements.push({
    ...baseElement("dg-title", "text", (layout.width - titleW) / 2, PAD_TOP, titleW, titleH),
    text: titleText,
    originalText: titleText,
    fontSize: 20,
    fontFamily: 5,
    textAlign: "center",
    verticalAlign: "top",
    containerId: null,
    autoResize: true,
    lineHeight: 1.25,
  });

  // Map node ids to their Excalidraw shape element objects so arrows can register boundElements.
  const shapeByNodeId = new Map<string, Record<string, unknown>>();

  for (const node of layout.nodes) {
    const shapeId = `node-${node.id}`;
    const textId = `text-${node.id}`;
    const exType =
      node.kind === "decision"
        ? "diamond"
        : node.kind === "start" || node.kind === "end"
          ? "ellipse"
          : "rectangle";

    const shape = {
      ...baseElement(shapeId, exType, node.x, node.y, node.width, node.height),
      strokeColor: SKETCH_COLORS.ink,
      backgroundColor: nodeFillColor(node.kind),
      roundness: exType === "rectangle" ? { type: 3 } : null,
      boundElements: [{ id: textId, type: "text" }],
    };
    shapeByNodeId.set(node.id, shape);
    elements.push(shape);

    const combinedLines =
      node.roleLines.length > 0
        ? [`[${node.roleLines.join(" ")}]`, ...node.labelLines]
        : node.labelLines;
    const bodyText = combinedLines.join("\n");
    const textW = Math.max(80, Math.min(node.width - 24, Math.ceil(maxLineWidth(combinedLines, 15, 500) + 8)));
    const textH = combinedLines.length * 20;

    elements.push({
      ...baseElement(
        textId,
        "text",
        node.x + (node.width - textW) / 2,
        node.y + (node.height - textH) / 2,
        textW,
        textH,
      ),
      text: bodyText,
      originalText: bodyText,
      fontSize: 15,
      fontFamily: 5,
      textAlign: "center",
      verticalAlign: "middle",
      containerId: shapeId,
      autoResize: true,
      lineHeight: 1.25,
    });
  }

  for (const edge of layout.edges) {
    const arrowId = `edge-${edge.index}`;
    const start = edge.points[0];
    const relPoints = edge.points.map((p) => [Math.round(p.x - start.x), Math.round(p.y - start.y)]);
    let minDx = 0;
    let maxDx = 0;
    let minDy = 0;
    let maxDy = 0;
    for (const [dx, dy] of relPoints) {
      minDx = Math.min(minDx, dx);
      maxDx = Math.max(maxDx, dx);
      minDy = Math.min(minDy, dy);
      maxDy = Math.max(maxDy, dy);
    }

    const fromShapeId = `node-${edge.from}`;
    const toShapeId = `node-${edge.to}`;
    const fromShape = shapeByNodeId.get(edge.from);
    const toShape = shapeByNodeId.get(edge.to);
    if (fromShape) {
      (fromShape.boundElements as { id: string; type: string }[]).push({ id: arrowId, type: "arrow" });
    }
    if (toShape) {
      (toShape.boundElements as { id: string; type: string }[]).push({ id: arrowId, type: "arrow" });
    }

    const labelId = edge.labelLines.length > 0 ? `edge-label-${edge.index}` : null;
    const arrow = {
      ...baseElement(arrowId, "arrow", start.x, start.y, Math.max(1, maxDx - minDx), Math.max(1, maxDy - minDy)),
      strokeColor: "#475569",
      strokeStyle: edge.isBackEdge ? "dashed" : "solid",
      points: relPoints,
      lastCommittedPoint: null,
      startBinding: { elementId: fromShapeId, focus: 0, gap: 4 },
      endBinding: { elementId: toShapeId, focus: 0, gap: 4 },
      startArrowhead: null,
      endArrowhead: "arrow",
      boundElements: labelId ? [{ id: labelId, type: "text" }] : [],
    };
    elements.push(arrow);

    if (labelId && edge.labelBox) {
      const labelText = edge.labelLines.join("\n");
      elements.push({
        ...baseElement(
          labelId,
          "text",
          edge.labelBox.x + 6,
          edge.labelBox.y + 4,
          Math.max(40, edge.labelBox.width - 12),
          Math.max(20, edge.labelBox.height - 8),
        ),
        strokeColor: "#334155",
        text: labelText,
        originalText: labelText,
        fontSize: 13,
        fontFamily: 5,
        textAlign: "center",
        verticalAlign: "middle",
        containerId: arrowId,
        autoResize: true,
        lineHeight: 1.25,
      });
    }
  }

  return JSON.stringify(
    {
      type: "excalidraw",
      version: 2,
      source: "https://excalidraw.com",
      elements,
      appState: {
        gridSize: null,
        viewBackgroundColor: "#ffffff",
      },
      files: {},
    },
    null,
    2,
  );
}
