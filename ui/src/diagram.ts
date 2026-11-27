import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import {
  buildExcalidrawFile,
  type LayoutDiagram,
  layoutDiagram,
  type ProcessDiagram,
  renderDiagramPng,
  renderDiagramSvg,
} from "./diagram-render";
import { el } from "./dom";
import { icon } from "./icons";
import { type AiView, overflowMenu, PROVIDER_NAME, PROVIDERS, type Provider } from "./ipc";

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
  layout: LayoutDiagram | null;
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
      layout: null,
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
  if (st.diagram && st.layout) return "ready";
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
  if ((st.diagram && st.layout) || st.error) {
    callbacks.onOpenDiagram(id);
    return;
  }

  // Check local saved diagram first (offline-safe, zero AI calls).
  try {
    const saved = await invoke<ProcessDiagram | null>("load_diagram", { id });
    if (saved) {
      st.layout = layoutDiagram(saved);
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
    let nextLayout: LayoutDiagram;
    try {
      nextLayout = layoutDiagram(nextDiagram);
      // Dry-run SVG construction before persisting so renderer errors never overwrite the saved diagram.
      renderDiagramSvg(nextLayout);
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
    st.layout = nextLayout;
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
  let zoom = 1;

  const activeAi = () => pickConfiguredProvider(ai);
  const hasAnyAiKey = () => activeAi() !== null;

  const exportFile = async (format: "png" | "excalidraw") => {
    const st = getState(id);
    if (!st.layout || !st.diagram) return;
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
        bytes = await renderDiagramPng(st.layout);
      } else {
        const json = buildExcalidrawFile(st.layout);
        bytes = new TextEncoder().encode(json);
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

    if (st.diagram && st.layout) {
      const layout = st.layout;
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

      const svg = renderDiagramSvg(layout);
      svg.setAttribute("width", String(Math.round(layout.width * zoom)));
      svg.setAttribute("height", String(Math.round(layout.height * zoom)));

      const badgeParts = [
        diagram.provider || picked?.name || "AI",
        diagram.model || picked?.model || "",
        `${diagram.nodes.length} bước`,
      ].filter(Boolean);

      const zoomBar = el(
        "div",
        { className: "diagram-toolbar" },
        el("span", { className: "diagram-model-tag", textContent: badgeParts.join(" · ") }),
        el(
          "div",
          { className: "diagram-zoom-group" },
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
              const availW = Math.max(320, (stage?.clientWidth ?? 760) - 44);
              const availH = Math.max(320, (stage?.clientHeight ?? 580) - 44);
              const fit = Math.min(1, availW / layout.width, availH / layout.height);
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
        ),
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
        zoomBar,
        el("div", { className: "diagram-stage" }, svg),
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
          st.layout = layoutDiagram(saved);
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
