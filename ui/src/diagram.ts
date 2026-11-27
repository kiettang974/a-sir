import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { el } from "./dom";
import { icon } from "./icons";
import {
  type AiView,
  ensureMenuDismissListeners,
  overflowMenu,
  PROVIDER_NAME,
  PROVIDERS,
  type Provider,
  type RecordFlowNav,
  recordFlowTabs,
} from "./ipc";

export type DiagramStrategyId =
  | "auto"
  | "process-flow"
  | "state-machine"
  | "explained-flow"
  | "decision"
  | "before-after"
  | "interaction"
  | "architecture"
  | "layer-stack"
  | "data-model"
  | "incident"
  | "mechanism"
  | "funnel"
  | "matrix-2x2"
  | "threshold-plot"
  | "causal-loop"
  | "concept-map";

type DiagramStrategyGroup = "Quy trình & Sản phẩm" | "Kỹ thuật & Dữ liệu" | "Kinh doanh & Tư duy";

type DiagramStrategyOption = {
  id: DiagramStrategyId;
  group: DiagramStrategyGroup;
  label: string;
  desc: string;
  previewSvg: string;
};

const STRATEGY_GROUPS: DiagramStrategyGroup[] = [
  "Quy trình & Sản phẩm",
  "Kỹ thuật & Dữ liệu",
  "Kinh doanh & Tư duy",
];

