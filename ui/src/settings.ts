import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask, open, save } from "@tauri-apps/plugin-dialog";

import { el } from "./dom";
import { icon } from "./icons";
import {
  type AiView,
  type LocalModelProgress,
  type MeetingTemplates,
  notifyModelLabelAdded,
  pickAndImportTemplate,
  PROVIDER_NAME,
  PROVIDERS,
  type Provider,
  progressBar,
  resetTemplate,
  setTemplatesEnabled,
  type SummaryStyle,
  type SummaryStyleOption,
} from "./ipc";

const FALLBACK_STYLES: SummaryStyleOption[] = [
  {
    id: "workEmail",
    label: "Email công việc",
    description:
      "Lịch sự, trực tiếp. Dùng câu ngắn và nêu rõ quyết định, việc cần làm, người phụ trách và thời hạn nếu có căn cứ.",
  },
];

function persistAiSettings(ai: AiView): Promise<unknown> {
  return invoke("save_ai_settings", {
    settings: {
      defaultProvider: ai.defaultProvider,
      models: ai.models,
      summaryStyle: ai.summaryStyle ?? "workEmail",
    },
  });
}

type Term = { correct: string; heardAs: string; note: string };

type OsId = "mac" | "windows" | "linux";
const OS_LABEL: Record<OsId, string> = { mac: "macOS", windows: "Windows", linux: "Linux" };

/** Small OS icons showing where a model runs; a dimmed icon means it does not. */
function platformChips(platforms?: string[]): HTMLElement {
  const supported = new Set(platforms ?? ["mac", "windows", "linux"]);
  return el(
    "span",
    { className: "os-chips", title: "Hệ điều hành chạy được model này" },
    ...(["mac", "windows", "linux"] as OsId[]).map((os) => {
      const ok = supported.has(os);
      const chip = el(
        "span",
        {
          className: `os-chip${ok ? "" : " off"}`,
          title: ok ? `Chạy được trên ${OS_LABEL[os]}` : `Chưa hỗ trợ ${OS_LABEL[os]}`,
        },
        icon(os, 14),
      );
      chip.setAttribute("aria-label", chip.title);
      return chip;
    }),
  );
}

type LocalModelView = {
  id: string;
  name: string;
  description: string;
  source: string;
  engine: string;
  bytes: number;
  ready: boolean;
  selected: boolean;
  custom?: boolean;
  platforms?: string[];
};

type DiscoveredModel = {
  id: string;
  name: string;
  repo: string;
  file: string;
  url: string;
  sha256: string;
  bytes: number;
  downloads: number;
  alreadyAdded: boolean;
};

type ResourceProfile = "eco" | "balanced" | "max";

const RESOURCE_PROFILES: { id: ResourceProfile; label: string; description: string }[] = [
  {
    id: "eco",
    label: "Nhẹ máy (chạy nền)",
    description: "Chạy êm, giữ máy mát khi bạn đang làm việc khác.",
  },
  {
    id: "balanced",
    label: "Cân bằng",
    description: "Vừa nhanh vừa mượt, phù hợp dùng hằng ngày.",
  },
  {
    id: "max",
    label: "Nhanh nhất",
    description: "Dồn tối đa sức mạnh của máy để chép lời nhanh nhất.",
  },
];

type LocalModelsView = {
  selected: string;
  resourceProfile: ResourceProfile;
  models: LocalModelView[];
};

type SummaryProfile = {
  id: string;
  name: string;
  description: string;
  content: string;
  sourceFile?: string | null;
};

type ProfilesConfig = {
  enabled: boolean;
  activeId: string;
  custom: SummaryProfile[];
};

type UpdateInfo = {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  releaseUrl: string;
  downloadUrl?: string | null;
  notes: string;
};

type TabId = "speech" | "ai" | "minutes" | "glossary" | "update";

const TABS: { id: TabId; label: string }[] = [
  { id: "speech", label: "Chép lời" },
  { id: "ai", label: "Tài khoản AI" },
  { id: "minutes", label: "Biên bản" },
  { id: "glossary", label: "Từ vựng riêng" },
  { id: "update", label: "Cập nhật" },
];

type LoadState<T> =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; data: T }
  | { kind: "error"; message: string };

type ProviderCache = {
  models: string[] | null;
  accessible: string[] | null;
  generation: number;
};

// Session cache for provider model lists; invalidated when a provider's key changes or is deleted.
const providerModelCache: Record<Provider, ProviderCache> = {
  claude: { models: null, accessible: null, generation: 0 },
  chatGpt: { models: null, accessible: null, generation: 0 },
  gemini: { models: null, accessible: null, generation: 0 },
};

// Module-level local-model download state so switching away from Settings and coming back
// preserves active download progress and never starts duplicate downloads.
const localDownloading = new Map<string, number>();
const localDownloadErrors = new Map<string, string>();
let lastActiveTab: TabId = "speech";
let hfQueryText = "";
let hfStatusText = "";
let hfStatusBad = false;
let hfFound: DiscoveredModel[] = [];
let onLocalModelChange: ((reload: boolean) => void) | null = null;
let progressListenerStarted = false;

function ensureLocalProgressListener() {
  if (progressListenerStarted) return;
  progressListenerStarted = true;
  listen<LocalModelProgress>("local-model-progress", ({ payload }) => {
    if (localDownloading.get(payload.modelId) !== payload.percent) {
      localDownloading.set(payload.modelId, payload.percent);
      onLocalModelChange?.(false);
    }
  }).catch(() => {
    progressListenerStarted = false;
  });
}

async function startLocalModelDownload(modelId: string) {
  if (localDownloading.has(modelId)) return;
  ensureLocalProgressListener();
  localDownloadErrors.delete(modelId);
  localDownloading.set(modelId, 0);
  onLocalModelChange?.(false);
  try {
    await invoke("download_local_model", { modelId });
    localDownloading.delete(modelId);
    onLocalModelChange?.(true);
  } catch (e) {
    localDownloading.delete(modelId);
    localDownloadErrors.set(modelId, String(e));
    onLocalModelChange?.(false);
  }
}

function invalidateProviderCache(provider: Provider) {
  providerModelCache[provider].models = null;
  providerModelCache[provider].accessible = null;
  providerModelCache[provider].generation += 1;
}

function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  return `${Math.round(bytes / 1_000_000)} MB`;
}

