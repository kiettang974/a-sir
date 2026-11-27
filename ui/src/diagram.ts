import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { el } from "./dom";
import { icon } from "./icons";
import { type AiView, overflowMenu, PROVIDER_NAME, PROVIDERS, type Provider } from "./ipc";

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
};

type JobDiagramState = {
  running: boolean;
  statusMessage: string;
  error: string | null;
  needsSettings: boolean;
  savedPath: string | null;
  diagram: ProcessDiagram | null;
  rendered: RenderedDiagram | null;
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
async function runGeneration(id: number, providerName: string, modelName: string) {
  const st = getState(id);
  if (st.running) return;
  const isRegenerate = st.diagram !== null;
  const tag = modelName ? `${providerName} (${modelName})` : providerName;
  st.running = true;
  st.error = null;
  st.needsSettings = false;
  st.exportError = null;
  st.exportNotice = null;
  st.statusMessage = isRegenerate
    ? `Đang tạo lại sơ đồ bằng ${tag}… Bản sơ đồ hiện tại vẫn có thể xem và xuất.`
    : `Đang phân tích bản chép lời và tạo sơ đồ bằng ${tag}…`;
  notify(id);

  try {
    const nextDiagram = await invoke<ProcessDiagram>("generate_diagram", { id });
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
  const scroll = el("div", { className: "diagram-scroll" });
  const pane = el("section", { className: "reader" }, head, scroll);

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

  const triggerGenerate = () => {
    if (!hasTranscriptText) return;
    const picked = activeAi();
    if (!picked) {
      const st = getState(id);
      st.error = "Chưa có API key cho nhà cung cấp AI nào. Hãy thiết lập trong Cài đặt trước khi tạo sơ đồ.";
      st.needsSettings = true;
      draw();
      return;
    }
    void runGeneration(id, picked.name, picked.model);
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
                onclick: triggerGenerate,
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
        el("h2", { textContent: `Sơ đồ quy trình — ${name}` }),
        el(
          "div",
          { className: "reader-actions" },
          el("button", { className: "link", textContent: "Về bản chép lời", onclick: links.back }),
        ),
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
          onClick: triggerGenerate,
        },
        ...(st.savedPath
          ? [
              {
                label: "Mở tệp đã lưu trong Finder",
                onClick: () => void revealItemInDir(st.savedPath!),
              },
            ]
          : []),
        {
          label: "Về bản chép lời",
          onClick: links.back,
        },
      ];

      head.replaceChildren(
        el("h2", { textContent: diagram.title || name }),
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
          el(
            "button",
            {
              className: "btn-ghost",
              title: "Mở bảng tóm tắt biên bản",
              onclick: links.openSummary,
            },
            icon("summary", 15),
            el("span", { textContent: "Tóm tắt" }),
          ),
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
        el("button", {
          className: "link",
          textContent: "Vừa khung",
          title: "Thu phóng để xem trọn sơ đồ trong khung",
          onclick: () => {
            const stage = scroll.querySelector<HTMLElement>(".diagram-stage");
            const availW = Math.max(300, (stage?.clientWidth ?? 720) - 36);
            const availH = Math.max(220, (stage?.clientHeight ?? 360) - 66);
            const fit = Math.min(1, availW / rendered.width, availH / rendered.height);
            zoom = Math.max(0.35, Math.round(fit * 100) / 100);
            draw();
          },
        }),
        el("button", {
          className: "link",
          textContent: "100%",
          onclick: () => {
            zoom = 1;
            draw();
          },
        }),
        el("span", { className: "diagram-stage-sep" }),
        el("button", {
          className: "btn-ghost diagram-zoom-btn",
          textContent: "Xem to",
          title: "Mở cửa sổ xem sơ đồ phóng to",
          onclick: () => openModal(diagram, rendered),
        }),
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
      el("h2", { textContent: `Sơ đồ quy trình — ${name}` }),
      el(
        "div",
        { className: "reader-actions" },
        el(
          "button",
          {
            className: "btn-primary",
            title: "Mở bảng tóm tắt biên bản",
            onclick: links.openSummary,
          },
          icon("summary", 15),
          el("span", { textContent: "Tóm tắt" }),
        ),
        el("button", { className: "link", textContent: "Về bản chép lời", onclick: links.back }),
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