const DIAGRAM_STRATEGIES: DiagramStrategyOption[] = [
  {
    id: "auto",
    group: "Quy trình & Sản phẩm",
    label: "Tự động theo nội dung",
    desc: "AI tự chọn dạng sơ đồ phù hợp nhất",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M24 5l2.2 5.8L32 13l-5.8 2.2L24 21l-2.2-5.8L16 13l5.8-2.2z" fill="currentColor" fill-opacity="0.14"/><path d="M11 21l1.2 3L15 25l-2.8 1-1.2 3-1.2-3L7 25l2.8-1z"/><path d="M36 20l1 2.2 2.2 1-2.2 1-1 2.2-1-2.2-2.2-1 2.2-1z"/></svg>`,
  },
  {
    id: "process-flow",
    group: "Quy trình & Sản phẩm",
    label: "Swimlane theo vai trò",
    desc: "Quy trình nhiều bộ phận, vai trò",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="42" height="26" rx="2"/><line x1="17" y1="3" x2="17" y2="29" stroke-opacity="0.4"/><line x1="31" y1="3" x2="31" y2="29" stroke-opacity="0.4"/><line x1="3" y1="9" x2="45" y2="9" stroke-opacity="0.4"/><rect x="5.5" y="12" width="9" height="5" rx="1" fill="currentColor" fill-opacity="0.14"/><rect x="19.5" y="18" width="9" height="5" rx="1" fill="currentColor" fill-opacity="0.14"/><rect x="33.5" y="21" width="9" height="5" rx="1" fill="currentColor" fill-opacity="0.14"/><path d="M14.5 14.5h3v6h2"/><path d="M28.5 20.5h3v3h2"/></svg>`,
  },
  {
    id: "state-machine",
    group: "Quy trình & Sản phẩm",
    label: "Vòng đời trạng thái",
    desc: "Chuyển trạng thái đơn hàng, hồ sơ",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="16" r="2.6" fill="currentColor"/><rect x="13" y="11" width="11" height="10" rx="2" fill="currentColor" fill-opacity="0.14"/><rect x="30" y="5" width="11" height="8" rx="2"/><rect x="30" y="19" width="11" height="8" rx="2" stroke-dasharray="2 2"/><path d="M8.6 16H13M24 14l6-4M24 18l6 4"/></svg>`,
  },
  {
    id: "explained-flow",
    group: "Quy trình & Sản phẩm",
    label: "Hướng dẫn từng bước",
    desc: "Cột bước đánh số kèm ví dụ bên cạnh",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="7" cy="8" r="2.5" fill="currentColor"/><rect x="12" y="4.5" width="14" height="7" rx="1.5" fill="currentColor" fill-opacity="0.14"/><rect x="31" y="4.5" width="13" height="7" rx="1.5" stroke-opacity="0.6"/><circle cx="7" cy="22" r="2.5" fill="currentColor"/><rect x="12" y="18.5" width="14" height="7" rx="1.5"/><rect x="31" y="18.5" width="13" height="7" rx="1.5" stroke-opacity="0.6"/><path d="M19 11.5v7M26 8h5M26 22h5" stroke-dasharray="2 2"/></svg>`,
  },
  {
    id: "decision",
    group: "Quy trình & Sản phẩm",
    label: "Cây quyết định",
    desc: "Điều kiện rẽ nhánh tới kết luận",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><polygon points="24,3 32,9 24,15 16,9" fill="currentColor" fill-opacity="0.14"/><rect x="5" y="21" width="13" height="7" rx="1.5"/><rect x="30" y="21" width="13" height="7" rx="1.5" fill="currentColor" fill-opacity="0.18"/><path d="M16 9H11.5v12M32 9h4.5v12"/></svg>`,
  },
  {
    id: "before-after",
    group: "Quy trình & Sản phẩm",
    label: "Trước – Sau",
    desc: "So sánh hiện trạng và đề xuất mới",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="24" rx="2" stroke-dasharray="2 2" stroke-opacity="0.5"/><rect x="27" y="4" width="18" height="24" rx="2"/><rect x="6" y="10" width="12" height="5" rx="1" stroke-opacity="0.5"/><rect x="6" y="19" width="12" height="5" rx="1"/><rect x="30" y="10" width="12" height="5" rx="1" stroke-opacity="0.5"/><rect x="30" y="19" width="12" height="5" rx="1" fill="currentColor" fill-opacity="0.22"/><path d="M22 16h4m-1.5-2l1.5 2-1.5 2"/></svg>`,
  },
  {
    id: "interaction",
    group: "Kỹ thuật & Dữ liệu",
    label: "Tuần tự tương tác (Sequence)",
    desc: "Ai gọi ai, luồng API / trao đổi",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="3" width="10" height="5" rx="1" fill="currentColor" fill-opacity="0.14"/><rect x="19" y="3" width="10" height="5" rx="1" fill="currentColor" fill-opacity="0.14"/><rect x="34" y="3" width="10" height="5" rx="1" fill="currentColor" fill-opacity="0.14"/><line x1="9" y1="8" x2="9" y2="29" stroke-dasharray="2 2" stroke-opacity="0.5"/><line x1="24" y1="8" x2="24" y2="29" stroke-dasharray="2 2" stroke-opacity="0.5"/><line x1="39" y1="8" x2="39" y2="29" stroke-dasharray="2 2" stroke-opacity="0.5"/><path d="M9 13h14m-2-2l2 2-2 2"/><path d="M24 19h14m-2-2l2 2-2 2"/><path d="M39 25H10m2-2l-2 2 2 2"/></svg>`,
  },
  {
    id: "architecture",
    group: "Kỹ thuật & Dữ liệu",
    label: "Kiến trúc hệ thống",
    desc: "Dịch vụ, luồng Ghi/Đọc, DB & hàng đợi",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="10" width="10" height="12" rx="1.5" fill="currentColor" fill-opacity="0.12"/><rect x="19" y="5" width="11" height="8" rx="1.5"/><rect x="19" y="19" width="11" height="8" rx="1.5"/><ellipse cx="40" cy="12" rx="4.5" ry="2"/><path d="M35.5 12v8c0 1.1 2 2 4.5 2s4.5-.9 4.5-2v-8"/><path d="M13 14l6-4M13 18l6 4M30 9l5.5 4M30 23l5.5-4"/></svg>`,
  },
  {
    id: "layer-stack",
    group: "Kỹ thuật & Dữ liệu",
    label: "Kiến trúc phân tầng",
    desc: "Xếp lớp nền tảng → nghiệp vụ → kênh",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="4" width="36" height="6" rx="1.5"/><rect x="6" y="13" width="36" height="6" rx="1.5" fill="currentColor" fill-opacity="0.2"/><rect x="6" y="22" width="36" height="6" rx="1.5"/><path d="M24 10v3M24 19v3"/></svg>`,
  },
  {
    id: "data-model",
    group: "Kỹ thuật & Dữ liệu",
    label: "Mô hình dữ liệu (ER)",
    desc: "Thực thể, khoá chính và quan hệ bảng",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="6" width="15" height="20" rx="1.5"/><line x1="3" y1="11.5" x2="18" y2="11.5"/><line x1="6" y1="16" x2="14" y2="16" stroke-opacity="0.5"/><line x1="6" y1="20" x2="15" y2="20" stroke-opacity="0.5"/><rect x="30" y="6" width="15" height="20" rx="1.5"/><line x1="30" y1="11.5" x2="45" y2="11.5"/><line x1="33" y1="16" x2="41" y2="16" stroke-opacity="0.5"/><line x1="33" y1="20" x2="42" y2="20" stroke-opacity="0.5"/><path d="M18 16h12M21 13.5v5M30 16l-3.5-2.5M30 16l-3.5 2.5"/></svg>`,
  },
  {
    id: "incident",
    group: "Kỹ thuật & Dữ liệu",
    label: "Phân tích sự cố (4 chặng)",
    desc: "Chuyện gì → Vì sao → Tác động → Phòng ngừa",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="19" height="9" rx="1.5"/><rect x="26" y="5" width="19" height="9" rx="1.5" fill="currentColor" fill-opacity="0.14"/><rect x="3" y="18" width="19" height="9" rx="1.5"/><rect x="26" y="18" width="19" height="9" rx="1.5"/><circle cx="7" cy="9.5" r="2" fill="currentColor"/><circle cx="30" cy="9.5" r="2" fill="currentColor"/><circle cx="7" cy="22.5" r="2" fill="currentColor"/><circle cx="30" cy="22.5" r="2" fill="currentColor"/><path d="M22 9.5h4M22 22.5h4"/></svg>`,
  },
  {
    id: "mechanism",
    group: "Kỹ thuật & Dữ liệu",
    label: "Cơ chế & Trục thời gian",
    desc: "Dải ô, timeout, hoạt động bên trong",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="25" x2="44" y2="25"/><path d="M42 23l2 2-2 2"/><rect x="6" y="7" width="12" height="5" rx="1" fill="currentColor" fill-opacity="0.14"/><rect x="14" y="14" width="18" height="5" rx="1" fill="currentColor" fill-opacity="0.22"/><line x1="32" y1="5" x2="32" y2="25" stroke-dasharray="2 2"/><line x1="10" y1="23" x2="10" y2="27"/><line x1="22" y1="23" x2="22" y2="27"/><line x1="34" y1="23" x2="34" y2="27"/></svg>`,
  },
  {
    id: "funnel",
    group: "Kinh doanh & Tư duy",
    label: "Phễu chuyển đổi (Funnel)",
    desc: "Tỷ lệ rơi từng bước bán hàng / tuyển dụng",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><polygon points="6,4 42,4 37,11 11,11" fill="currentColor" fill-opacity="0.1"/><polygon points="12,13 36,13 32,20 16,20" fill="currentColor" fill-opacity="0.18"/><polygon points="17,22 31,22 28,28 20,28" fill="currentColor" fill-opacity="0.28"/></svg>`,
  },
  {
    id: "matrix-2x2",
    group: "Kinh doanh & Tư duy",
    label: "Ma trận 2×2",
    desc: "So sánh ưu tiên, rủi ro – giá trị",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M8 27V4m-2 2l2-2 2 2"/><path d="M8 27h35m-2-2l2 2-2 2"/><rect x="12" y="6" width="13" height="8" rx="1"/><rect x="27" y="6" width="13" height="8" rx="1" fill="currentColor" fill-opacity="0.22"/><rect x="12" y="16" width="13" height="8" rx="1"/><rect x="27" y="16" width="13" height="8" rx="1"/></svg>`,
  },
  {
    id: "threshold-plot",
    group: "Kinh doanh & Tư duy",
    label: "Biểu đồ ngưỡng & KPI",
    desc: "So sánh số liệu và điểm vượt ngưỡng",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M7 27V5M7 27h36"/><rect x="11" y="17" width="6" height="10" rx="1" fill="currentColor" fill-opacity="0.14"/><rect x="20" y="12" width="6" height="15" rx="1" fill="currentColor" fill-opacity="0.14"/><rect x="29" y="7" width="6" height="20" rx="1" fill="currentColor" fill-opacity="0.28"/><line x1="7" y1="11" x2="43" y2="11" stroke-dasharray="2 2"/></svg>`,
  },
  {
    id: "causal-loop",
    group: "Kinh doanh & Tư duy",
    label: "Vòng lặp nhân quả",
    desc: "Tác động qua lại & điểm đòn bẩy",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="17" y="3" width="14" height="6" rx="2" fill="currentColor" fill-opacity="0.14"/><rect x="17" y="23" width="14" height="6" rx="2"/><path d="M31 6c7 2 8 15 1 19m2-1l-2 1 1-2"/><path d="M17 26c-7-2-8-15-1-19m-2 1l2-1-1 2"/></svg>`,
  },
  {
    id: "concept-map",
    group: "Kinh doanh & Tư duy",
    label: "Bản đồ khái niệm",
    desc: "Liên kết ý chính có động từ nối",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="24" cy="16" rx="7" ry="4.5" fill="currentColor" fill-opacity="0.2"/><rect x="3" y="5" width="10" height="6" rx="1.5"/><rect x="3" y="21" width="10" height="6" rx="1.5"/><rect x="35" y="5" width="10" height="6" rx="1.5"/><rect x="35" y="21" width="10" height="6" rx="1.5"/><path d="M13 9l5 4M13 23l5-4M30 13l5-4M30 19l5 4"/></svg>`,
  },
];