/** Fills itself in and keeps its own tab & data state, so the rest of the app can leave it alone. */
export function settingsPane(): HTMLElement {
  let activeTab: TabId = lastActiveTab;
  let speechState: LoadState<LocalModelsView> = { kind: "idle" };
  let aiState: LoadState<AiView> = { kind: "idle" };
  let glossaryState: LoadState<Term[]> = { kind: "idle" };

  const tablist = el("div", {
    className: "settings-tabs",
    role: "tablist",
    ariaLabel: "Nhóm cấu hình",
  });
  const panel = el("div", {
    className: "settings",
    role: "tabpanel",
    tabIndex: 0,
  });

  const section = el(
    "section",
    { className: "reader" },
    el("header", { className: "reader-head" }, el("h2", { textContent: "Cấu hình" })),
    tablist,
    panel,
  );

  ensureLocalProgressListener();
  onLocalModelChange = (reload: boolean) => {
    if (!section.isConnected) {
      onLocalModelChange = null;
      return;
    }
    if (reload) {
      loadSpeech();
    } else if (activeTab === "speech") {
      renderPanel();
    }
  };

  const loadSpeech = async () => {
    speechState = { kind: "loading" };
    if (activeTab === "speech") renderPanel();
    try {
      const data = await invoke<LocalModelsView>("local_models");
      speechState = { kind: "ready", data };
    } catch (e) {
      speechState = { kind: "error", message: String(e) };
    }
    if (activeTab === "speech") renderPanel();
  };

  const loadAi = async () => {
    aiState = { kind: "loading" };
    if (activeTab === "ai" || activeTab === "minutes") renderPanel();
    try {
      const data = await invoke<AiView>("ai_settings");
      aiState = { kind: "ready", data };
    } catch (e) {
      aiState = { kind: "error", message: String(e) };
    }
    if (activeTab === "ai" || activeTab === "minutes") renderPanel();
  };

  const loadGlossary = async () => {
    glossaryState = { kind: "loading" };
    if (activeTab === "glossary") renderPanel();
    try {
      const data = await invoke<Term[]>("get_glossary");
      glossaryState = { kind: "ready", data };
    } catch (e) {
      glossaryState = { kind: "error", message: String(e) };
    }
    if (activeTab === "glossary") renderPanel();
  };

  const ensureLoaded = (tab: TabId) => {
    if (tab === "speech" && speechState.kind === "idle") loadSpeech();
    if ((tab === "ai" || tab === "minutes") && aiState.kind === "idle") loadAi();
    if (tab === "glossary" && glossaryState.kind === "idle") loadGlossary();
  };

  const selectTab = (tab: TabId, focusTabButton = false) => {
    activeTab = tab;
    lastActiveTab = tab;
    ensureLoaded(tab);
    renderTabs();
    renderPanel();
    if (focusTabButton) {
      tablist.querySelector<HTMLButtonElement>(`#settings-tab-${tab}`)?.focus();
    }
  };

  const renderTabs = () => {
    tablist.replaceChildren(
      ...TABS.map((t, index) => {
        const selected = t.id === activeTab;
        const btn = el("button", {
          id: `settings-tab-${t.id}`,
          className: `settings-tab${selected ? " active" : ""}`,
          role: "tab",
          ariaSelected: String(selected),
          tabIndex: selected ? 0 : -1,
          textContent: t.label,
          onclick: () => selectTab(t.id),
          onkeydown: (e: KeyboardEvent) => {
            if (e.key === "ArrowRight") {
              e.preventDefault();
              selectTab(TABS[(index + 1) % TABS.length].id, true);
            } else if (e.key === "ArrowLeft") {
              e.preventDefault();
              selectTab(TABS[(index - 1 + TABS.length) % TABS.length].id, true);
            } else if (e.key === "Home") {
              e.preventDefault();
              selectTab(TABS[0].id, true);
            } else if (e.key === "End") {
              e.preventDefault();
              selectTab(TABS[TABS.length - 1].id, true);
            }
          },
        });
        btn.setAttribute("aria-controls", "settings-panel");
        return btn;
      }),
    );
    panel.id = "settings-panel";
    panel.setAttribute("aria-labelledby", `settings-tab-${activeTab}`);
  };

  const errorBlock = (title: string, message: string, retry: () => void) =>
    el(
      "section",
      {},
      el("h3", { textContent: title }),
      el("p", { className: "problem problem-text", textContent: message }),
      el("button", { className: "add", textContent: "Thử lại", onclick: retry }),
    );

  const renderPanel = () => {
    if (activeTab === "speech") {
      if (speechState.kind === "loading" || speechState.kind === "idle") {
        panel.replaceChildren(el("p", { className: "hint", textContent: "Đang đọc danh sách model chép lời…" }));
      } else if (speechState.kind === "error") {
        panel.replaceChildren(errorBlock("Model chép lời cục bộ", speechState.message, loadSpeech));
      } else {
        panel.replaceChildren(
          speechSection(
            speechState.data,
            localDownloading,
            localDownloadErrors,
            (modelId) => {
              startLocalModelDownload(modelId);
            },
            async (modelId) => {
              localDownloadErrors.delete(modelId);
              try {
                const updated = await invoke<LocalModelsView>("select_local_model", { modelId });
                speechState = { kind: "ready", data: updated };
                if (activeTab === "speech") renderPanel();
              } catch (e) {
                localDownloadErrors.set(modelId, String(e));
                if (activeTab === "speech") renderPanel();
              }
            },
            async (model) => {
              const yes = await ask(
                `Xoá file model "${model.name}" (${formatBytes(model.bytes)}) khỏi máy? Bạn có thể tải lại bất cứ lúc nào.`,
                { title: "Xoá model khỏi máy?", kind: "warning", okLabel: "Xoá", cancelLabel: "Giữ lại" },
              );
              if (!yes) return;
              localDownloadErrors.delete(model.id);
              try {
                const updated = await invoke<LocalModelsView>("delete_local_model", { modelId: model.id });
                speechState = { kind: "ready", data: updated };
                if (activeTab === "speech") renderPanel();
              } catch (e) {
                localDownloadErrors.set(model.id, String(e));
                if (activeTab === "speech") renderPanel();
              }
            },
            async (profile) => {
              try {
                const updated = await invoke<LocalModelsView>("set_resource_profile", { profile });
                speechState = { kind: "ready", data: updated };
                if (activeTab === "speech") renderPanel();
              } catch (e) {
                localDownloadErrors.set("__resource__", String(e));
                if (activeTab === "speech") renderPanel();
              }
            },
            (updated) => {
              speechState = { kind: "ready", data: updated };
              if (activeTab === "speech") renderPanel();
            },
          ),
        );
      }
      return;
    }

    if (activeTab === "ai") {
      if (aiState.kind === "loading" || aiState.kind === "idle") {
        panel.replaceChildren(el("p", { className: "hint", textContent: "Đang đọc cấu hình nhà cung cấp AI…" }));
      } else if (aiState.kind === "error") {
        panel.replaceChildren(errorBlock("Nhà cung cấp AI", aiState.message, loadAi));
      } else {
        panel.replaceChildren(aiSection(aiState.data, loadAi));
      }
      return;
    }

    if (activeTab === "minutes") {
      if (aiState.kind === "loading" || aiState.kind === "idle") {
        panel.replaceChildren(el("p", { className: "hint", textContent: "Đang đọc cấu hình biên bản…" }));
      } else if (aiState.kind === "error") {
        panel.replaceChildren(errorBlock("Cấu hình biên bản", aiState.message, loadAi));
      } else {
        panel.replaceChildren(minutesSection(aiState.data));
      }
      return;
    }

    if (activeTab === "update") {
      panel.replaceChildren(updateSection());
      return;
    }

    if (glossaryState.kind === "loading" || glossaryState.kind === "idle") {
      panel.replaceChildren(el("p", { className: "hint", textContent: "Đang đọc danh sách thuật ngữ…" }));
    } else if (glossaryState.kind === "error") {
      panel.replaceChildren(errorBlock("Thuật ngữ", glossaryState.message, loadGlossary));
    } else {
      panel.replaceChildren(glossarySection(glossaryState.data));
    }
  };

  renderTabs();
  ensureLoaded(activeTab);
  renderPanel();
  return section;
}

