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
  type Provider,
  progressBar,
  resetTemplate,
  setTemplatesEnabled,
  type SummaryStyle,
  type SummaryStyleOption,
} from "./ipc";

const PROVIDER_KEY_URL: Record<Provider, string> = {
  claude: "https://console.anthropic.com/settings/keys",
  chatGpt: "https://platform.openai.com/api-keys",
  gemini: "https://aistudio.google.com/apikey",
};

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
      autoFallback: ai.autoFallback ?? true,
    },
  });
}

type Term = { correct: string; heardAs: string; note: string };

type OsId = "mac" | "windows" | "linux" | "mobile";
const OS_LABEL: Record<OsId, string> = {
  mac: "macOS",
  windows: "Windows",
  linux: "Linux",
  mobile: "Di động",
};

type SystemInfo = {
  os: OsId;
  arch: string;
  chip: string;
  ramGb: number;
  cpuCores: number;
};

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
  minRamGb?: number;
  compatible?: boolean;
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
    description: "Dồn tối đa sức mạnh của máy để chuyển văn bản nhanh nhất.",
  },
];

type LocalModelsView = {
  selected: string;
  resourceProfile: ResourceProfile;
  system?: SystemInfo;
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

type TabId = "speech" | "ai" | "minutes" | "glossary" | "storage" | "update";

const TABS: { id: TabId; label: string }[] = [
  { id: "speech", label: "Chuyển văn bản" },
  { id: "ai", label: "Tài khoản AI" },
  { id: "minutes", label: "Biên bản" },
  { id: "glossary", label: "Từ vựng riêng" },
  { id: "storage", label: "Dữ liệu & dọn dẹp" },
  { id: "update", label: "Cập nhật" },
];

type StorageBucket = { files: number; bytes: number };
type StorageStats = {
  cache: StorageBucket;
  cleaned: StorageBucket;
  diagrams: StorageBucket;
  transcripts: StorageBucket;
};
type CleanupTarget = "cache" | "cleaned" | "diagrams" | "transcripts";
type CleanupOutcome = {
  removedFiles: number;
  freedBytes: number;
  stats: StorageStats;
};

type LoadState<T> =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; data: T }
  | { kind: "error"; message: string };

type ProviderCache = {
  models: string[] | null;
  accessible: string[] | null;
  fetchedAt: number;
  generation: number;
};

const MODEL_CACHE_TTL_MS = 24 * 60 * 60 * 1000;
const MODEL_CACHE_KEY = "asr-ai-model-cache-v1";

function loadPersistedModelCache(): Partial<Record<Provider, { models: string[]; fetchedAt: number }>> {
  try {
    const raw = localStorage.getItem(MODEL_CACHE_KEY);
    if (!raw) return {};
    return JSON.parse(raw) as Partial<Record<Provider, { models: string[]; fetchedAt: number }>>;
  } catch {
    return {};
  }
}

function persistProviderModels(provider: Provider, models: string[], fetchedAt: number) {
  try {
    const current = loadPersistedModelCache();
    current[provider] = { models, fetchedAt };
    localStorage.setItem(MODEL_CACHE_KEY, JSON.stringify(current));
  } catch {
    // Ignore storage quota issues.
  }
}

function clearPersistedProviderModels(provider: Provider) {
  try {
    const current = loadPersistedModelCache();
    delete current[provider];
    localStorage.setItem(MODEL_CACHE_KEY, JSON.stringify(current));
  } catch {
    // Ignore storage quota issues.
  }
}

function isCacheFresh(cache: ProviderCache): boolean {
  return cache.models !== null && Date.now() - cache.fetchedAt < MODEL_CACHE_TTL_MS;
}

const persistedCache = loadPersistedModelCache();
const initCache = (p: Provider): ProviderCache => {
  const saved = persistedCache[p];
  if (saved && Array.isArray(saved.models) && Date.now() - saved.fetchedAt < MODEL_CACHE_TTL_MS) {
    return { models: saved.models, accessible: null, fetchedAt: saved.fetchedAt, generation: 0 };
  }
  return { models: null, accessible: null, fetchedAt: 0, generation: 0 };
};

// Session + 24h localStorage cache for provider model lists; invalidated when a key changes or is deleted.
const providerModelCache: Record<Provider, ProviderCache> = {
  claude: initCache("claude"),
  chatGpt: initCache("chatGpt"),
  gemini: initCache("gemini"),
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
  providerModelCache[provider].fetchedAt = 0;
  providerModelCache[provider].generation += 1;
  clearPersistedProviderModels(provider);
}

