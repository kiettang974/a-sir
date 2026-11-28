//! First-run 2-step onboarding:
//! 1. Pick and download a local speech-to-text model suited to this machine's RAM.
//! 2. Optionally configure an AI API key (Gemini / ChatGPT / Claude) for Clean & Summary.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { el } from "./dom";
import {
  type AiView,
  type LocalModelProgress,
  PROVIDER_NAME,
  PROVIDERS,
  type Provider,
  progressBar,
} from "./ipc";

type SystemInfo = {
  os: string;
  arch: string;
  chip: string;
  ramGb: number;
  cpuCores: number;
};

type LocalModelItem = {
  id: string;
  name: string;
  description: string;
  bytes: number;
  ready: boolean;
  selected: boolean;
  minRamGb?: number;
  compatible?: boolean;
};

type LocalModelsView = {
  selected: string;
  system?: SystemInfo;
  models: LocalModelItem[];
};

type TestConnectionView = {
  ok: boolean;
  message: string;
};

const CURATED_ORDER = [
  "whisper-large-v3-turbo-q5_0",
  "whisper-large-v3-turbo",
  "whisper-large-v3",
  "phowhisper-medium-q5_0",
];

const SHORT_PITCH: Record<string, string> = {
  "whisper-large-v3-turbo-q5_0": "Nhanh, nhẹ RAM — hợp máy 8 GB & họp dài",
  "whisper-large-v3-turbo": "Cân bằng tốc độ và độ chính xác Anh–Việt",
  "whisper-large-v3": "Chính xác nhất cho thuật ngữ khó (cần ≥ 16 GB RAM)",
  "phowhisper-medium-q5_0": "Chuyên tiếng Việt, nhẹ máy",
};

const PROVIDER_KEY_URL: Record<Provider, string> = {
  gemini: "https://aistudio.google.com/apikey",
  chatGpt: "https://platform.openai.com/api-keys",
  claude: "https://console.anthropic.com/settings/keys",
};

const PROVIDER_ORDER: Provider[] = ["gemini", "chatGpt", "claude"];

function formatSize(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  return `${Math.round(bytes / 1_000_000)} MB`;
}

function pickRecommendedId(view: LocalModelsView): string {
  const ram = view.system?.ramGb ?? 8;
  const preferred = ram <= 8 ? "whisper-large-v3-turbo-q5_0" : "whisper-large-v3-turbo";
  if (view.models.some((m) => m.id === preferred && m.compatible !== false)) {
    return preferred;
  }
  const firstCompat = view.models.find((m) => m.compatible !== false);
  return firstCompat?.id ?? view.selected;
}

function curatedModels(view: LocalModelsView): LocalModelItem[] {
  const compat = view.models.filter((m) => m.compatible !== false);
  const picked: LocalModelItem[] = [];
  for (const id of CURATED_ORDER) {
    const found = compat.find((m) => m.id === id);
    if (found) picked.push(found);
  }
  if (picked.length > 0) return picked;
  return compat.slice(0, 4);
}