function speechSection(
  data: LocalModelsView,
  downloading: Map<string, number>,
  errors: Map<string, string>,
  onDownload: (modelId: string) => void,
  onSelect: (modelId: string) => void,
  onDelete: (model: LocalModelView) => void,
  onResourceChange: (profile: ResourceProfile) => void,
  onCatalogUpdated: (updated: LocalModelsView) => void,
): HTMLElement {
  const currentProfile =
    RESOURCE_PROFILES.find((p) => p.id === data.resourceProfile) ?? RESOURCE_PROFILES[1];
  const resourceErr = errors.get("__resource__");
  const resourceSelect = el(
    "select",
    {
      className: "field",
      ariaLabel: "Mức tài nguyên chép lời",
      onchange: () => onResourceChange(resourceSelect.value as ResourceProfile),
    },
    ...RESOURCE_PROFILES.map((p) =>
      el("option", {
        value: p.id,
        selected: p.id === currentProfile.id,
        textContent: p.label,
      }),
    ),
  );
  const resourceBox = el(
    "div",
    { className: "style-box" },
    el("div", { className: "style-title", textContent: "Tốc độ xử lý" }),
    el(
      "div",
      { className: "style-row" },
      resourceSelect,
      el("p", {
        className: "style-desc",
        textContent: `${currentProfile.description} Có hiệu lực từ lượt chép lời tiếp theo.`,
      }),
    ),
    resourceErr
      ? el("p", { className: "provider-status status-inline bad", textContent: resourceErr })
      : null,
  );

  const hfQuery = el("input", {
    className: "field",
    type: "search",
    value: hfQueryText,
    placeholder: "Gõ tên model muốn tìm (VD: phowhisper, whisper, vietnamese)…",
    ariaLabel: "Tìm model chép lời mới trên Hugging Face",
    oninput: (e: Event) => {
      hfQueryText = (e.target as HTMLInputElement).value;
    },
  });
  const hfStatus = el("p", {
    className: hfStatusBad ? "provider-status status-inline bad" : "provider-status status-inline",
    textContent: hfStatusText,
  });
  const hfResults = el("div", { className: "hf-results" });
  const drawHfResults = () => {
    const existingIds = new Set(data.models.map((m) => m.id));
    hfResults.replaceChildren(
      ...hfFound.map((m) =>
        el(
          "div",
          { className: "hf-row" },
          el(
            "div",
            { className: "hf-row-info" },
            el("strong", { textContent: m.file }),
            el("span", {
              className: "hf-row-meta",
              textContent: `${m.repo} · ${formatBytes(m.bytes)}`,
            }),
          ),
          m.alreadyAdded || existingIds.has(m.id)
            ? el("span", { className: "model-badge", textContent: "Đã có" })
            : el("button", {
                className: "link",
                textContent: "Thêm vào danh sách",
                onclick: async () => {
                  try {
                    const updated = await invoke<LocalModelsView>("add_custom_model", {
                      model: {
                        id: m.id,
                        name: m.name,
                        description: `Model cộng đồng từ ${m.repo}.`,
                        repo: m.repo,
                        file: m.file,
                        url: m.url,
                        sha256: m.sha256,
                        bytes: m.bytes,
                      },
                    });
                    m.alreadyAdded = true;
                    notifyModelLabelAdded(m.id, m.name);
                    onCatalogUpdated(updated);
                  } catch (e) {
                    hfStatusText = String(e);
                    hfStatusBad = true;
                    hfStatus.textContent = hfStatusText;
                    hfStatus.className = "provider-status status-inline bad";
                  }
                },
              }),
        ),
      ),
    );
  };
  drawHfResults();
  const runDiscover = async () => {
    hfBtn.disabled = true;
    hfQueryText = hfQuery.value;
    hfStatusBad = false;
    hfStatusText = "Đang tìm model GGML mới trên Hugging Face…";
    hfStatus.className = "provider-status status-inline";
    hfStatus.textContent = hfStatusText;
    hfFound = [];
    hfResults.replaceChildren();
    try {
      const found = await invoke<DiscoveredModel[]>("discover_hf_models", { query: hfQueryText });
      hfFound = found;
      if (found.length === 0) {
        hfStatusText = "Không tìm thấy file model GGML (.bin) phù hợp với từ khoá này.";
        hfStatus.textContent = hfStatusText;
        return;
      }
      hfStatusText = `Tìm thấy ${found.length} model trên Hugging Face.`;
      hfStatus.textContent = hfStatusText;
      drawHfResults();
    } catch (e) {
      hfStatusText = String(e);
      hfStatusBad = true;
      hfStatus.textContent = hfStatusText;
      hfStatus.className = "provider-status status-inline bad";
    } finally {
      hfBtn.disabled = false;
    }
  };
  const hfBtn = el("button", {
    className: "btn-ghost",
    textContent: "Tìm trên Hugging Face",
    onclick: runDiscover,
  });
  hfQuery.addEventListener("keydown", (e) => {
    if (e.key === "Enter") runDiscover();
  });

  const hfBox = el(
    "div",
    { className: "style-box" },
    el(
      "div",
      { className: "hf-head" },
      el("div", { className: "style-title tpl-label", textContent: "Tìm thêm model mới" }),
      el("button", {
        className: "link",
        textContent: "Xem thư viện model",
        onclick: () => invoke("open_hf_asr_hub"),
      }),
    ),
    el("div", { className: "hf-bar" }, hfQuery, hfBtn),
    hfStatus,
    hfResults,
  );

  return el(
    "section",
    {},
    el(
      "div",
      { className: "hf-head" },
      el("h3", { textContent: "Giọng nói & tốc độ chép lời" }),
      el("button", {
        className: "link",
        textContent: "Mở thư mục model",
        title: "Mở thư mục lưu các file model đã tải trên máy trong Finder",
        onclick: () => invoke("open_models_dir"),
      }),
    ),
    el("p", {
      className: "hint",
      textContent:
        "Mọi model đều chạy thẳng trên máy. Đổi model sẽ áp dụng cho bản ghi mới; bản đang chạy dở vẫn tiếp tục đến xong.",
    }),
    resourceBox,
    hfBox,
    el(
      "div",
      { className: "model-list" },
      ...data.models.map((m) => {
        const progress = downloading.get(m.id);
        const error = errors.get(m.id);
        const statusLabel = m.selected
          ? "Đang dùng"
          : progress !== undefined
            ? `Đang tải ${progress}%`
            : m.ready
              ? "Sẵn sàng"
              : "Chưa tải";

        const action =
          progress !== undefined
            ? el("span", { className: "model-status-text", textContent: `Đang tải: ${progress}%` })
            : m.selected
              ? el("span", { className: "model-badge active", textContent: "Đang dùng" })
              : m.ready
                ? el(
                    "div",
                    { className: "actions" },
                    el("button", {
                      className: "add",
                      textContent: "Chọn dùng",
                      onclick: () => onSelect(m.id),
                    }),
                    el("button", {
                      className: "link danger-link",
                      textContent: "Xoá",
                      onclick: () => onDelete(m),
                    }),
                  )
                : el("button", {
                    className: "link",
                    textContent: error ? "Thử tải lại" : "Tải model",
                    onclick: () => onDownload(m.id),
                  });

        return el(
          "div",
          { className: `model-card${m.selected ? " selected" : ""}` },
          el(
            "div",
            { className: "model-card-head" },
            el(
              "div",
              { className: "model-card-title" },
              el("strong", { textContent: m.name }),
              el("span", {
                className: `model-badge${m.selected ? " active" : m.ready ? " ready" : ""}`,
                textContent: statusLabel,
              }),
            ),
            action,
          ),
          el("p", { className: "model-desc", textContent: m.description }),
          el(
            "div",
            { className: "model-meta" },
            el("span", { textContent: `Dung lượng: ${formatBytes(m.bytes)}` }),
            platformChips(m.platforms),
          ),
          progress !== undefined ? progressBar(progress) : null,
          error ? el("p", { className: "provider-status status-inline bad", textContent: error }) : null,
        );
      }),
    ),
  );
}

function aiSection(ai: AiView, reload: () => void): HTMLElement {
  const persist = () => persistAiSettings(ai);
  return el(
    "section",
    {},
    el("h3", { textContent: "Tài khoản AI tóm tắt" }),
    el("p", {
      className: "hint",
      textContent:
        "Bản chép lời chỉ được gửi đi khi bạn bấm tạo tóm tắt. Mã API key được lưu riêng trên máy của bạn.",
    }),
    ...PROVIDERS.map((p) => providerBlock(p, ai, persist, reload)),
  );
}

