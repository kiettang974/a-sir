//! Shared IPC DTO types and tiny UI primitives reused across `main.ts`, `note.ts`, and `settings.ts`.

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import { el } from "./dom";
import { icon } from "./icons";

export type Provider = "claude" | "chatGpt" | "gemini";
export const PROVIDERS: Provider[] = ["claude", "chatGpt", "gemini"];
export const PROVIDER_NAME: Record<Provider, string> = {
  claude: "Claude",
  chatGpt: "ChatGPT",
  gemini: "Gemini",
};

export type SummaryStyle =
  | "workEmail"
  | "administrativeNotice"
  | "casualShare"
  | "productIntro"
  | "howToGuide"
  | "communityNewsletter";

export type SummaryStyleOption = {
  id: SummaryStyle;
  label: string;
  description: string;
  before?: string;
  after?: string;
};

export type AiView = {
  defaultProvider: Provider;
  models: Partial<Record<Provider, string>>;
  keys: Partial<Record<Provider, string>>;
  summaryStyle?: SummaryStyle;
  autoFallback?: boolean;
  recommendedModels?: Partial<Record<Provider, string[]>>;
  summaryStyles?: SummaryStyleOption[];
};

export type MeetingTemplates = {
  enabled?: boolean;
  excel: string | null;
  docx: string | null;
  excelColumns?: string[];
  docxColumns?: string[];
};

export type LocalModelProgress = {
  modelId: string;
  percent: number;
};

export function progressBar(percent: number): HTMLElement {
  const fill = el("i");
  fill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
  return el("div", { className: "bar" }, fill);
}

export type MenuAction = {
  label: string;
  title?: string;
  danger?: boolean;
  disabled?: boolean;
  onClick: () => void;
};

let menuDismissBound = false;
function ensureMenuDismissListeners() {
  if (menuDismissBound) return;
  menuDismissBound = true;
  document.addEventListener("click", (e) => {
    const target = e.target as Node | null;
    for (const openDetails of document.querySelectorAll<HTMLDetailsElement>("details.more-menu[open]")) {
      if (!target || !openDetails.contains(target)) openDetails.open = false;
    }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      for (const openDetails of document.querySelectorAll<HTMLDetailsElement>("details.more-menu[open]")) {
        openDetails.open = false;
      }
    }
  });
}

/** Compact overflow menu (`⋯`) for secondary toolbar actions; owns its outside-click & Escape dismissal. */
export function overflowMenu(ariaLabel: string, items: MenuAction[]): HTMLElement {
  ensureMenuDismissListeners();
  const details = el("details", { className: "more-menu" }) as HTMLDetailsElement;
  const summary = el("summary", { title: ariaLabel }, icon("more", 16));
  summary.setAttribute("aria-label", ariaLabel);
  const panel = el(
    "div",
    { className: "more-menu-panel" },
    ...items.map((item) =>
      el("button", {
        className: item.danger ? "more-menu-item danger" : "more-menu-item",
        textContent: item.label,
        title: item.title ?? "",
        disabled: item.disabled ?? false,
        onclick: () => {
          details.open = false;
          item.onClick();
        },
      }),
    ),
  );
  details.append(summary, panel);
  return details;
}

/** Shared template import dialog + IPC wrapper used by both Settings and Note panes. */
export async function pickAndImportTemplate(): Promise<MeetingTemplates | null> {
  const path = await open({
    filters: [{ name: "Word hoặc Excel", extensions: ["xlsx", "docx"] }],
  });
  if (!path || Array.isArray(path)) return null;
  return invoke<MeetingTemplates>("import_meeting_template", { path });
}

export function resetTemplate(kind: "excel" | "docx"): Promise<MeetingTemplates> {
  return invoke<MeetingTemplates>("reset_meeting_template", { kind });
}

export function setTemplatesEnabled(enabled: boolean): Promise<MeetingTemplates> {
  return invoke<MeetingTemplates>("set_templates_enabled", { enabled });
}

/** Notifies the queue sidebar immediately when a custom model is added so labels never stay stale. */
export function notifyModelLabelAdded(id: string, label: string) {
  window.dispatchEvent(new CustomEvent("asr-model-label", { detail: { id, label } }));
}