export function onboardingPane(onComplete: () => void): HTMLElement {
  const host = el("section", { className: "invite onboarding-host" });
  const card = el("div", { className: "onboarding" });
  host.append(card);

  let step: 1 | 2 = 1;
  let modelsData: LocalModelsView | null = null;
  let aiData: AiView | null = null;
  let chosenModelId = "whisper-large-v3-turbo-q5_0";
  let recommendedId = "whisper-large-v3-turbo-q5_0";
  let downloading: { modelId: string; percent: number } | null = null;
  let downloadError = "";

  let chosenProvider: Provider = "gemini";
  let keyInputValue = "";
  let testingKey = false;
  let keyVerified = false;
  let keyStatus = "";
  let keyStatusBad = false;

  let unlistenProgress: (() => void) | null = null;
  const cleanup = () => {
    unlistenProgress?.();
    unlistenProgress = null;
  };

  const observer = new MutationObserver(() => {
    if (!host.isConnected) {
      cleanup();
      observer.disconnect();
    }
  });
  queueMicrotask(() => {
    if (host.parentNode) observer.observe(host.parentNode, { childList: true });
  });

  listen<LocalModelProgress>("local-model-progress", ({ payload }) => {
    if (!downloading || downloading.modelId !== payload.modelId) return;
    if (downloading.percent !== payload.percent) {
      downloading = { ...downloading, percent: payload.percent };
      draw();
    }
  })
    .then((fn) => {
      if (!host.isConnected) fn();
      else unlistenProgress = fn;
    })
    .catch(() => {});

  const hasAnyAiKey = () =>
    Boolean(aiData && PROVIDERS.some((p) => Boolean(aiData?.keys[p])));

  const stepper = () =>
    el(
      "div",
      { className: "onboarding-steps", role: "list" },
      el(
        "span",
        { className: `onboarding-step${step === 1 ? " active" : " done"}` },
        "1. Chọn model chuyển văn bản",
      ),
      el(
        "span",
        { className: `onboarding-step${step === 2 ? " active" : ""}` },
        "2. API key AI",
      ),
    );

  const startModelInstall = async () => {
    if (downloading) return;
    downloadError = "";
    const model = modelsData?.models.find((m) => m.id === chosenModelId);
    if (model?.ready) {
      try {
        await invoke("select_local_model", { modelId: chosenModelId });
        if (hasAnyAiKey()) {
          cleanup();
          onComplete();
        } else {
          step = 2;
          draw();
        }
      } catch (e) {
        downloadError = String(e);
        draw();
      }
      return;
    }

    downloading = { modelId: chosenModelId, percent: 0 };
    draw();
    try {
      await invoke("download_local_model", { modelId: chosenModelId });
      await invoke("select_local_model", { modelId: chosenModelId });
      downloading = null;
      if (hasAnyAiKey()) {
        cleanup();
        onComplete();
      } else {
        step = 2;
        draw();
      }
    } catch (e) {
      downloading = null;
      downloadError = String(e);
      draw();
    }
  };

  const saveAndVerifyKey = async () => {
    const trimmed = keyInputValue.trim();
    if (!trimmed) {
      keyStatus = "Nhập API key trước khi lưu.";
      keyStatusBad = true;
      draw();
      return;
    }
    testingKey = true;
    keyStatus = "Đang kiểm tra kết nối…";
    keyStatusBad = false;
    draw();
    try {
      await invoke("set_api_key", { provider: chosenProvider, key: trimmed });
      const freshAi = await invoke<AiView>("ai_settings");
      aiData = freshAi;
      const model = freshAi.models[chosenProvider] ?? "";
      await invoke("save_ai_settings", {
        settings: {
          defaultProvider: chosenProvider,
          models: freshAi.models,
          summaryStyle: freshAi.summaryStyle ?? "workEmail",
          autoFallback: freshAi.autoFallback ?? true,
        },
      });
      const res = await invoke<TestConnectionView>("test_provider", {
        provider: chosenProvider,
        model,
      });
      testingKey = false;
      keyVerified = res.ok;
      keyStatus = res.ok ? `Đã lưu và kết nối ${PROVIDER_NAME[chosenProvider]}.` : res.message;
      keyStatusBad = !res.ok;
    } catch (e) {
      testingKey = false;
      keyVerified = false;
      keyStatus = String(e);
      keyStatusBad = true;
    }
    draw();
  };

  const drawStep1 = () => {
    if (!modelsData) {
      card.replaceChildren(stepper(), el("h2", { textContent: "Đang kiểm tra cấu hình máy…" }));
      return;
    }
    const sys = modelsData.system;
    const hwLine = sys
      ? `${sys.chip || sys.os} · ${sys.ramGb} GB RAM`
      : null;
    const list = curatedModels(modelsData);
    const activeModel = list.find((m) => m.id === chosenModelId) ?? list[0];
    if (activeModel) chosenModelId = activeModel.id;

    const modelCards = list.map((m) => {
      const selected = m.id === chosenModelId;
      const isRec = m.id === recommendedId;
      return el(
        "button",
        {
          type: "button",
          className: `onboarding-model${selected ? " selected" : ""}`,
          disabled: Boolean(downloading),
          onclick: () => {
            chosenModelId = m.id;
            downloadError = "";
            draw();
          },
        },
        el(
          "div",
          { className: "onboarding-model-top" },
          el("strong", { textContent: m.name }),
          el(
            "div",
            { className: "onboarding-model-meta" },
            isRec ? el("span", { className: "onboarding-badge", textContent: "Khuyên dùng" }) : null,
            m.ready ? el("span", { className: "onboarding-badge ready", textContent: "Đã tải" }) : null,
            el("span", { className: "onboarding-size", textContent: formatSize(m.bytes) }),
          ),
        ),
        el("span", {
          className: "onboarding-model-desc",
          textContent: SHORT_PITCH[m.id] ?? m.description,
        }),
      );
    });

    const progressNode = downloading
      ? el(
          "div",
          { className: "onboarding-progress" },
          el("span", {
            className: "onboarding-progress-label",
            textContent: `Đang tải ${activeModel?.name ?? ""} · ${downloading.percent}%`,
          }),
          progressBar(downloading.percent),
        )
      : null;

    const nodes: (HTMLElement | null)[] = [
      stepper(),
      el("h2", { textContent: "Chọn model chuyển văn bản" }),
      hwLine ? el("p", { className: "onboarding-hw", textContent: `Máy của bạn: ${hwLine}` }) : null,
      el("div", { className: "onboarding-models", role: "radiogroup", ariaLabel: "Chọn model chuyển văn bản" }, ...modelCards),
      downloadError ? el("p", { className: "problem problem-text", textContent: downloadError }) : null,
      progressNode,
      !downloading
        ? el(
            "div",
            { className: "onboarding-actions" },
            el("button", {
              className: "btn-primary",
              textContent: activeModel?.ready
                ? "Dùng model này"
                : downloadError
                  ? "Thử tải lại"
                  : `Tải ${activeModel?.name ?? "model"}`,
              onclick: () => void startModelInstall(),
            }),
          )
        : null,
    ];
    card.replaceChildren(...nodes.filter((n): n is HTMLElement => n !== null));
  };

  const drawStep2 = () => {
    const providerTabs = PROVIDER_ORDER.map((p) =>
      el("button", {
        type: "button",
        className: `onboarding-provider${chosenProvider === p ? " selected" : ""}`,
        textContent: PROVIDER_NAME[p],
        disabled: testingKey,
        onclick: () => {
          chosenProvider = p;
          keyStatus = "";
          keyStatusBad = false;
          keyVerified = false;
          draw();
        },
      }),
    );

    const keyInput = el("input", {
      type: "password",
      className: "field onboarding-key-input",
      value: keyInputValue,
      placeholder: `Dán API key ${PROVIDER_NAME[chosenProvider]}`,
      ariaLabel: `API key ${PROVIDER_NAME[chosenProvider]}`,
      disabled: testingKey,
      oninput: (e: Event) => {
        keyInputValue = (e.target as HTMLInputElement).value;
      },
      onkeydown: (e: KeyboardEvent) => {
        if (e.key === "Enter" && !testingKey) void saveAndVerifyKey();
      },
    }) as HTMLInputElement;

    const step2Nodes: (HTMLElement | null)[] = [
      stepper(),
      el("h2", { textContent: "Thiết lập API key AI" }),
      el("p", {
        className: "onboarding-sub",
        textContent: "Dùng cho Làm sạch và Tóm tắt biên bản. Chuyển văn bản chạy tại máy, không cần key.",
      }),
      el(
        "div",
        { className: "onboarding-providers", role: "tablist", ariaLabel: "Nhà cung cấp AI" },
        ...providerTabs,
        el("button", {
          type: "button",
          className: "link onboarding-key-link",
          textContent: `Lấy key ${PROVIDER_NAME[chosenProvider]} ↗`,
          onclick: () => void invoke("open_url", { url: PROVIDER_KEY_URL[chosenProvider] }),
        }),
      ),
      el("div", { className: "onboarding-key-row" }, keyInput),
      keyStatus
        ? el("p", {
            className: keyStatusBad ? "provider-status bad" : "provider-status",
            textContent: keyStatus,
          })
        : null,
      el(
        "div",
        { className: "onboarding-actions" },
        keyVerified
          ? el("button", {
              className: "btn-primary",
              textContent: "Vào ứng dụng",
              onclick: () => {
                cleanup();
                onComplete();
              },
            })
          : el("button", {
              className: "btn-primary",
              textContent: testingKey ? "Đang kiểm tra…" : "Lưu & kiểm tra",
              disabled: testingKey,
              onclick: () => void saveAndVerifyKey(),
            }),
        el("button", {
          className: "btn-ghost",
          textContent: "Để sau",
          disabled: testingKey,
          onclick: () => {
            cleanup();
            onComplete();
          },
        }),
      ),
    ];
    card.replaceChildren(...step2Nodes.filter((n): n is HTMLElement => n !== null));
  };

  const draw = () => {
    if (step === 1) drawStep1();
    else drawStep2();
  };

  draw();
  Promise.all([
    invoke<LocalModelsView>("local_models"),
    invoke<AiView>("ai_settings").catch(() => null),
  ])
    .then(([lm, ai]) => {
      modelsData = lm;
      aiData = ai;
      recommendedId = pickRecommendedId(lm);
      chosenModelId = recommendedId;
      if (ai?.defaultProvider && PROVIDER_ORDER.includes(ai.defaultProvider)) {
        chosenProvider = ai.defaultProvider;
      }
      draw();
    })
    .catch((e) => {
      downloadError = String(e);
      draw();
    });

  return host;
}