function minutesSection(ai: AiView): HTMLElement {
  const styles = ai.summaryStyles?.length ? ai.summaryStyles : FALLBACK_STYLES;
  const currentStyle = (): SummaryStyle =>
    styles.some((s) => s.id === ai.summaryStyle) ? (ai.summaryStyle as SummaryStyle) : styles[0].id;
  ai.summaryStyle = currentStyle();

  const styleMeta = (id: SummaryStyle) =>
    styles.find((s) => s.id === id) ?? styles[0];
  const styleDesc = el("p", {
    className: "style-desc",
    textContent: styleMeta(ai.summaryStyle).description,
  });
  const stylePreview = el("div", { className: "style-preview" });
  const drawStylePreview = (id: SummaryStyle) => {
    const meta = styleMeta(id);
    if (!meta.before || !meta.after) {
      stylePreview.replaceChildren();
      return;
    }
    stylePreview.replaceChildren(
      el(
        "div",
        { className: "style-preview-col" },
        el("div", { className: "style-preview-tag", textContent: "Trước (cách máy hay viết)" }),
        el("p", { className: "style-preview-before", textContent: meta.before }),
      ),
      el(
        "div",
        { className: "style-preview-col after" },
        el("div", { className: "style-preview-tag after", textContent: `Sau (${meta.label})` }),
        el("p", { className: "style-preview-after", textContent: meta.after }),
      ),
    );
  };
  drawStylePreview(ai.summaryStyle);

  const styleStatus = el("p", { className: "provider-status status-inline" });
  const styleSelect = el(
    "select",
    {
      className: "field style-select",
      ariaLabel: "Văn phong tóm tắt",
      onchange: async () => {
        const prev = ai.summaryStyle ?? "workEmail";
        const next = styleSelect.value as SummaryStyle;
        ai.summaryStyle = next;
        styleDesc.textContent = styleMeta(next).description;
        drawStylePreview(next);
        styleStatus.textContent = "";
        styleStatus.className = "provider-status status-inline";
        try {
          await persistAiSettings(ai);
          styleStatus.textContent = `Đã lưu văn phong: ${styleMeta(next).label}. Áp dụng cho lần tạo tóm tắt tiếp theo.`;
        } catch (e) {
          ai.summaryStyle = prev;
          styleSelect.value = prev;
          styleDesc.textContent = styleMeta(prev).description;
          drawStylePreview(prev);
          styleStatus.textContent = `Chưa lưu được văn phong: ${e}`;
          styleStatus.className = "provider-status status-inline bad";
        }
      },
    },
    ...styles.map((s) =>
      el("option", {
        value: s.id,
        selected: s.id === ai.summaryStyle,
        textContent: s.label,
      }),
    ),
  );

  const styleBlock = el(
    "div",
    { className: "style-box" },
    el("div", { className: "style-title", textContent: "Văn phong tóm tắt" }),
    el("div", { className: "style-stack" }, styleSelect, styleDesc, stylePreview),
    styleStatus,
  );

  // Meeting export template configuration (Word .docx & Excel .xlsx, off by default).
  const tplSwitch = el("input", {
    type: "checkbox",
    className: "switch-input",
    ariaLabel: "Bật dùng template xuất biên bản",
  }) as HTMLInputElement;
  const tplSwitchText = el("span", { className: "switch-text", textContent: "Đang tắt" });
  const tplToggle = el(
    "label",
    { className: "switch" },
    tplSwitch,
    el("span", { className: "switch-track" }),
    tplSwitchText,
  );
  const tplLabel = el("p", {
    className: "style-desc tpl-label",
    textContent: "Đang kiểm tra template biên bản…",
  });
  const tplCols = el("p", { className: "style-desc" });
  const tplPreview = el("div", { className: "tpl-preview" });
  const tplStatus = el("p", { className: "provider-status status-inline" });
  const tplActions = el("div", { className: "actions wrap tpl-actions" });

  tplSwitch.onchange = async () => {
    const next = tplSwitch.checked;
    tplSwitch.disabled = true;
    try {
      const updated = await setTemplatesEnabled(next);
      tplStatus.textContent = next
        ? "Đã bật chế độ dùng template xuất biên bản."
        : "Đã tắt template cố định — quay về thuần BA-Open-Kit + Humanizer.";
      tplStatus.className = "provider-status status-inline";
      drawTemplateBox(updated);
    } catch (e) {
      tplSwitch.checked = !next;
      tplStatus.textContent = String(e);
      tplStatus.className = "provider-status status-inline bad";
    } finally {
      tplSwitch.disabled = false;
    }
  };

  const buildPreviewTable = (title: string, columns: string[], sampleRows: string[][]): HTMLElement =>
    el(
      "div",
      { className: "tpl-preview-block" },
      el("div", { className: "tpl-preview-label", textContent: title }),
      el(
        "div",
        { className: "tpl-preview-scroll" },
        el(
          "table",
          { className: "tpl-preview-table" },
          el(
            "thead",
            {},
            el("tr", {}, ...columns.map((c) => el("th", { textContent: c }))),
          ),
          el(
            "tbody",
            {},
            ...sampleRows.map((row) =>
              el("tr", {}, ...row.map((cell) => el("td", { textContent: cell }))),
            ),
          ),
        ),
      ),
    );

  const defaultTemplateCols = ["#", "Hạng mục", "Nội dung thống nhất", "Ghi chú"];
  const defaultTemplateSamples = [
    [
      "1",
      "Chốt phạm vi phát hành",
      "Giữ luồng chép lời offline và thêm xuất biên bản Word/Excel.",
      "Phụ trách: Nhóm sản phẩm · Hạn: Thứ 6",
    ],
    [
      "2",
      "Kiểm thử file âm thanh dài",
      "Chạy thử bản ghi 60 phút để kiểm tra độ ổn định và thời gian xử lý.",
      "Cần xác nhận lại lịch test",
    ],
  ];

  const renderTemplatePreview = (tpl: MeetingTemplates) => {
    const blocks: HTMLElement[] = [];
    const enabled = Boolean(tpl.enabled);

    if (!enabled) {
      blocks.push(
        el(
          "div",
          { className: "tpl-preview-doc" },
          el("div", {
            className: "tpl-preview-title",
            textContent: "Đang tắt — tự tóm tắt theo bố cục biên bản chuẩn",
          }),
          el("div", {
            className: "tpl-preview-meta",
            textContent:
              "App tự chia 4 phần rõ ràng: Ý trao đổi chính · Việc đã chốt · Việc cần làm (ai làm, hạn khi nào) · Điểm cần hỏi lại, viết theo đúng văn phong bạn chọn ở trên.",
          }),
        ),
      );
    } else if (!tpl.excel && !tpl.docx) {
      blocks.push(
        buildPreviewTable(
          "Mẫu mặc định đang áp dụng (Word & Excel)",
          defaultTemplateCols,
          defaultTemplateSamples,
        ),
      );
    } else {
      if (tpl.excel) {
        const cols = tpl.excelColumns?.length ? tpl.excelColumns : defaultTemplateCols;
        const sample = cols.map((_, idx) => (idx === 0 ? "1" : "Nội dung AI điền khớp cột…"));
        blocks.push(buildPreviewTable(`Template Excel đang bật: ${tpl.excel}`, cols, [sample]));
      } else {
        blocks.push(
          buildPreviewTable(
            "Mẫu mặc định cho Excel (.xlsx)",
            defaultTemplateCols,
            defaultTemplateSamples.slice(0, 1),
          ),
        );
      }
      if (tpl.docx) {
        const cols = tpl.docxColumns?.length ? tpl.docxColumns : defaultTemplateCols;
        const sample = cols.map((_, idx) => (idx === 0 ? "1" : "Nội dung AI điền khớp cột…"));
        blocks.push(buildPreviewTable(`Template Word đang bật: ${tpl.docx}`, cols, [sample]));
      } else {
        blocks.push(
          buildPreviewTable(
            "Mẫu mặc định cho Word (.docx)",
            defaultTemplateCols,
            defaultTemplateSamples.slice(0, 1),
          ),
        );
      }
    }
    tplPreview.replaceChildren(...blocks);
  };

  const drawTemplateBox = (tpl: MeetingTemplates) => {
    const enabled = Boolean(tpl.enabled);
    tplSwitch.checked = enabled;
    tplSwitchText.textContent = enabled ? "Đang bật" : "Đang tắt";

    const activeParts: string[] = [];
    if (tpl.excel) activeParts.push(`Excel: ${tpl.excel}`);
    if (tpl.docx) activeParts.push(`Word: ${tpl.docx}`);
    tplLabel.textContent = !enabled
      ? activeParts.length
        ? `Đang tắt (đã lưu sẵn ${activeParts.join(" · ")}, bật công tắc để dùng lại)`
        : "Đang tắt — mặc định dùng thuần BA-Open-Kit + Humanizer."
      : activeParts.length
        ? `Đang bật — ${activeParts.join(" · ")}`
        : "Đang bật — đang dùng mẫu mặc định (Word & Excel). Chọn file riêng bên dưới nếu muốn thay.";
    const detected: string[] = [];
    if (enabled && tpl.excel && tpl.excelColumns?.length) {
      detected.push(`Cột Excel nhận diện: ${tpl.excelColumns.join(" | ")}`);
    }
    if (enabled && tpl.docx && tpl.docxColumns?.length) {
      detected.push(`Cột Word nhận diện: ${tpl.docxColumns.join(" | ")}`);
    }
    tplCols.textContent = detected.length
      ? `${detected.join(" — ")}. AI sẽ điền khớp các cột này khi tạo tóm tắt.`
      : enabled
        ? `Cột mẫu mặc định: ${defaultTemplateCols.join(" | ")}. Bạn có thể nạp file Word (.docx) hoặc Excel (.xlsx) riêng để thay thế.`
        : "Bật công tắc góc phải nếu bạn muốn áp dụng mẫu biên bản Word (.docx) hoặc Excel (.xlsx).";
    renderTemplatePreview(tpl);
    const buttons: HTMLElement[] = [
      el("button", {
        className: "link",
        textContent: "Chọn file mẫu (.xlsx / .docx)",
        onclick: async () => {
          try {
            const updated = await pickAndImportTemplate();
            if (!updated) return;
            tplStatus.textContent = "Đã bật file mẫu biên bản.";
            tplStatus.className = "provider-status status-inline";
            drawTemplateBox(updated);
          } catch (e) {
            tplStatus.textContent = String(e);
            tplStatus.className = "provider-status status-inline bad";
          }
        },
      }),
    ];
    if (tpl.excel) {
      buttons.push(
        el("button", {
          className: "link danger-link",
          textContent: "Tắt template Excel",
          onclick: async () => {
            try {
              const updated = await resetTemplate("excel");
              tplStatus.textContent = "Đã tắt template Excel.";
              tplStatus.className = "provider-status status-inline";
              drawTemplateBox(updated);
            } catch (e) {
              tplStatus.textContent = String(e);
              tplStatus.className = "provider-status status-inline bad";
            }
          },
        }),
      );
    }
    if (tpl.docx) {
      buttons.push(
        el("button", {
          className: "link danger-link",
          textContent: "Tắt template Word",
          onclick: async () => {
            try {
              const updated = await resetTemplate("docx");
              tplStatus.textContent = "Đã tắt template Word.";
              tplStatus.className = "provider-status status-inline";
              drawTemplateBox(updated);
            } catch (e) {
              tplStatus.textContent = String(e);
              tplStatus.className = "provider-status status-inline bad";
            }
          },
        }),
      );
    }
    tplActions.replaceChildren(...buttons);
  };
  invoke<MeetingTemplates>("meeting_templates")
    .then(drawTemplateBox)
    .catch(() => drawTemplateBox({ excel: null, docx: null }));

  // Summary profile selector (default: BA-Open-Kit, toggleable, plus user-uploaded .md profiles).
  const profSwitch = el("input", {
    type: "checkbox",
    className: "switch-input",
    ariaLabel: "Bật dùng profile tóm tắt",
  }) as HTMLInputElement;
  const profSwitchText = el("span", { className: "switch-text", textContent: "Đang bật" });
  const profToggle = el(
    "label",
    { className: "switch" },
    profSwitch,
    el("span", { className: "switch-track" }),
    profSwitchText,
  );
  const profDesc = el("p", {
    className: "style-desc",
    textContent: "Đang tải danh sách profile…",
  });
  const profList = el("div", { className: "profile-list" });
  const profStatus = el("p", { className: "provider-status status-inline" });
  const profUploadBtn = el("button", {
    className: "link",
    textContent: "Tải file .md tạo profile mới",
    onclick: async () => {
      const picked = await open({
        multiple: false,
        filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
      });
      if (!picked || Array.isArray(picked)) return;
      profUploadBtn.disabled = true;
      profStatus.className = "provider-status status-inline";
      profStatus.textContent = "Đang đọc file .md và dùng AI đặt tên profile…";
      try {
        const updated = await invoke<ProfilesConfig>("import_summary_profile", { path: picked });
        const created = updated.custom.find((p) => p.id === updated.activeId);
        profStatus.textContent = created
          ? `Đã tạo và chọn profile "${created.name}".`
          : "Đã tạo profile mới.";
        drawProfiles(updated);
      } catch (e) {
        profStatus.className = "provider-status status-inline bad";
        profStatus.textContent = `Chưa tạo được profile: ${e}`;
      } finally {
        profUploadBtn.disabled = false;
      }
    },
  });

  const drawProfiles = (cfg: ProfilesConfig) => {
    profSwitch.checked = cfg.enabled;
    profSwitchText.textContent = cfg.enabled ? "Đang bật" : "Đang tắt";
    profDesc.textContent = cfg.enabled
      ? "Khi tóm tắt, AI sẽ bám sát hướng dẫn của profile đang chọn. Bạn có thể tải file .md riêng để AI tự đặt tên và áp dụng."
      : "Đang tắt profile — AI tóm tắt gọn gàng theo văn phong bên dưới mà không ép theo khung BA-Open-Kit.";

    if (!cfg.enabled) {
      profList.replaceChildren();
      return;
    }

    const row = (
      id: string,
      name: string,
      desc: string,
      badge: string | null,
      removable: boolean,
    ) =>
      el(
        "div",
        { className: `profile-item${cfg.activeId === id ? " active" : ""}` },
        el(
          "label",
          { className: "profile-pick" },
          el("input", {
            type: "radio",
            name: "summary-profile",
            checked: cfg.activeId === id,
            onchange: async () => {
              try {
                const updated = await invoke<ProfilesConfig>("select_summary_profile", { id });
                profStatus.className = "provider-status status-inline";
                profStatus.textContent = `Đang dùng profile: ${name}.`;
                drawProfiles(updated);
              } catch (e) {
                profStatus.className = "provider-status status-inline bad";
                profStatus.textContent = String(e);
              }
            },
          }),
          el(
            "span",
            { className: "profile-info" },
            el(
              "span",
              { className: "profile-name-row" },
              el("strong", { textContent: name }),
              badge ? el("span", { className: "model-option-badge", textContent: badge }) : null,
            ),
            el("span", { className: "style-desc", textContent: desc }),
          ),
        ),
        removable
          ? el("button", {
              className: "link danger-link",
              textContent: "Xoá",
              onclick: async () => {
                try {
                  const updated = await invoke<ProfilesConfig>("delete_summary_profile", { id });
                  profStatus.className = "provider-status status-inline";
                  profStatus.textContent = `Đã xoá profile "${name}".`;
                  drawProfiles(updated);
                } catch (e) {
                  profStatus.className = "provider-status status-inline bad";
                  profStatus.textContent = String(e);
                }
              },
            })
          : null,
      );

    const nodes: HTMLElement[] = [
      row(
        "ba-open-kit",
        "BA-Open-Kit (/meet)",
        "Bố cục chuẩn 4 phần: Trao đổi chính · Quyết định chốt · Việc cần làm · Điểm cần xác nhận.",
        "Mặc định",
        false,
      ),
      ...cfg.custom.map((p) =>
        row(p.id, p.name, p.description, p.sourceFile ?? "File .md", true),
      ),
    ];
    profList.replaceChildren(...nodes);
  };

  profSwitch.onchange = async () => {
    const next = profSwitch.checked;
    profSwitch.disabled = true;
    try {
      const updated = await invoke<ProfilesConfig>("set_profiles_enabled", { enabled: next });
      profStatus.className = "provider-status status-inline";
      profStatus.textContent = next
        ? "Đã bật profile tóm tắt."
        : "Đã tắt profile — tóm tắt tự do theo văn phong đã chọn.";
      drawProfiles(updated);
    } catch (e) {
      profSwitch.checked = !next;
      profStatus.className = "provider-status status-inline bad";
      profStatus.textContent = String(e);
    } finally {
      profSwitch.disabled = false;
    }
  };

  invoke<ProfilesConfig>("summary_profiles")
    .then(drawProfiles)
    .catch(() => drawProfiles({ enabled: true, activeId: "ba-open-kit", custom: [] }));

  const profileBlock = el(
    "div",
    { className: "style-box" },
    el(
      "div",
      { className: "hf-head" },
      el("div", { className: "style-title tpl-label", textContent: "Profile tóm tắt" }),
      profToggle,
    ),
    profDesc,
    profList,
    el("div", { className: "actions wrap tpl-actions" }, profUploadBtn),
    profStatus,
  );

  const templateBlock = el(
    "div",
    { className: "style-box" },
    el(
      "div",
      { className: "hf-head" },
      el("div", { className: "style-title tpl-label", textContent: "Dùng file mẫu riêng (Word / Excel)" }),
      tplToggle,
    ),
    tplLabel,
    tplCols,
    tplPreview,
    tplActions,
    tplStatus,
  );

  return el(
    "section",
    {},
    el("h3", { textContent: "Cách viết & xuất biên bản" }),
    el("p", {
      className: "hint",
      textContent:
        "Chọn profile tóm tắt, văn phong viết biên bản và tuỳ chọn bật file mẫu Word (.docx) hoặc Excel (.xlsx) khi cần.",
    }),
    profileBlock,
    styleBlock,
    templateBlock,
  );
}