function parsePreviewSvg(markup: string, className: string): SVGSVGElement {
  const doc = new DOMParser().parseFromString(markup, "image/svg+xml");
  const svg = document.importNode(doc.documentElement, true) as unknown as SVGSVGElement;
  svg.setAttribute("class", className);
  svg.setAttribute("aria-hidden", "true");
  return svg;
}

export type UncertaintyDisplayMode = "off" | "footer" | "inline" | "badges-footer";

type UncertaintyModeOption = {
  id: UncertaintyDisplayMode;
  label: string;
  desc: string;
  previewSvg: string;
};

const UNCERTAINTY_MODES: UncertaintyModeOption[] = [
  {
    id: "off",
    label: "Chỉ hiện bên dưới",
    desc: "Giữ sơ đồ sạch, xem mục chưa chốt dưới khung",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="4" width="14" height="8" rx="1.5" fill="currentColor" fill-opacity="0.12"/><rect x="28" y="4" width="14" height="8" rx="1.5"/><path d="M20 8h8"/><line x1="6" y1="20" x2="42" y2="20" stroke-opacity="0.35"/><line x1="8" y1="25" x2="30" y2="25" stroke-opacity="0.5"/><line x1="8" y1="29" x2="24" y2="29" stroke-opacity="0.35"/></svg>`,
  },
  {
    id: "footer",
    label: "Khung chân hình (PA 1)",
    desc: "Tự ghép khung nét đứt vàng dưới sơ đồ, không cần gọi AI",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="3" width="14" height="8" rx="1.5"/><rect x="28" y="3" width="14" height="8" rx="1.5"/><path d="M20 7h8"/><rect x="6" y="16" width="36" height="13" rx="2" stroke-dasharray="3 2" fill="currentColor" fill-opacity="0.16"/><line x1="10" y1="21" x2="32" y2="21"/><line x1="10" y1="25" x2="26" y2="25" stroke-opacity="0.6"/></svg>`,
  },
  {
    id: "inline",
    label: "Nhánh nét đứt trong luồng (PA 2)",
    desc: "AI vẽ thẳng hộp/nhánh nét đứt vàng ngay tại bước liên quan",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="4" width="14" height="9" rx="1.5"/><rect x="30" y="4" width="14" height="9" rx="1.5"/><path d="M18 8.5h12"/><rect x="26" y="19" width="18" height="9" rx="1.5" stroke-dasharray="3 2" fill="currentColor" fill-opacity="0.18"/><path d="M11 13v10.5h15" stroke-dasharray="2 2"/></svg>`,
  },
  {
    id: "badges-footer",
    label: "Mốc ?1, ?2 + Khung chú giải (PA 3)",
    desc: "Gắn chấm ?1, ?2 trên bước liên quan + bảng chú giải chân hình",
    previewSvg: `<svg viewBox="0 0 48 32" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="4" width="14" height="8" rx="1.5"/><rect x="29" y="4" width="14" height="8" rx="1.5"/><path d="M19 8h10"/><circle cx="19" cy="4" r="3" fill="currentColor"/><circle cx="43" cy="4" r="3" fill="currentColor"/><rect x="5" y="17" width="38" height="12" rx="2" stroke-dasharray="3 2" fill="currentColor" fill-opacity="0.16"/><line x1="10" y1="21.5" x2="34" y2="21.5"/><line x1="10" y1="25.5" x2="28" y2="25.5" stroke-opacity="0.6"/></svg>`,
  },
];

function findUncertaintyModeOption(id?: string | null): UncertaintyModeOption {
  return UNCERTAINTY_MODES.find((m) => m.id === id) ?? UNCERTAINTY_MODES[2];
}

function findStrategyOption(id?: string | null): DiagramStrategyOption {
  return DIAGRAM_STRATEGIES.find((s) => s.id === id) ?? DIAGRAM_STRATEGIES[0];
}

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
  veElements?: unknown[];
  uncertainties: DiagramUncertainty[];
  provider?: string;
  model: string;
  strategy?: string | null;
  uncertaintyMode?: string | null;
};

type RenderedDiagram = {
  width: number;
  height: number;
  svg: string;
  excalidrawJson: string;
};

function parseRustSvg(svgMarkup: string, zoom: number, width: number, height: number): SVGSVGElement {
  const doc = new DOMParser().parseFromString(svgMarkup, "image/svg+xml");
  const svg = doc.documentElement as unknown as SVGSVGElement;
  svg.setAttribute("width", String(Math.round(width * zoom)));
  svg.setAttribute("height", String(Math.round(height * zoom)));
  return document.importNode(svg, true);
}

async function rasterizeSvgToPng(svgMarkup: string, width: number, height: number): Promise<Uint8Array> {
  if (typeof document !== "undefined" && document.fonts?.ready) {
    await document.fonts.ready;
  }
  const scale = 2;
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(width * scale);
  canvas.height = Math.ceil(height * scale);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Không khởi tạo được bộ dựng ảnh PNG.");

  const blob = new Blob([svgMarkup], { type: "image/svg+xml;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  try {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const image = new Image();
      image.onload = () => resolve(image);
      image.onerror = () => reject(new Error("Không nạp được SVG để xuất PNG."));
      image.src = url;
    });
    ctx.scale(scale, scale);
    ctx.drawImage(img, 0, 0, width, height);
  } finally {
    URL.revokeObjectURL(url);
  }

  const outBlob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!outBlob) throw new Error("Không xuất được dữ liệu ảnh PNG.");
  return new Uint8Array(await outBlob.arrayBuffer());
}

type Links = {
  openSettings: () => void;
  openSummary: () => void;
  back: () => void;
  flow: RecordFlowNav;
};

type JobDiagramState = {
  running: boolean;
  statusMessage: string;
  error: string | null;
  needsSettings: boolean;
  savedPath: string | null;
  diagram: ProcessDiagram | null;
  rendered: RenderedDiagram | null;
  selectedStrategy: DiagramStrategyId;
  uncertaintyMode: UncertaintyDisplayMode;
  exportError: { format: "png" | "excalidraw"; message: string } | null;
  exportNotice: string | null;
};

const jobStates = new Map<number, JobDiagramState>();
const jobListeners = new Map<number, Set<() => void>>();

function getState(id: number): JobDiagramState {
  let st = jobStates.get(id);
  if (!st) {
    st = {
      running: false,
      statusMessage: "",
      error: null,
      needsSettings: false,
      savedPath: null,
      diagram: null,
      rendered: null,
      selectedStrategy: "auto",
      uncertaintyMode: "inline",
      exportError: null,
      exportNotice: null,
    };
    jobStates.set(id, st);
  }
  return st;
}

function notify(id: number) {
  const listeners = jobListeners.get(id);
  if (!listeners) return;
  for (const fn of listeners) fn();
}

export function isDiagramGenerating(id: number): boolean {
  return getState(id).running;
}

export function getDiagramButtonState(id: number): "idle" | "running" | "ready" | "error" {
  const st = getState(id);
  if (st.running) return "running";
  if (st.diagram && st.rendered) return "ready";
  if (st.error) return "error";
  return "idle";
}

function pickConfiguredProvider(ai: AiView | null): { provider: Provider; name: string; model: string } | null {
  if (!ai) return null;
  const usable = PROVIDERS.filter((p) => Boolean(ai.keys[p]));
  if (usable.length === 0) return null;
  const provider = usable.includes(ai.defaultProvider) ? ai.defaultProvider : usable[0];
  return {
    provider,
    name: PROVIDER_NAME[provider],
    model: ai.models[provider]?.trim() || "",
  };
}

/**
 * Opens an existing saved diagram immediately, or starts AI generation in place while
 * keeping the user on the Reader screen. Never auto-navigates away when generation finishes;
 * the user clicks the button again to open the Diagram screen.
 */
export async function openOrGenerateDiagram(
  id: number,
  callbacks: {
    onStateChange: () => void;
    onOpenDiagram: (jobId: number) => void;
    onFinished?: (jobId: number, error: string | null) => void;
  },
): Promise<void> {
  const st = getState(id);
  if (st.running) return;
  if ((st.diagram && st.rendered) || st.error) {
    callbacks.onOpenDiagram(id);
    return;
  }

  // Check local saved diagram first (offline-safe, zero AI calls).
  try {
    const saved = await invoke<ProcessDiagram | null>("load_diagram", { id });
    if (saved) {
      st.rendered = await invoke<RenderedDiagram>("render_diagram", { diagram: saved });
      st.diagram = saved;
      callbacks.onOpenDiagram(id);
      return;
    }
  } catch {
    // Saved file missing or unreadable; proceed to check AI setup.
  }

  const settings = await invoke<AiView>("ai_settings").catch(() => null);
  const picked = pickConfiguredProvider(settings);
  if (!picked) {
    st.error = "Chưa có API key cho nhà cung cấp AI nào. Hãy thiết lập trong Cài đặt trước khi tạo sơ đồ.";
    st.needsSettings = true;
    callbacks.onOpenDiagram(id);
    return;
  }

  const unsub = subscribe(id, callbacks.onStateChange);
  try {
    await runGeneration(id, picked.name, picked.model);
  } finally {
    unsub();
  }
  callbacks.onStateChange();
  callbacks.onFinished?.(id, getState(id).error);
}

function subscribe(id: number, fn: () => void): () => void {
  let set = jobListeners.get(id);
  if (!set) {
    set = new Set();
    jobListeners.set(id, set);
  }
  set.add(fn);
  return () => {
    set?.delete(fn);
    if (set?.size === 0) jobListeners.delete(id);
  };
}

function sanitizeFileStem(raw: string): string {
  const cleaned = raw
    .trim()
    .replace(/[\\/:*?"<>|]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return cleaned || "so-do-quy-trinh";
}

function errorNeedsSettings(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("api key") ||
    lower.includes("chưa có api key") ||
    lower.includes("không có quyền") ||
    lower.includes("hết hạn mức") ||
    lower.includes("limit: 0") ||
    lower.includes("giới hạn ngữ cảnh") ||
    lower.includes("model")
  );
}

/** Runs diagram generation for `id` independently of whichever recording is currently on screen. */
async function runGeneration(
  id: number,
  providerName: string,
  modelName: string,
  strategy?: DiagramStrategyId,
  uncertaintyMode?: UncertaintyDisplayMode,
) {
  const st = getState(id);
  if (st.running) return;
  const chosen = strategy ?? st.selectedStrategy ?? "auto";
  const chosenUncertainty = uncertaintyMode ?? st.uncertaintyMode ?? "inline";
  st.selectedStrategy = chosen;
  st.uncertaintyMode = chosenUncertainty;
  const opt = findStrategyOption(chosen);
  const isRegenerate = st.diagram !== null;
  const tag = modelName ? `${providerName} (${modelName})` : providerName;
  const styleHint = chosen === "auto" ? "" : ` theo kiểu ${opt.label}`;
  st.running = true;
  st.error = null;
  st.needsSettings = false;
  st.exportError = null;
  st.exportNotice = null;
  st.statusMessage = isRegenerate
    ? `Đang tạo lại sơ đồ${styleHint} bằng ${tag}… Bản sơ đồ hiện tại vẫn có thể xem và xuất.`
    : `Đang phân tích bản chép lời và tạo sơ đồ${styleHint} bằng ${tag}…`;
  notify(id);

  try {
    const nextDiagram = await invoke<ProcessDiagram>("generate_diagram", {
      id,
      strategy: chosen === "auto" ? null : chosen,
      uncertaintyMode: chosenUncertainty,
    });
    let nextRendered: RenderedDiagram;
    try {
      nextRendered = await invoke<RenderedDiagram>("render_diagram", { diagram: nextDiagram });
    } catch (renderErr) {
      throw new Error(
        `Dựng hình sơ đồ thất bại: ${renderErr instanceof Error ? renderErr.message : String(renderErr)}`,
      );
    }

    let savedPath: string;
    try {
      savedPath = await invoke<string>("save_diagram", { id, diagram: nextDiagram });
    } catch (saveErr) {
      throw new Error(
        `Không lưu được sơ đồ mới vào máy (${String(saveErr)}). Bản sơ đồ trước đó vẫn được giữ nguyên.`,
      );
    }

    st.diagram = nextDiagram;
    st.rendered = nextRendered;
    st.savedPath = savedPath;
    st.error = null;
    st.needsSettings = false;
    st.exportNotice = isRegenerate ? "Đã tạo lại và lưu sơ đồ mới." : null;
  } catch (e) {
    const raw = e instanceof Error ? e.message : String(e);
    st.error = isRegenerate ? `Tạo lại sơ đồ thất bại: ${raw}` : raw;
    st.needsSettings = errorNeedsSettings(raw);
  } finally {
    st.running = false;
    st.statusMessage = "";
    notify(id);
  }
}

export function diagramPane(
  id: number,
  name: string,
  hasTranscriptText: boolean,
  links: Links,
): HTMLElement {
  const head = el("header", { className: "reader-head" });
  const tabs = recordFlowTabs("diagram", links.flow);
  const scroll = el("div", { className: "diagram-scroll" });
  const pane = el("section", { className: "reader" }, head, tabs, scroll);

  let ai: AiView | null = null;
  let loadedLocal = false;
  let loadError: string | null = null;
  let zoom = 0.75;

  const openModal = (diagram: ProcessDiagram, rendered: RenderedDiagram) => {
    let modalZoom = 1;
    const stage = el("div", { className: "diagram-modal-stage" });
    const zoomLabel = el("span", { className: "diagram-zoom-val" });

    const renderModalSvg = () => {
      zoomLabel.textContent = `${Math.round(modalZoom * 100)}%`;
      stage.replaceChildren(parseRustSvg(rendered.svg, modalZoom, rendered.width, rendered.height));
    };

    const close = () => {
      document.removeEventListener("keydown", onKey);
      backdrop.remove();
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") close();
    };
    document.addEventListener("keydown", onKey);

    const modalHead = el(
      "header",
      { className: "diagram-modal-head" },
      el("h3", { textContent: diagram.title || name }),
      el(
        "div",
        { className: "diagram-zoom-group" },
        el("button", {
          className: "btn-ghost diagram-zoom-btn",
          title: "Thu nhỏ",
          ariaLabel: "Thu nhỏ sơ đồ",
          textContent: "−",
          onclick: () => {
            modalZoom = Math.max(0.35, Math.round((modalZoom - 0.15) * 100) / 100);
            renderModalSvg();
          },
        }),
        zoomLabel,
        el("button", {
          className: "btn-ghost diagram-zoom-btn",
          title: "Phóng to",
          ariaLabel: "Phóng to sơ đồ",
          textContent: "+",
          onclick: () => {
            modalZoom = Math.min(2.5, Math.round((modalZoom + 0.15) * 100) / 100);
            renderModalSvg();
          },
        }),
        el("button", {
          className: "link",
          textContent: "Vừa khung",
          onclick: () => {
            const availW = Math.max(360, stage.clientWidth - 48);
            const availH = Math.max(320, stage.clientHeight - 48);
            const fit = Math.min(1.5, availW / rendered.width, availH / rendered.height);
            modalZoom = Math.max(0.35, Math.round(fit * 100) / 100);
            renderModalSvg();
          },
        }),
        el("button", {
          className: "link",
          textContent: "100%",
          onclick: () => {
            modalZoom = 1;
            renderModalSvg();
          },
        }),
        el("span", { className: "diagram-stage-sep" }),
        el("button", {
          className: "btn-ghost",
          textContent: "Xuất PNG",
          onclick: () => void exportFile("png"),
        }),
        el("button", {
          className: "btn-primary",
          textContent: "Đóng",
          onclick: close,
        }),
      ),
    );

    const dialog = el(
      "div",
      {
        className: "diagram-modal",
        role: "dialog",
        ariaLabel: diagram.title || "Xem sơ đồ phóng to",
        onclick: (ev: MouseEvent) => ev.stopPropagation(),
      },
      modalHead,
      stage,
    );
    const backdrop = el(
      "div",
      { className: "diagram-modal-backdrop", onclick: close },
      dialog,
    );
    document.body.append(backdrop);
    renderModalSvg();
  };

  const activeAi = () => pickConfiguredProvider(ai);
  const hasAnyAiKey = () => activeAi() !== null;

  const exportFile = async (format: "png" | "excalidraw") => {
    const st = getState(id);
    if (!st.rendered || !st.diagram) return;
    st.exportError = null;
    st.exportNotice = null;
    draw();

    const ext = format === "png" ? "png" : "excalidraw";
    const filterName = format === "png" ? "Ảnh PNG" : "Tệp Excalidraw";
    const defaultPath = `${sanitizeFileStem(st.diagram.title || name)}.${ext}`;
    const chosen = await saveDialog({
      defaultPath,
      filters: [{ name: filterName, extensions: [ext] }],
    });
    if (!chosen) return;

    try {
      let bytes: Uint8Array;
      if (format === "png") {
        bytes = await rasterizeSvgToPng(st.rendered.svg, st.rendered.width, st.rendered.height);
      } else {
        bytes = new TextEncoder().encode(st.rendered.excalidrawJson);
      }
      await invoke<string>("export_diagram_file", {
        path: chosen,
        bytes: Array.from(bytes),
      });
      st.exportNotice = `Đã xuất tệp ${ext.toUpperCase()}: ${chosen.split(/[\\/]/).pop() ?? chosen}`;
    } catch (e) {
      st.exportError = {
        format,
        message: String(e),
      };
    }
    draw();
  };

  const triggerGenerate = (
    strategy?: DiagramStrategyId,
    uncertaintyMode?: UncertaintyDisplayMode,
  ) => {
    if (!hasTranscriptText) return;
    const picked = activeAi();
    if (!picked) {
      const st = getState(id);
      st.error = "Chưa có API key cho nhà cung cấp AI nào. Hãy thiết lập trong Cài đặt trước khi tạo sơ đồ.";
      st.needsSettings = true;
      draw();
      return;
    }
    void runGeneration(id, picked.name, picked.model, strategy, uncertaintyMode);
  };

  const applyUncertaintyMode = async (mode: UncertaintyDisplayMode) => {
    const st = getState(id);
    st.uncertaintyMode = mode;
    if (!st.diagram) {
      draw();
      return;
    }
    // Modes "off" and "footer" re-render immediately in Rust with zero AI calls.
    if (mode === "off" || mode === "footer") {
      const updated: ProcessDiagram = { ...st.diagram, uncertaintyMode: mode };
      try {
        const nextRendered = await invoke<RenderedDiagram>("render_diagram", { diagram: updated });
        st.savedPath = await invoke<string>("save_diagram", { id, diagram: updated });
        st.diagram = updated;
        st.rendered = nextRendered;
        st.exportNotice =
          mode === "footer"
            ? "Đã ghép khung Cần xác nhận vào chân sơ đồ."
            : "Đã ẩn khung Cần xác nhận khỏi sơ đồ.";
      } catch (e) {
        st.error = `Không cập nhật được hiển thị Cần xác nhận: ${e}`;
      }
      draw();
      return;
    }
    // Modes "inline" and "badges-footer" ask AI to draw inline dashed branches or ?1/?2 badges on the flow.
    triggerGenerate(st.selectedStrategy, mode);
  };

  const renderUncertaintyDropdown = (st: JobDiagramState): HTMLElement => {
    ensureMenuDismissListeners();
    const current = findUncertaintyModeOption(st.uncertaintyMode);
    const details = el("details", { className: "diagram-type-menu" }) as HTMLDetailsElement;
    const summary = el(
      "summary",
      {
        className: `btn-ghost diagram-type-trigger${st.running ? " disabled" : ""}`,
        title: "Chọn cách hiển thị các mục Cần xác nhận trên sơ đồ",
      },
      parsePreviewSvg(current.previewSvg, "diagram-type-thumb-sm"),
      el("span", { className: "diagram-type-trigger-label", textContent: `Cần xác nhận: ${current.label}` }),
      el("span", { className: "diagram-type-caret", textContent: "▾" }),
    );
    summary.setAttribute("aria-label", `Hiển thị Cần xác nhận (${current.label})`);

    const panel = el(
      "div",
      { className: "diagram-type-panel", role: "menu", ariaLabel: "Cách hiển thị Cần xác nhận" },
      ...UNCERTAINTY_MODES.map((opt) =>
        el(
          "button",
          {
            className: `diagram-type-item${opt.id === current.id ? " active" : ""}`,
            role: "menuitemradio",
            disabled: st.running,
            onclick: () => {
              details.open = false;
              void applyUncertaintyMode(opt.id);
            },
          },
          el("span", { className: "diagram-type-thumb" }, parsePreviewSvg(opt.previewSvg, "diagram-type-svg")),
          el(
            "span",
            { className: "diagram-type-meta" },
            el("span", { className: "diagram-type-name", textContent: opt.label }),
            el("span", { className: "diagram-type-desc", textContent: opt.desc }),
          ),
        ),
      ),
    );
    details.append(summary, panel);
    return details;
  };

  const renderStrategyDropdown = (st: JobDiagramState): HTMLElement => {
    ensureMenuDismissListeners();
    const current = findStrategyOption(st.selectedStrategy);
    const details = el("details", { className: "diagram-type-menu" }) as HTMLDetailsElement;
    const summary = el(
      "summary",
      {
        className: `btn-ghost diagram-type-trigger${st.running ? " disabled" : ""}`,
        title: "Đổi kiểu sơ đồ và tạo lại bằng AI",
      },
      parsePreviewSvg(current.previewSvg, "diagram-type-thumb-sm"),
      el("span", { className: "diagram-type-trigger-label", textContent: `Kiểu: ${current.label}` }),
      el("span", { className: "diagram-type-caret", textContent: "▾" }),
    );
    summary.setAttribute("aria-label", `Đổi kiểu sơ đồ (hiện tại: ${current.label})`);

    const panel = el(
      "div",
      { className: "diagram-type-panel", role: "menu", ariaLabel: "Chọn kiểu sơ đồ" },
      ...STRATEGY_GROUPS.flatMap((group) => [
        el("div", { className: "diagram-type-group", textContent: group }),
        ...DIAGRAM_STRATEGIES.filter((opt) => opt.group === group).map((opt) =>
          el(
            "button",
            {
              className: `diagram-type-item${opt.id === current.id ? " active" : ""}`,
              role: "menuitemradio",
              disabled: st.running || !hasTranscriptText,
              onclick: () => {
                details.open = false;
                st.selectedStrategy = opt.id;
                triggerGenerate(opt.id);
              },
            },
            el("span", { className: "diagram-type-thumb" }, parsePreviewSvg(opt.previewSvg, "diagram-type-svg")),
            el(
              "span",
              { className: "diagram-type-meta" },
              el("span", { className: "diagram-type-name", textContent: opt.label }),
              el("span", { className: "diagram-type-desc", textContent: opt.desc }),
            ),
          ),
        ),
      ]),
    );
    details.append(summary, panel);
    return details;
  };

  const renderNoticeBanner = (st: JobDiagramState): HTMLElement | null => {
    if (st.running && st.statusMessage) {
      return el(
        "div",
        { className: "diagram-banner", role: "status" },
        el("span", { textContent: st.statusMessage }),
      );
    }
    if (st.error) {
      return el(
        "div",
        { className: "diagram-banner bad", role: "alert" },
        el("span", { textContent: st.error }),
        el(
          "div",
          { className: "diagram-banner-actions" },
          st.needsSettings
            ? el("button", {
                className: "link",
                textContent: "Mở Cài đặt",
                onclick: links.openSettings,
              })
            : null,
          hasTranscriptText && hasAnyAiKey()
            ? el("button", {
                className: "link",
                textContent: "Thử lại",
                onclick: () => triggerGenerate(),
              })
            : null,
        ),
      );
    }
    if (st.exportError) {
      const fmt = st.exportError.format;
      return el(
        "div",
        { className: "diagram-banner bad", role: "alert" },
        el("span", { textContent: st.exportError.message }),
        el(
          "div",
          { className: "diagram-banner-actions" },
          el("button", {
            className: "link",
            textContent: "Chọn nơi lưu khác",
            onclick: () => void exportFile(fmt),
          }),
        ),
      );
    }
    if (st.exportNotice) {
      return el(
        "div",
        { className: "diagram-banner", role: "status" },
        el("span", { textContent: st.exportNotice }),
      );
    }
    return null;
  };

  const draw = () => {
    if (!pane.isConnected && loadedLocal) return;
    const st = getState(id);

    if (!loadedLocal) {
      head.replaceChildren(
        el("div", { className: "reader-head-top" }, el("h2", { textContent: name })),
      );
      scroll.replaceChildren(
        el("p", { className: "hint", textContent: "Đang mở dữ liệu sơ đồ trên máy…" }),
      );
      return;
    }

    if (st.diagram && st.rendered) {
      const rendered = st.rendered;
      const diagram = st.diagram;

      const picked = activeAi();
      const overflowItems = [
        {
          label: st.running ? "Đang tạo lại sơ đồ…" : "Tạo lại sơ đồ",
          title: picked
            ? `Gửi bản chép lời đến ${picked.name}${picked.model ? ` (${picked.model})` : ""} để tạo lại`
            : "Tạo lại sơ đồ bằng AI đã cấu hình",
          disabled: st.running || !hasTranscriptText,
          onClick: () => triggerGenerate(),
        },
        ...(st.savedPath
          ? [
              {
                label: "Mở tệp đã lưu trong Finder",
                onClick: () => void revealItemInDir(st.savedPath!),
              },
            ]
          : []),
      ];

      head.replaceChildren(
        el("div", { className: "reader-head-top" }, el("h2", { textContent: diagram.title || name })),
        el(
          "div",
          { className: "reader-actions" },
          el(
            "button",
            {
              className: "btn-primary",
              title: "Xuất ảnh PNG độ nét cao để chèn vào tài liệu",
              onclick: () => void exportFile("png"),
            },
            el("span", { textContent: "Xuất PNG" }),
          ),
          el(
            "button",
            {
              className: "btn-ghost",
              title: "Xuất tệp .excalidraw để mở và chỉnh sửa trong Excalidraw",
              onclick: () => void exportFile("excalidraw"),
            },
            el("span", { textContent: "Xuất .excalidraw" }),
          ),
          renderStrategyDropdown(st),
          renderUncertaintyDropdown(st),
          overflowMenu("Thao tác khác cho sơ đồ", overflowItems),
        ),
      );

      const svg = parseRustSvg(rendered.svg, zoom, rendered.width, rendered.height);

      const badgeParts = [
        diagram.provider || picked?.name || "AI",
        diagram.model || picked?.model || "",
        diagram.veElements && diagram.veElements.length > 0
          ? `${diagram.veElements.length} phần tử`
          : `${diagram.nodes.length} bước`,
      ].filter(Boolean);

      const metaBar = el(
        "div",
        { className: "diagram-toolbar" },
        el("span", { className: "diagram-model-tag", textContent: badgeParts.join(" · ") }),
      );

      const stageControls = el(
        "div",
        { className: "diagram-stage-controls" },
        el("button", {
          className: "btn-ghost diagram-zoom-btn",
          title: "Thu nhỏ",
          ariaLabel: "Thu nhỏ sơ đồ",
          textContent: "−",
          onclick: () => {
            zoom = Math.max(0.35, Math.round((zoom - 0.15) * 100) / 100);
            draw();
          },
        }),
        el("span", { className: "diagram-zoom-val", textContent: `${Math.round(zoom * 100)}%` }),
        el("button", {
          className: "btn-ghost diagram-zoom-btn",
          title: "Phóng to",
          ariaLabel: "Phóng to sơ đồ",
          textContent: "+",
          onclick: () => {
            zoom = Math.min(2.5, Math.round((zoom + 0.15) * 100) / 100);
            draw();
          },
        }),
        el("span", { className: "diagram-stage-sep" }),
        el(
          "button",
          {
            className: "btn-ghost diagram-zoom-btn",
            title: "Xem toàn màn hình",
            ariaLabel: "Xem sơ đồ toàn màn hình",
            onclick: () => openModal(diagram, rendered),
          },
          icon("expand", 14),
        ),
      );

      const stageWrap = el(
        "div",
        { className: "diagram-stage-wrap" },
        stageControls,
        el("div", { className: "diagram-stage" }, svg),
      );

      const uncertaintiesSection =
        diagram.uncertainties.length > 0
          ? el(
              "section",
              { className: "diagram-uncertainties" },
              el("h3", { textContent: `Cần xác nhận (${diagram.uncertainties.length})` }),
              el(
                "ul",
                { className: "diagram-uncertainty-list" },
                ...diagram.uncertainties.map((u) =>
                  el(
                    "li",
                    {},
                    el("strong", { textContent: u.item || "Chi tiết chưa rõ" }),
                    u.reason ? el("span", { textContent: ` — ${u.reason}` }) : null,
                  ),
                ),
              ),
            )
          : null;

      const children = [
        renderNoticeBanner(st),
        diagram.summary ? el("p", { className: "diagram-summary", textContent: diagram.summary }) : null,
        metaBar,
        stageWrap,
        uncertaintiesSection,
      ].filter((node): node is HTMLElement => node !== null);
      scroll.replaceChildren(...children);
      return;
    }

    // No diagram yet: show progress, missing-key prompt, or error directly without an extra confirmation step.
    head.replaceChildren(
      el("div", { className: "reader-head-top" }, el("h2", { textContent: `Sơ đồ — ${name}` })),
      el(
        "div",
        { className: "reader-actions" },
        renderStrategyDropdown(st),
        el(
          "button",
          {
            className: "btn-primary",
            disabled: st.running || !hasTranscriptText,
            onclick: () => triggerGenerate(),
          },
          icon("diagram", 15),
          el("span", { textContent: st.running ? "Đang tạo sơ đồ…" : "Tạo sơ đồ" }),
        ),
      ),
    );

    const missingKey = !hasAnyAiKey();
    const emptyChildren = [
      loadError ? el("p", { className: "status-inline bad", textContent: loadError }) : null,
      renderNoticeBanner(st),
      missingKey && !st.error
        ? el(
            "div",
            { className: "diagram-banner bad", role: "alert" },
            el("span", {
              textContent: "Chưa cấu hình API key cho nhà cung cấp AI nào. Mở Cài đặt để nhập key.",
            }),
            el("button", {
              className: "link",
              textContent: "Mở Cài đặt",
              onclick: links.openSettings,
            }),
          )
        : null,
      !hasTranscriptText
        ? el("p", {
            className: "status-inline bad",
            textContent: "Bản chép lời đang trống nên chưa thể tạo sơ đồ.",
          })
        : null,
    ].filter((node): node is HTMLElement => node !== null);

    scroll.replaceChildren(...emptyChildren);
  };

  const unsubscribe = subscribe(id, draw);
  const observer = new MutationObserver(() => {
    if (!pane.isConnected) {
      unsubscribe();
      observer.disconnect();
    }
  });
  queueMicrotask(() => {
    if (pane.parentNode) observer.observe(pane.parentNode, { childList: true });
  });

  draw();

  void (async () => {
    try {
      const [settings, saved] = await Promise.all([
        invoke<AiView>("ai_settings"),
        invoke<ProcessDiagram | null>("load_diagram", { id }).catch((e) => {
          loadError = String(e);
          return null;
        }),
      ]);
      ai = settings;
      const st = getState(id);
      if (!st.diagram && saved) {
        try {
          st.rendered = await invoke<RenderedDiagram>("render_diagram", { diagram: saved });
          st.diagram = saved;
          if (saved.strategy) {
            st.selectedStrategy = findStrategyOption(saved.strategy).id;
          }
          if (saved.uncertaintyMode) {
            st.uncertaintyMode = findUncertaintyModeOption(saved.uncertaintyMode).id;
          }
        } catch (e) {
          loadError = `Sơ đồ đã lưu không hợp lệ: ${e instanceof Error ? e.message : String(e)}`;
        }
      }
      loadedLocal = true;
      draw();
      // Auto-start generation when opening Tạo sơ đồ on a recording with no saved diagram yet.
      const picked = activeAi();
      if (!st.diagram && !st.running && !st.error && hasTranscriptText && picked) {
        void runGeneration(id, picked.name, picked.model);
      }
    } catch (e) {
      loadError = String(e);
      loadedLocal = true;
      draw();
    }
  })();

  return pane;
}