function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  return `${Math.round(bytes / 1_000_000)} MB`;
}

/** Fills itself in and keeps its own tab & data state, so the rest of the app can leave it alone. */
export function settingsPane(initialTab?: TabId): HTMLElement {
  let activeTab: TabId = initialTab ?? lastActiveTab;
  if (initialTab) lastActiveTab = initialTab;
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
        panel.replaceChildren(el("p", { className: "hint", textContent: "Đang đọc danh sách model chuyển văn bản…" }));
      } else if (speechState.kind === "error") {
        panel.replaceChildren(errorBlock("Model chuyển văn bản cục bộ", speechState.message, loadSpeech));
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

    if (activeTab === "storage") {
      panel.replaceChildren(storageSection());
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

const CURATED_SPEECH_IDS = [
  "whisper-large-v3-turbo-q5_0",
  "whisper-large-v3-turbo",
  "whisper-large-v3",
  "phowhisper-medium-q5_0",
];

const SPEECH_PLAIN_DESC: Record<string, string> = {
  "whisper-large-v3-turbo-q5_0": "Nhanh, nhẹ máy — hợp máy 8 GB RAM và cuộc họp dài.",
  "whisper-large-v3-turbo": "Cân bằng tốc độ và độ chính xác tiếng Việt & tiếng Anh.",
  "whisper-large-v3": "Chính xác cao nhất cho nhiều thuật ngữ khó (cần máy ≥ 16 GB RAM).",
  "phowhisper-medium-q5_0": "Chuyên cho cuộc họp thuần tiếng Việt, nhẹ máy.",
};

function recommendedSpeechId(data: LocalModelsView): string {
  const ram = data.system?.ramGb ?? 8;
  const preferred = ram <= 8 ? "whisper-large-v3-turbo-q5_0" : "whisper-large-v3-turbo";
  if (data.models.some((m) => m.id === preferred && m.compatible !== false)) return preferred;
  return data.models.find((m) => m.compatible !== false)?.id ?? data.selected;
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
  const recId = recommendedSpeechId(data);
  const activeModel = data.models.find((m) => m.selected && m.ready);
  const sys = data.system;
  const hwSummary = sys
    ? [sys.chip || (OS_LABEL[sys.os] ?? sys.os), sys.ramGb > 0 ? `${sys.ramGb} GB RAM` : ""].filter(Boolean).join(" · ")
    : "";

  const summaryBanner = el(
    "div",
    { className: "settings-summary-bar" },
    el(
      "div",
      { className: "settings-summary-main" },
      el("strong", {
        textContent: activeModel ? `Đang dùng: ${activeModel.name}` : "Chưa chọn gói nhận dạng giọng nói",
      }),
      hwSummary ? el("span", { className: "settings-summary-meta", textContent: hwSummary }) : null,
    ),
  );

  // Visual 3-option speed picker instead of dropdown + verbose copy.
  const speedGroup = el(
    "div",
    { className: "speed-pills", role: "radiogroup", ariaLabel: "Mức tốc độ xử lý" },
    ...RESOURCE_PROFILES.map((p) =>
      el(
        "button",
        {
          type: "button",
          className: `speed-pill${p.id === currentProfile.id ? " selected" : ""}`,
          onclick: () => {
            if (p.id !== currentProfile.id) onResourceChange(p.id);
          },
        },
        el("strong", { textContent: p.label }),
        el("span", { textContent: p.description }),
      ),
    ),
  );

  const resourceBox = el(
    "div",
    { className: "style-box" },
    el("div", { className: "style-title", textContent: "Mức sử dụng máy khi chuyển văn bản" }),
    speedGroup,
    resourceErr
      ? el("p", { className: "provider-status status-inline bad", textContent: resourceErr })
      : null,
  );

  const renderModelCard = (m: LocalModelView) => {
    const progress = downloading.get(m.id);
    const error = errors.get(m.id);
    const active = m.selected && m.ready;
    const isRec = m.id === recId;

    const action =
      progress !== undefined
        ? el("span", { className: "model-status-text", textContent: `Đang tải ${progress}%` })
        : active
          ? el("span", { className: "model-badge active", textContent: "Đang dùng" })
          : m.ready
            ? el(
                "div",
                { className: "actions" },
                el("button", {
                  type: "button",
                  className: "add",
                  textContent: "Chọn dùng",
                  onclick: () => onSelect(m.id),
                }),
                el("button", {
                  type: "button",
                  className: "link danger-link",
                  textContent: "Xoá",
                  onclick: () => onDelete(m),
                }),
              )
            : el("button", {
                type: "button",
                className: "btn-ghost",
                textContent: error ? "Thử tải lại" : "Tải về",
                onclick: () => onDownload(m.id),
              });

    const compatWarn =
      m.compatible === false
        ? el("span", {
            className: "model-badge",
            textContent:
              sys && m.platforms && !m.platforms.includes(sys.os)
                ? `Không hỗ trợ ${OS_LABEL[sys.os]}`
                : "Nặng với máy này",
          })
        : null;

    return el(
      "div",
      { className: `model-card${active ? " selected" : ""}` },
      el(
        "div",
        { className: "model-card-head" },
        el(
          "div",
          { className: "model-card-title" },
          el("strong", { textContent: m.name }),
          isRec && !active ? el("span", { className: "onboarding-badge", textContent: "Khuyên dùng" }) : null,
          m.ready && !active ? el("span", { className: "model-badge ready", textContent: "Đã tải" }) : null,
          compatWarn,
        ),
        action,
      ),
      el("p", { className: "model-desc", textContent: SPEECH_PLAIN_DESC[m.id] ?? m.description }),
      el(
        "div",
        { className: "model-meta" },
        el("span", {
          textContent: m.minRamGb
            ? `${formatBytes(m.bytes)} · Cần ≥ ${m.minRamGb} GB RAM`
            : formatBytes(m.bytes),
        }),
      ),
      progress !== undefined ? progressBar(progress) : null,
      error ? el("p", { className: "provider-status status-inline bad", textContent: error }) : null,
    );
  };

  // Curated main list: only the 3–4 practical choices + any already-downloaded/selected model.
  const curatedSet = new Set(CURATED_SPEECH_IDS);
  const primaryModels = data.models.filter(
    (m) => (curatedSet.has(m.id) && m.compatible !== false) || m.ready || m.selected,
  );
  const primaryIds = new Set(primaryModels.map((m) => m.id));
  const extraModels = data.models.filter((m) => !primaryIds.has(m.id));

  const mainModelList = el(
    "div",
    { className: "model-list" },
    ...(primaryModels.length ? primaryModels : data.models.slice(0, 4)).map(renderModelCard),
  );

  // Power-user Hugging Face search + extra models tucked cleanly inside Advanced disclosure.
  const hfQuery = el("input", {
    className: "field",
    type: "search",
    value: hfQueryText,
    placeholder: "Tìm model trên Hugging Face (VD: phowhisper, whisper)…",
    ariaLabel: "Tìm model chuyển văn bản mới trên Hugging Face",
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
                type: "button",
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
    hfStatusText = "Đang tìm model trên Hugging Face…";
    hfStatus.className = "provider-status status-inline";
    hfStatus.textContent = hfStatusText;
    hfFound = [];
    hfResults.replaceChildren();
    try {
      const found = await invoke<DiscoveredModel[]>("discover_hf_models", { query: hfQueryText });
      hfFound = found;
      if (found.length === 0) {
        hfStatusText = "Không tìm thấy model phù hợp với từ khoá này.";
        hfStatus.textContent = hfStatusText;
        return;
      }
      hfStatusText = `Tìm thấy ${found.length} model.`;
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
    type: "button",
    className: "btn-ghost",
    textContent: "Tìm kiếm",
    onclick: runDiscover,
  });
  hfQuery.addEventListener("keydown", (e) => {
    if (e.key === "Enter") runDiscover();
  });

  const advancedBox = el(
    "details",
    { className: "advanced-box" },
    el("summary", {
      textContent:
        extraModels.length > 0
          ? `Tuỳ chọn nâng cao (${extraModels.length} model khác, Hugging Face, thư mục lưu)`
          : "Tuỳ chọn nâng cao (Hugging Face, thư mục lưu model)",
    }),
    el(
      "div",
      { className: "advanced-body" },
      extraModels.length > 0
        ? el("div", { className: "model-list" }, ...extraModels.map(renderModelCard))
        : null,
      el(
        "div",
        { className: "style-box" },
        el(
          "div",
          { className: "hf-head" },
          el("div", { className: "style-title tpl-label", textContent: "Tải thêm model cộng đồng (Hugging Face)" }),
          el("button", {
            type: "button",
            className: "link",
            textContent: "Mở thư mục lưu model",
            onclick: () => invoke("open_models_dir"),
          }),
        ),
        el("div", { className: "hf-bar" }, hfQuery, hfBtn),
        hfStatus,
        hfResults,
      ),
    ),
  );

  return el(
    "section",
    {},
    el("h3", { textContent: "Chuyển văn bản trên máy" }),
    summaryBanner,
    resourceBox,
    mainModelList,
    advancedBox,
  );
}

const PROVIDER_ORDER: Provider[] = ["gemini", "chatGpt", "claude"];
const PROVIDER_KEY_STEPS: Record<Provider, string[]> = {
  gemini: [
    "Mở Google AI Studio và đăng nhập tài khoản Google (có gói miễn phí).",
    "Bấm \"Create API key\" rồi chọn dự án mặc định.",
    "Sao chép chuỗi key (bắt đầu bằng AIza...) và dán vào ô bên dưới.",
  ],
  chatGpt: [
    "Mở OpenAI Platform và đăng nhập tài khoản OpenAI (cần có số dư API).",
    "Vào API keys → bấm \"Create new secret key\".",
    "Sao chép key (bắt đầu bằng sk-...) và dán vào ô bên dưới.",
  ],
  claude: [
    "Mở Anthropic Console và đăng nhập (cần có credit API).",
    "Vào API Keys → bấm \"Create Key\".",
    "Sao chép key (bắt đầu bằng sk-ant-...) và dán vào ô bên dưới.",
  ],
};

let activeAiProvider: Provider | null = null;

function aiSection(ai: AiView, reload: () => void): HTMLElement {
  const persist = () => persistAiSettings(ai);
  const hasAnyKey = PROVIDER_ORDER.some((p) => Boolean(ai.keys[p]));
  if (!activeAiProvider) {
    activeAiProvider = hasAnyKey ? ai.defaultProvider : "gemini";
  }

  const tabsHost = el("div", {
    className: "ai-provider-tabs",
    role: "tablist",
    ariaLabel: "Chọn nhà cung cấp AI",
  });
  const formHost = el("div", { className: "ai-provider-form-host" });

  const renderProviderView = () => {
    const selected = activeAiProvider ?? "gemini";
    tabsHost.replaceChildren(
      ...PROVIDER_ORDER.map((p) => {
        const hasKey = Boolean(ai.keys[p]);
        const isDefault = ai.defaultProvider === p;
        const badgeText = isDefault && hasKey
          ? "Đang dùng"
          : hasKey
            ? "Đã có key"
            : p === "gemini"
              ? "Miễn phí"
              : "";
        return el(
          "button",
          {
            type: "button",
            role: "tab",
            className: `ai-provider-tab${selected === p ? " selected" : ""}`,
            ariaSelected: String(selected === p),
            onclick: () => {
              activeAiProvider = p;
              renderProviderView();
            },
          },
          el("strong", { textContent: PROVIDER_NAME[p] }),
          badgeText
            ? el("span", {
                className: `ai-provider-badge${isDefault && hasKey ? " active" : ""}`,
                textContent: badgeText,
              })
            : null,
        );
      }),
    );
    formHost.replaceChildren(providerBlock(selected, ai, persist, reload, renderProviderView));
  };

  renderProviderView();

  const fallbackSwitch = el("input", {
    type: "checkbox",
    className: "switch-input",
    checked: ai.autoFallback ?? true,
    ariaLabel: "Tự chuyển sang model hoặc nhà cung cấp khác khi quá tải",
  }) as HTMLInputElement;
  const fallbackText = el("span", {
    className: "switch-text",
    textContent: fallbackSwitch.checked ? "Đang bật" : "Đang tắt",
  });
  const fallbackStatus = el("p", { className: "provider-status status-inline" });
  fallbackSwitch.onchange = async () => {
    const next = fallbackSwitch.checked;
    ai.autoFallback = next;
    fallbackText.textContent = next ? "Đang bật" : "Đang tắt";
    fallbackStatus.textContent = "";
    try {
      await persist();
    } catch (e) {
      ai.autoFallback = !next;
      fallbackSwitch.checked = !next;
      fallbackText.textContent = !next ? "Đang bật" : "Đang tắt";
      fallbackStatus.textContent = String(e);
      fallbackStatus.className = "provider-status status-inline bad";
    }
  };
  const fallbackBox = el(
    "div",
    { className: "style-box" },
    el(
      "div",
      { className: "hf-head" },
      el("div", {
        className: "style-title tpl-label",
        textContent: "Tự đổi AI dự phòng khi quá tải",
      }),
      el("label", { className: "switch" }, fallbackSwitch, el("span", { className: "switch-track" }), fallbackText),
    ),
    fallbackStatus,
  );

  return el(
    "section",
    {},
    el("h3", { textContent: "Tài khoản AI" }),
    tabsHost,
    formHost,
    fallbackBox,
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
        ? "Đã bật dùng file mẫu xuất biên bản."
        : "Đã tắt file mẫu riêng.";
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
      "Giữ luồng chuyển văn bản tại máy và thêm xuất biên bản Word/Excel.",
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
      tplPreview.hidden = true;
      tplPreview.replaceChildren();
      return;
    }
    tplPreview.hidden = false;
    if (!tpl.excel && !tpl.docx) {
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
        ? activeParts.join(" · ")
        : ""
      : activeParts.length
        ? activeParts.join(" · ")
        : "";
    tplLabel.hidden = !tplLabel.textContent;
    const detected: string[] = [];
    if (enabled && tpl.excel && tpl.excelColumns?.length) {
      detected.push(`Excel: ${tpl.excelColumns.join(" | ")}`);
    }
    if (enabled && tpl.docx && tpl.docxColumns?.length) {
      detected.push(`Word: ${tpl.docxColumns.join(" | ")}`);
    }
    tplCols.textContent = detected.length
      ? detected.join(" — ")
      : enabled
        ? defaultTemplateCols.join(" | ")
        : "";
    tplCols.hidden = !tplCols.textContent;
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

  // Summary profile selector (default: BA, toggleable, plus user-uploaded .md profiles).
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
      : "Đang tắt profile — AI tóm tắt gọn gàng theo văn phong bên dưới mà không ép theo khung profile BA.";

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
        "BA",
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

  const advancedMinutes = el(
    "details",
    { className: "advanced-box" },
    el("summary", { textContent: "Tuỳ chọn nâng cao (Mẫu Word/Excel riêng & Khung biên bản)" }),
    el("div", { className: "advanced-body" }, templateBlock, profileBlock),
  );

  return el(
    "section",
    {},
    el("h3", { textContent: "Cách viết & xuất biên bản" }),
    styleBlock,
    advancedMinutes,
  );
}

function providerBlock(
  provider: Provider,
  ai: AiView,
  persist: () => Promise<unknown>,
  reload: () => void,
  onTabsDirty: () => void,
): HTMLElement {
  const name = PROVIDER_NAME[provider];
  const hint = ai.keys[provider];
  const status = el("p", { className: "provider-status" });
  const show = (text: string, tone: boolean | "bad" | "warn" | "ok" = false) => {
    const kind = tone === true ? "bad" : tone === false ? "" : tone;
    status.textContent = text;
    status.className = `provider-status${kind ? ` ${kind}` : ""}`;
  };

  let modelName = ai.models[provider] ?? "";
  let fetchingModels = false;
  let testingConn = false;

  const keyInput = el("input", {
    className: "field",
    type: "password",
    placeholder: hint ? `Đang dùng ${hint} — dán key mới để thay` : `Dán API key ${name}`,
    autocomplete: "off",
    ariaLabel: `API key của ${name}`,
  }) as HTMLInputElement;

  const modelHost = el("div", { className: "ai-model-control" });

  const saveModel = async (value: string, announce = true) => {
    const cleaned = value.trim();
    if (!cleaned) return;
    modelName = cleaned;
    ai.models[provider] = cleaned;
    await persist();
    if (announce) show(`Đã chọn model ${cleaned}.`, "ok");
  };

  const availableModelOptions = (): string[] => {
    const cache = providerModelCache[provider];
    const list: string[] = [];
    const pushUnique = (id: string) => {
      const t = id.trim();
      if (t && !list.includes(t)) list.push(t);
    };
    if (modelName) pushUnique(modelName);
    for (const id of cache.models ?? []) pushUnique(id);
    for (const id of ai.recommendedModels?.[provider] ?? []) pushUnique(id);
    return list;
  };

  const renderModelControl = () => {
    const options = availableModelOptions();
    const select = el(
      "select",
      {
        className: "field ai-model-select",
        ariaLabel: `Chọn model của ${name}`,
        onchange: () => void saveModel(select.value),
      },
      ...options.map((id) =>
        el("option", {
          value: id,
          selected: id === modelName,
          textContent: id,
        }),
      ),
    ) as HTMLSelectElement;

    const refreshBtn = el("button", {
      type: "button",
      className: "link",
      textContent: fetchingModels ? "Đang tải…" : "Tải lại danh sách",
      disabled: !hint || fetchingModels,
      title: hint ? "Lấy danh sách model mới nhất (1 request GET /models)" : "Lưu API key trước",
      onclick: () => void fetchModelsFast(true),
    });

    modelHost.replaceChildren(select, refreshBtn);
  };

  /** Fast single GET /models (~0.3–0.5s, 0 tokens). Never runs 6-model probe and never hides models. */
  const fetchModelsFast = async (force: boolean, announce = true): Promise<string[] | null> => {
    const cache = providerModelCache[provider];
    if (!force && isCacheFresh(cache) && cache.models) {
      renderModelControl();
      return cache.models;
    }
    const generation = ++cache.generation;
    fetchingModels = true;
    renderModelControl();
    try {
      const models = await invoke<string[]>("list_models", { provider });
      if (providerModelCache[provider].generation !== generation) return null;
      const now = Date.now();
      cache.models = models;
      cache.fetchedAt = now;
      persistProviderModels(provider, models, now);
      if (models.length > 0 && (!modelName || !models.includes(modelName))) {
        await saveModel(models[0], false);
      }
      if (announce) {
        show(`Đã tải ${models.length} model từ ${name}.`, "ok");
      }
      return models;
    } catch (e) {
      if (providerModelCache[provider].generation === generation && announce) {
        show(String(e), "bad");
      }
      return null;
    } finally {
      fetchingModels = false;
      renderModelControl();
    }
  };

  /** Single fail-fast completion ping (6s timeout, 0 retries). */
  const runTestConnection = async () => {
    if (!hint || testingConn) return;
    testingConn = true;
    testBtn.disabled = true;
    testBtn.textContent = "Đang kiểm tra…";
    show(`Đang thử kết nối ${name} (${modelName})…`);
    const generation = providerModelCache[provider].generation;
    try {
      const res = await invoke<{ ok: boolean; transient: boolean; message: string }>("test_provider", {
        provider,
        model: modelName,
      });
      if (providerModelCache[provider].generation !== generation) return;
      if (res?.transient) {
        show(res.message, "warn");
      } else {
        show(res?.message || "Kết nối thành công.", "ok");
      }
    } catch (e) {
      if (providerModelCache[provider].generation !== generation) return;
      show(String(e), "bad");
    } finally {
      testingConn = false;
      testBtn.disabled = !hint;
      testBtn.textContent = "Thử kết nối";
    }
  };

  const saveKeyBtn = el("button", {
    type: "button",
    className: "btn-primary",
    textContent: "Lưu key",
    onclick: async () => {
      const raw = keyInput.value.trim();
      if (!raw) return show("Hãy dán API key trước.", "bad");
      saveKeyBtn.disabled = true;
      saveKeyBtn.textContent = "Đang xác thực…";
      show(`Đang xác thực key với ${name}…`);
      try {
        await invoke("set_api_key", { provider, key: raw });
        invalidateProviderCache(provider);
        keyInput.value = "";
        // Fast verification + model list load in 1 GET /models call (~0.4s, 0 tokens).
        const models = await invoke<string[]>("list_models", { provider });
        const now = Date.now();
        providerModelCache[provider].models = models;
        providerModelCache[provider].fetchedAt = now;
        persistProviderModels(provider, models, now);
        if (models.length > 0 && (!modelName || !models.includes(modelName))) {
          modelName = models[0];
          ai.models[provider] = modelName;
        }
        const hadNoKey = !PROVIDER_ORDER.some((p) => Boolean(ai.keys[p]));
        if (hadNoKey) {
          ai.defaultProvider = provider;
        }
        await persist();
        reload();
      } catch (e) {
        show(`Key đã lưu nhưng chưa xác thực được: ${e}`, "bad");
        saveKeyBtn.disabled = false;
        saveKeyBtn.textContent = "Lưu key";
      }
    },
  }) as HTMLButtonElement;

  keyInput.onkeydown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !saveKeyBtn.disabled) saveKeyBtn.click();
  };

  const testBtn = el("button", {
    type: "button",
    className: "btn-ghost",
    textContent: "Thử kết nối",
    disabled: !hint,
    title: hint ? `Gọi nhanh 1 yêu cầu tới ${name} (timeout 6s)` : "Lưu API key trước",
    onclick: () => void runTestConnection(),
  }) as HTMLButtonElement;

  const guideDetails = el(
    "details",
    { className: "ai-key-guide" },
    el("summary", { textContent: `Cách lấy API key ${name} (3 bước)` }),
    el(
      "ol",
      { className: "ai-key-steps" },
      ...PROVIDER_KEY_STEPS[provider].map((step) => el("li", { textContent: step })),
    ),
    el("button", {
      type: "button",
      className: "link",
      textContent: `Mở trang tạo API key ${name} ↗`,
      onclick: () => void invoke("open_url", { url: PROVIDER_KEY_URL[provider] }),
    }),
  ) as HTMLDetailsElement;
  guideDetails.open = !hint;

  const defaultCheck = el("input", {
    type: "checkbox",
    checked: ai.defaultProvider === provider,
    onchange: async () => {
      if (!defaultCheck.checked) {
        defaultCheck.checked = true;
        return;
      }
      ai.defaultProvider = provider;
      await persist();
      onTabsDirty();
    },
  }) as HTMLInputElement;

  renderModelControl();

  // Background refresh of model list only if key exists and cache is stale (1 fast GET, silent).
  if (hint && !isCacheFresh(providerModelCache[provider])) {
    void fetchModelsFast(false, false);
  }

  return el(
    "div",
    { className: "ai-form-card" },
    el(
      "div",
      { className: "ai-form-head" },
      el("strong", { textContent: name }),
      el(
        "label",
        { className: "ai-default-toggle" },
        defaultCheck,
        el("span", { textContent: "Dùng mặc định" }),
      ),
    ),
    guideDetails,
    el(
      "div",
      { className: "ai-form-row" },
      el("span", { className: "ai-form-label", textContent: "API key" }),
      el(
        "div",
        { className: "ai-form-inline" },
        keyInput,
        saveKeyBtn,
        hint
          ? el("button", {
              type: "button",
              className: "link danger-link",
              textContent: "Xoá key",
              onclick: async () => {
                await invoke("delete_api_key", { provider });
                invalidateProviderCache(provider);
                reload();
              },
            })
          : null,
      ),
    ),
    el(
      "div",
      { className: "ai-form-row" },
      el("span", { className: "ai-form-label", textContent: "Model" }),
      el("div", { className: "ai-form-inline" }, modelHost, testBtn),
    ),
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

function formatStorageSize(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${Math.round(bytes / 1_000)} KB`;
  return `${bytes} B`;
}

function storageSection(): HTMLElement {
  const host = el("section", {});
  const listEl = el("div", { className: "storage-list" });
  const status = el("p", { className: "provider-status status-inline" });
  let busyTarget: CleanupTarget | null = null;
  let currentStats: StorageStats | null = null;

  const ITEMS: {
    target: CleanupTarget;
    title: string;
    desc: string;
    btnLabel: string;
    confirmText: string;
    danger?: boolean;
  }[] = [
    {
      target: "cache",
      title: "Bộ nhớ đệm & tệp tạm",
      desc: "Tệp giải mã âm thanh tạm và cache danh sách model AI. Không ảnh hưởng văn bản hay biên bản.",
      btnLabel: "Xoá cache",
      confirmText: "Xoá toàn bộ bộ nhớ đệm và tệp tạm của ứng dụng?",
    },
    {
      target: "cleaned",
      title: "Các bản làm sạch",
      desc: "Các bản văn bản đã lọc máy / AI. Văn bản gốc và biên bản tóm tắt vẫn được giữ nguyên.",
      btnLabel: "Xoá bản làm sạch",
      confirmText: "Xoá toàn bộ các bản làm sạch đã lưu? Văn bản gốc vẫn được giữ nguyên.",
      danger: true,
    },
    {
      target: "diagrams",
      title: "Các sơ đồ đã tạo",
      desc: "Toàn bộ sơ đồ quy trình đã lưu. Có thể tạo lại bất cứ lúc nào từ văn bản.",
      btnLabel: "Xoá sơ đồ",
      confirmText: "Xoá toàn bộ sơ đồ quy trình đã lưu?",
      danger: true,
    },
    {
      target: "transcripts",
      title: "Các bản ghi đã chuyển văn bản",
      desc: "Xoá danh sách đã xong cùng toàn bộ văn bản, bản làm sạch, biên bản và sơ đồ. File ghi âm gốc trên máy KHÔNG bị xoá.",
      btnLabel: "Xoá toàn bộ bản ghi đã xong",
      confirmText:
        "Xoá danh sách đã xong và toàn bộ file văn bản, biên bản, sơ đồ trong thư mục kết quả? File ghi âm gốc của bạn trên máy sẽ KHÔNG bị xoá.",
      danger: true,
    },
  ];

  const runClear = async (item: (typeof ITEMS)[number]) => {
    if (busyTarget) return;
    const yes = await ask(item.confirmText, {
      title: item.btnLabel,
      kind: item.danger ? "warning" : "info",
      okLabel: "Xoá",
      cancelLabel: "Huỷ",
    });
    if (!yes) return;
    busyTarget = item.target;
    status.className = "provider-status status-inline";
    status.textContent = "Đang dọn dẹp…";
    draw();
    try {
      if (item.target === "cache") {
        try {
          localStorage.removeItem(MODEL_CACHE_KEY);
        } catch {
          // Ignore storage access errors.
        }
        for (const p of ["gemini", "chatGpt", "claude"] as Provider[]) {
          invalidateProviderCache(p);
        }
      }
      const res = await invoke<CleanupOutcome>("clear_storage", { target: item.target });
      currentStats = res.stats;
      status.textContent =
        res.removedFiles > 0
          ? `Đã xoá ${res.removedFiles} tệp (giải phóng ${formatStorageSize(res.freedBytes)}).`
          : "Đã dọn sạch (không còn tệp nào cần xoá).";
    } catch (e) {
      status.className = "provider-status status-inline bad";
      status.textContent = `Chưa xoá được: ${e}`;
    } finally {
      busyTarget = null;
      draw();
    }
  };

  const draw = () => {
    if (!currentStats) {
      listEl.replaceChildren(el("p", { className: "hint", textContent: "Đang tính dung lượng dữ liệu…" }));
      return;
    }
    const stats = currentStats;
    listEl.replaceChildren(
      ...ITEMS.map((item) => {
        const bucket = stats[item.target];
        const metaText =
          bucket.files > 0
            ? `${bucket.files} tệp · ${formatStorageSize(bucket.bytes)}`
            : "Trống (0 tệp)";
        const isWorking = busyTarget === item.target;
        return el(
          "div",
          { className: "storage-card" },
          el(
            "div",
            { className: "storage-info" },
            el(
              "div",
              { className: "storage-head" },
              el("strong", { textContent: item.title }),
              el("span", { className: "storage-meta", textContent: metaText }),
            ),
            el("p", { className: "storage-desc", textContent: item.desc }),
          ),
          el("button", {
            type: "button",
            className: item.danger ? "btn-ghost danger-btn" : "btn-ghost",
            textContent: isWorking ? "Đang xoá…" : item.btnLabel,
            disabled: Boolean(busyTarget) || (item.target !== "cache" && bucket.files === 0),
            onclick: () => void runClear(item),
          }),
        );
      }),
    );
  };

  const refresh = async () => {
    try {
      currentStats = await invoke<StorageStats>("storage_stats");
    } catch (e) {
      status.className = "provider-status status-inline bad";
      status.textContent = `Không đọc được dung lượng: ${e}`;
      currentStats = {
        cache: { files: 0, bytes: 0 },
        cleaned: { files: 0, bytes: 0 },
        diagrams: { files: 0, bytes: 0 },
        transcripts: { files: 0, bytes: 0 },
      };
    }
    draw();
  };

  host.append(
    el(
      "div",
      { className: "hf-head" },
      el("h3", { textContent: "Dữ liệu & dọn dẹp" }),
      el("button", {
        type: "button",
        className: "link",
        textContent: "Mở thư mục kết quả",
        onclick: () => void invoke("open_output_dir"),
      }),
    ),
    listEl,
    status,
  );
  draw();
  void refresh();
  return host;
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