function providerBlock(
  provider: Provider,
  ai: AiView,
  persist: () => Promise<unknown>,
  reload: () => void,
): HTMLElement {
  const name = PROVIDER_NAME[provider];
  const hint = ai.keys[provider];
  const status = el("p", { className: "provider-status" });
  const show = (text: string, bad = false) => {
    status.textContent = text;
    status.className = `provider-status${bad ? " bad" : ""}`;
  };

  const key = el("input", {
    className: "field",
    type: "password",
    placeholder: hint ? `Đang dùng ${hint}. Nhập để thay key` : "Dán API key",
    autocomplete: "off",
    ariaLabel: `API key của ${name}`,
  });

  let modelName = ai.models[provider] ?? "";
  const modelInput = el("input", {
    className: "field",
    value: modelName,
    placeholder: "Nhập ID model",
    ariaLabel: `ID model của ${name}`,
    onchange: () => saveModel(modelInput.value),
  });

  const saveModel = async (value: string) => {
    modelName = value.trim();
    modelInput.value = modelName;
    ai.models[provider] = modelName;
    await persist();
    show("Đã lưu model.");
    if (pickerOpen) drawPicker();
  };

  // Lazy model picker state (US-CFG-02): never fetches on mount; only fetches when opened.
  // When an API key is saved, automatically probes accessible models and filters by default.
  let pickerOpen = false;
  let pickerLoading = false;
  let probing = false;
  let showAllModels = false;
  let pickerError: string | null = null;
  let filterQuery = "";
  const pickerHost = el("div", { className: "model-picker-host" });
  const toggleBtn = el("button", {
    className: "link",
    textContent: "Chọn model",
    title: hint
      ? "Tự lọc và hiển thị các model tài khoản gọi được"
      : "Chọn từ danh sách gợi ý mới nhất (lưu API key để lọc model gọi được)",
    onclick: () => {
      pickerOpen = !pickerOpen;
      toggleBtn.textContent = pickerOpen ? "Đóng danh sách" : "Chọn model";
      if (pickerOpen) {
        const cache = providerModelCache[provider];
        if (hint && (cache.models === null || cache.accessible === null) && !pickerLoading && !probing) {
          fetchProviderModels(false);
        } else {
          drawPicker();
        }
      } else {
        pickerHost.replaceChildren();
      }
    },
  });

  const fetchProviderModels = async (force: boolean) => {
    if (!hint) return;
    const cache = providerModelCache[provider];
    if (!force && cache.models !== null && cache.accessible !== null) {
      drawPicker();
      return;
    }
    const generation = ++cache.generation;
    pickerLoading = force || cache.models === null;
    probing = true;
    pickerError = null;
    drawPicker();
    try {
      // Step 1: fast list_models (~0.3-0.6s), render immediately without blocking on probing.
      let models = cache.models;
      if (force || models === null) {
        models = await invoke<string[]>("list_models", { provider });
        if (providerModelCache[provider].generation !== generation) return;
        cache.models = models;
        pickerLoading = false;
        if (pickerOpen) drawPicker();
      }
      // Step 2: parallel probe (<=5s cap) reusing the already-fetched list (zero duplicate GET).
      const accessible = await invoke<string[]>("probe_accessible_models", {
        provider,
        candidates: models,
      });
      if (providerModelCache[provider].generation !== generation) return;
      cache.accessible = accessible;
      probing = false;
      if (accessible.length > 0 && !accessible.includes(modelName)) {
        await saveModel(accessible[0]);
        show(`Tìm thấy ${accessible.length} model gọi được — đã chọn ${accessible[0]}.`);
      } else if (accessible.length === 0) {
        show("Không có model nào phản hồi thành công với API key hiện tại (hết hạn mức hoặc chưa bật billing).", true);
      }
      if (pickerOpen) drawPicker();
    } catch (e) {
      if (providerModelCache[provider].generation !== generation) return;
      pickerLoading = false;
      probing = false;
      pickerError = String(e);
      if (pickerOpen) drawPicker();
    }
  };

  const drawPicker = () => {
    if (!pickerOpen) {
      pickerHost.replaceChildren();
      return;
    }
    const cached = providerModelCache[provider].models;
    const accessible = providerModelCache[provider].accessible;
    const search = el("input", {
      className: "field model-search",
      type: "search",
      placeholder: "Tìm model theo ID…",
      ariaLabel: `Tìm model của ${name}`,
      value: filterQuery,
      oninput: (e: Event) => {
        filterQuery = (e.target as HTMLInputElement).value;
        drawList();
      },
    });
    const filterToggleBtn = el("button", {
      className: "link",
      textContent: showAllModels ? "Chỉ model gọi được" : "Hiện tất cả",
      disabled: pickerLoading || probing || !hint || accessible === null,
      title: showAllModels
        ? "Chỉ hiện những model tài khoản gọi được"
        : "Xem toàn bộ danh sách model (kể cả model chưa có hạn mức)",
      onclick: () => {
        showAllModels = !showAllModels;
        drawPicker();
      },
    });
    const refreshBtn = el("button", {
      className: "link",
      textContent: "Làm mới",
      disabled: pickerLoading || probing || !hint,
      title: hint ? "Tải lại và kiểm tra model gọi được" : "Lưu API key để tải danh sách theo tài khoản",
      onclick: () => fetchProviderModels(true),
    });
    const listBox = el("div", { className: "model-picker-list" });

    const drawList = () => {
      if (pickerLoading) {
        listBox.replaceChildren(
          el("p", {
            className: "hint",
            textContent: "Đang tải danh sách model…",
          }),
        );
        return;
      }
      if (pickerError) {
        listBox.replaceChildren(
          el("p", { className: "problem problem-text", textContent: `Không tải được danh sách: ${pickerError}` }),
          el("button", {
            className: "link",
            textContent: "Thử lại",
            onclick: () => fetchProviderModels(true),
          }),
        );
        return;
      }
      const fromApi = cached !== null;
      const fallbackRecommended = ai.recommendedModels?.[provider] ?? [];
      const models = cached ?? fallbackRecommended;
      const q = filterQuery.trim().toLowerCase();
      const matches = (id: string) => !q || id.toLowerCase().includes(q);
      const items: HTMLElement[] = [];
      const filteringAccessible = Boolean(hint && accessible !== null && !showAllModels);
      const verifiedSet = new Set(accessible ?? []);

      if (probing) {
        items.push(
          el("p", {
            className: "hint picker-note",
            textContent: "Đang lọc nhanh các model tài khoản gọi được…",
          }),
        );
      }

      if (!hint) {
        items.push(
          el("p", {
            className: "hint picker-note",
            textContent: "Chưa nhập API key — đang hiện các model gợi ý. Lưu key để tự lọc đúng model tài khoản gọi được.",
          }),
        );
      }

      const optionBtn = (id: string, badge?: string) =>
        el(
          "button",
          {
            className: `model-option${id === modelName ? " active" : ""}`,
            onclick: () => saveModel(id),
          },
          el("span", { textContent: id }),
          badge ? el("span", { className: "model-option-badge", textContent: badge }) : null,
        );

      // Saved model always shown at top.
      if (modelName && matches(modelName)) {
        const present = models.includes(modelName);
        const savedBadge =
          accessible !== null
            ? verifiedSet.has(modelName)
              ? "Dùng được · Đang chọn"
              : "Không gọi được"
            : !fromApi || present
              ? "Đang chọn"
              : "Không thấy trong danh sách";
        items.push(el("div", { className: "model-group-label", textContent: "Model đang lưu" }));
        items.push(optionBtn(modelName, savedBadge));
      }

      // Verified accessible models (shown exclusively by default when API key is set).
      if (accessible !== null) {
        const verified = accessible.filter((id) => id !== modelName && matches(id));
        if (verified.length) {
          items.push(
            el("div", {
              className: "model-group-label",
              textContent: `Tài khoản gọi được (${accessible.length})`,
            }),
          );
          for (const id of verified) items.push(optionBtn(id, "Dùng được"));
        } else if (accessible.length === 0) {
          items.push(
            el("p", {
              className: "hint picker-note",
              textContent:
                "Chưa có model nào phản hồi thành công với API key này. Bấm \"Hiện tất cả\" nếu muốn xem toàn bộ danh sách.",
            }),
          );
        }
      }

      // Unverified / other models: hidden by default when accessible filter is active.
      if (!filteringAccessible) {
        const curated = fallbackRecommended.filter((id) => models.includes(id));
        const recommended = (curated.length ? curated : models.slice(0, 3)).filter(
          (id) => id !== modelName && !verifiedSet.has(id) && matches(id),
        );
        if (recommended.length) {
          items.push(el("div", { className: "model-group-label", textContent: "Gợi ý (mới nhất)" }));
          for (const id of recommended) items.push(optionBtn(id));
        }

        const rest = models.filter(
          (id) => id !== modelName && !verifiedSet.has(id) && !recommended.includes(id) && matches(id),
        );
        if (rest.length) {
          items.push(el("div", { className: "model-group-label", textContent: `Model mới khác (${rest.length})` }));
          for (const id of rest) items.push(optionBtn(id));
        }
      }

      if (items.length === 0) {
        items.push(el("p", { className: "hint", textContent: "Không có model nào khớp bộ lọc." }));
      }
      listBox.replaceChildren(...items);
    };

    drawList();
    pickerHost.replaceChildren(
      el(
        "div",
        { className: "model-picker" },
        el("div", { className: "model-picker-head" }, search, filterToggleBtn, refreshBtn),
        listBox,
      ),
    );
  };

  return el(
    "div",
    { className: "provider" },
    el(
      "div",
      { className: "provider-head" },
      el("span", { textContent: name }),
      el(
        "label",
        {},
        el("input", {
          type: "radio",
          name: "default-provider",
          checked: ai.defaultProvider === provider,
          onchange: async () => {
            ai.defaultProvider = provider;
            await persist();
          },
        }),
        "Dùng mặc định",
      ),
    ),
    el(
      "div",
      { className: "field-grid" },
      el("span", { textContent: "API key" }),
      key,
      el("button", {
        className: "link",
        textContent: "Lưu key",
        onclick: async () => {
          if (!key.value.trim()) return show("Hãy dán API key trước.", true);
          try {
            await invoke("set_api_key", { provider, key: key.value });
            invalidateProviderCache(provider);
            reload();
          } catch (e) {
            show(String(e), true);
          }
        },
      }),
      hint
        ? el("button", {
            className: "link",
            textContent: "Xoá key",
            onclick: async () => {
              await invoke("delete_api_key", { provider });
              invalidateProviderCache(provider);
              reload();
            },
          })
        : el("span"),
      el("span", { textContent: "Model" }),
      modelInput,
      toggleBtn,
      el("button", {
        className: "link",
        textContent: "Thử kết nối",
        onclick: async () => {
          show("Đang thử kết nối…");
          try {
            await invoke("test_provider", { provider, model: modelName });
            show(`Kết nối được với ${name} bằng model này.`);
          } catch (e) {
            show(String(e), true);
          }
        },
      }),
    ),
    pickerHost,
    status,
  );
}

function glossarySection(initial: Term[]): HTMLElement {
  let terms = initial;
  const rows = el("tbody");
  const status = el("p", { className: "provider-status status-inline" });
  const persist = async (announce = false) => {
    try {
      await invoke("save_glossary", { terms });
      if (announce) {
        const count = terms.filter((t) => t.correct.trim()).length;
        status.textContent = `Đã lưu danh sách thuật ngữ (${count} từ).`;
        status.className = "provider-status";
      }
    } catch (e) {
      status.textContent = `Chưa lưu được thuật ngữ: ${e}`;
      status.className = "provider-status bad";
    }
  };
  const input = (term: Term, field: keyof Term, label: string, placeholder: string) =>
    el("input", {
      className: "term-input",
      value: term[field],
      placeholder,
      ariaLabel: label,
      oninput: (e: Event) => {
        term[field] = (e.target as HTMLInputElement).value;
        status.textContent = "Có thay đổi chưa lưu.";
        status.className = "provider-status";
      },
      onchange: () => {
        persist(false);
      },
    });
  const draw = () => {
    if (terms.length === 0) {
      rows.replaceChildren(
        el(
          "tr",
          { className: "terms-empty" },
          el("td", { colSpan: 5, textContent: "Chưa có thuật ngữ nào. Bấm \"Thêm từ\" hoặc \"Nhập từ CSV\" để bắt đầu." }),
        ),
      );
      return;
    }
    rows.replaceChildren(
      ...terms.map((term, index) =>
        el(
          "tr",
          {},
          el("td", { className: "terms-num", textContent: String(index + 1) }),
          el("td", {}, input(term, "correct", "Viết đúng", "VD: Bravo")),
          el("td", {}, input(term, "heardAs", "Hay bị nghe thành", "VD: ba vô")),
          el("td", {}, input(term, "note", "Giải thích", "VD: Phần mềm kế toán")),
          el(
            "td",
            { className: "terms-act" },
            el(
              "button",
              {
                className: "icon-btn danger",
                title: "Xoá thuật ngữ này",
                ariaLabel: `Xoá thuật ngữ ${term.correct || index + 1}`,
                onclick: () => {
                  terms = terms.filter((t) => t !== term);
                  persist();
                  draw();
                },
              },
              icon("trash", 14),
            ),
          ),
        ),
      ),
    );
  };
  draw();
  return el(
    "section",
    {},
    el("h3", { textContent: "Từ vựng & tên riêng" }),
    el("p", {
      className: "hint",
      textContent:
        "Thêm tên sản phẩm, từ viết tắt hoặc tên riêng hay bị nghe nhầm. App sẽ dùng danh sách này để nghe chuẩn hơn, sửa chính tả và viết biên bản đúng tên.",
    }),
    el(
      "div",
      { className: "terms-wrap" },
      el(
        "table",
        { className: "terms" },
        el(
          "thead",
          {},
          el(
            "tr",
            {},
            el("th", { className: "terms-num", textContent: "#" }),
            el("th", { textContent: "Viết đúng" }),
            el("th", { textContent: "Hay bị nghe thành" }),
            el("th", { textContent: "Giải thích" }),
            el("th", { className: "terms-act", textContent: "" }),
          ),
        ),
        rows,
      ),
    ),
    el(
      "div",
      { className: "actions wrap" },
      el("button", {
        className: "add",
        textContent: "Lưu thuật ngữ",
        onclick: () => persist(true),
      }),
      el("button", {
        className: "link",
        textContent: "Thêm từ",
        onclick: () => {
          terms = [...terms, { correct: "", heardAs: "", note: "" }];
          draw();
          rows.querySelector<HTMLInputElement>("tr:last-child input")?.focus();
        },
      }),
      el("button", {
        className: "link",
        textContent: "Nhập từ CSV",
        onclick: async () => {
          const path = await open({ filters: [{ name: "CSV", extensions: ["csv"] }] });
          if (!path) return;
          try {
            terms = await invoke<Term[]>("import_glossary", { terms, path });
            draw();
            status.textContent = `Danh sách hiện có ${terms.length} từ.`;
          } catch (e) {
            status.textContent = String(e);
          }
        },
      }),
      el("button", {
        className: "link",
        textContent: "Xuất ra CSV",
        onclick: async () => {
          const path = await save({ defaultPath: "thuat-ngu.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
          if (!path) return;
          try {
            await invoke("export_glossary", { terms, path });
            status.textContent = "Đã xuất danh sách.";
          } catch (e) {
            status.textContent = String(e);
          }
        },
      }),
    ),
    status,
  );
}

function updateSection(): HTMLElement {
  const versionText = el("p", {
    className: "style-desc tpl-label",
    textContent: "Phiên bản hiện tại: đang đọc…",
  });
  const resultBox = el("div", { className: "update-result" });
  const status = el("p", { className: "provider-status status-inline" });

  invoke<string>("app_version")
    .then((v) => {
      versionText.textContent = `Phiên bản hiện tại: A-Sir v${v}`;
    })
    .catch(() => {
      versionText.textContent = "Phiên bản hiện tại: A-Sir";
    });

  const checkBtn = el("button", {
    className: "add",
    textContent: "Kiểm tra bản cập nhật",
    onclick: async () => {
      checkBtn.disabled = true;
      status.className = "provider-status status-inline";
      status.textContent = "Đang kiểm tra phiên bản mới trên GitHub Releases…";
      resultBox.replaceChildren();
      try {
        const info = await invoke<UpdateInfo>("check_update");
        versionText.textContent = `Phiên bản hiện tại: A-Sir v${info.currentVersion}`;
        if (info.hasUpdate) {
          status.textContent = `Đã có bản mới A-Sir v${info.latestVersion}!`;
          const targetUrl = info.downloadUrl ?? info.releaseUrl;
          const nodes: HTMLElement[] = [];
          if (info.notes) {
            nodes.push(el("div", { className: "update-notes", textContent: info.notes }));
          }
          nodes.push(
            el(
              "div",
              { className: "actions wrap tpl-actions" },
              info.downloadUrl
                ? el("button", {
                    className: "add",
                    textContent: `Cập nhật lên v${info.latestVersion}`,
                    onclick: async (e: MouseEvent) => {
                      const btn = e.currentTarget as HTMLButtonElement;
                      btn.disabled = true;
                      status.className = "provider-status status-inline";
                      status.textContent = "Đang tải bộ cài, vui lòng đợi…";
                      try {
                        await invoke("install_update", { url: info.downloadUrl });
                        status.textContent =
                          "Đã tải xong và mở bộ cài. Hãy đóng A-Sir rồi cài theo hướng dẫn.";
                      } catch (err) {
                        status.className = "provider-status status-inline bad";
                        status.textContent = `Chưa cập nhật được: ${err}`;
                        btn.disabled = false;
                      }
                    },
                  })
                : el("button", {
                    className: "add",
                    textContent: `Mở trang tải v${info.latestVersion}`,
                    onclick: () => invoke("open_url", { url: targetUrl }),
                  }),
              el("button", {
                className: "link",
                textContent: "Xem ghi chú phát hành",
                onclick: () => invoke("open_url", { url: info.releaseUrl }),
              }),
            ),
          );
          resultBox.replaceChildren(...nodes);
        } else {
          status.textContent = `Bạn đang dùng phiên bản mới nhất (v${info.currentVersion}).`;
          resultBox.replaceChildren(
            el(
              "div",
              { className: "actions wrap tpl-actions" },
              el("button", {
                className: "link",
                textContent: "Mở trang GitHub Releases",
                onclick: () => invoke("open_url", { url: info.releaseUrl }),
              }),
            ),
          );
        }
      } catch (e) {
        status.className = "provider-status status-inline bad";
        status.textContent = `Chưa kiểm tra được bản cập nhật: ${e}`;
      } finally {
        checkBtn.disabled = false;
      }
    },
  });

  const section = el(
    "section",
    {},
    el("h3", { textContent: "Cập nhật phần mềm" }),
    el("p", {
      className: "hint",
      textContent:
        "Kiểm tra phiên bản A-Sir mới nhất (macOS và Windows) từ GitHub Releases.",
    }),
    el(
      "div",
      { className: "style-box" },
      versionText,
      el("div", { className: "actions wrap tpl-actions" }, checkBtn),
      status,
      resultBox,
    ),
  );
  // Tự kiểm tra ngay khi mở tab, không cần bấm nút.
  queueMicrotask(() => checkBtn.click());
  return section;
}
