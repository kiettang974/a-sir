import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { writeHtml } from "@tauri-apps/plugin-clipboard-manager";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { describeContext, type MeetingContextView } from "./context";
import { el } from "./dom";
import { icon } from "./icons";
import {
  type AiView,
  type MeetingTemplates,
  pickAndImportTemplate,
  PROVIDER_NAME,
  PROVIDERS,
  type Provider,
  type RecordFlowNav,
  recordFlowTabs,
  resetTemplate,
} from "./ipc";

type Evidence = { time: string; quote: string; seconds: number };
type Row = {
  title: string;
  agreed: string;
  note: string;
  evidence: Evidence[];
  uncertain: string[];
  fields?: Record<string, string>;
};
type ActionItem = {
  task: string;
  owner?: string | null;
  deadline?: string | null;
  missing?: string[];
  evidence?: Evidence[];
};
type Pending = { row: Row; reason: string };
type Draft = {
  discussion?: string[];
  rows: Row[];
  actions?: ActionItem[];
  pending: Pending[];
  usedModel?: string | null;
  fallbackFrom?: string | null;
};
/** The working copy. Row text is HTML by now, because it is edited in place. */
type Note = Draft & { greeting: string; intro: string };

const SAVE_DELAY_MS = 700;

function escapeHtml(text: string): string {
  const holder = document.createElement("div");
  holder.textContent = text;
  return holder.innerHTML;
}

/** The model writes **bold** and line breaks; everything else from it is plain text. */
function fromModelText(text: string): string {
  return escapeHtml(text)
    .replace(/\*\*(.+?)\*\*/g, "<b>$1</b>")
    .replace(/\n/g, "<br>");
}

function toHtmlRow(row: Row): Row {
  return {
    ...row,
    title: fromModelText(row.title),
    agreed: fromModelText(row.agreed),
    note: fromModelText(row.note),
  };
}

/** Keeps only what an email should carry: bold, italics, line breaks and http(s) links. */
export function cleanHtml(html: string): string {
  const source = document.createElement("template");
  source.innerHTML = html;
  const walk = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return escapeHtml(node.textContent ?? "");
    if (node.nodeType !== Node.ELEMENT_NODE) return "";
    const element = node as Element;
    const inner = [...element.childNodes].map(walk).join("");
    switch (element.tagName) {
      case "B":
      case "STRONG":
        return `<b>${inner}</b>`;
      case "I":
      case "EM":
        return `<i>${inner}</i>`;
      case "BR":
        return "<br>";
      case "A": {
        const href = element.getAttribute("href") ?? "";
        return /^https?:\/\//i.test(href)
          ? `<a href="${escapeHtml(href).replace(/"/g, "&quot;")}" style="color:#0563C1">${inner}</a>`
          : inner;
      }
      case "DIV":
      case "P":
        return `<br>${inner}`;
      default:
        return inner;
    }
  };
  return [...source.content.childNodes].map(walk).join("").replace(/^(<br>)+/, "");
}

/** What the recipients see: no times, quotes or pending items. Styles are inline for email clients. */
export function emailHtml(note: Note): string {
  const font = "font-family:'Times New Roman',Times,serif;font-size:11pt;";
  const cell = `border:1px solid #d9d9d9;padding:6px 10px;vertical-align:middle;${font}`;
  const head = `background:#2F5D46;color:#ffffff;border:1px solid #2F5D46;padding:6px 10px;text-align:left;${font}`;
  const rows = note.rows
    .map(
      (r, i) =>
        `<tr style="background:${i % 2 ? "#F3F5F7" : "#FFFFFF"}">` +
        `<td style="${cell}">${i + 1}</td><td style="${cell}">${cleanHtml(r.title)}</td>` +
        `<td style="${cell}">${cleanHtml(r.agreed)}</td><td style="${cell}">${cleanHtml(r.note)}</td></tr>`,
    )
    .join("");
  return (
    `<p style="${font}">${cleanHtml(note.greeting)}</p>` +
    `<p style="${font}">${cleanHtml(note.intro)}</p>` +
    `<table style="border-collapse:collapse;width:100%"><thead><tr>` +
    `<th style="${head}">#</th><th style="${head}">Hạng mục</th>` +
    `<th style="${head}">Nội dung thống nhất</th><th style="${head}">Ghi chú</th>` +
    `</tr></thead><tbody>${rows}</tbody></table>`
  );
}

/** Converts edited HTML to plain text (preserving `<br>` as newlines by default). */
function htmlToText(html: string, preserveBreaks = true): string {
  const holder = document.createElement("div");
  const cleaned = cleanHtml(html);
  holder.innerHTML = preserveBreaks ? cleaned.replace(/<br\s*\/?>/gi, "\n") : cleaned;
  return holder.textContent ?? "";
}

function plainText(note: Note): string {
  const rows = note.rows.map(
    (r, i) =>
      `${i + 1}. ${htmlToText(r.title)}\n${htmlToText(r.agreed)}${r.note ? `\nGhi chú: ${htmlToText(r.note)}` : ""}`,
  );
  return [htmlToText(note.greeting), htmlToText(note.intro), ...rows].join("\n\n");
}

function isUrl(text: string): boolean {
  return /^https?:\/\/\S+$/i.test(text.trim());
}

/** A URL pasted over selected text becomes a link; other pastes arrive as plain text. */
function handlePaste(event: ClipboardEvent) {
  event.preventDefault();
  const text = event.clipboardData?.getData("text/plain") ?? "";
  const selection = window.getSelection();
  if (isUrl(text) && selection && !selection.isCollapsed) {
    document.execCommand("createLink", false, text.trim());
  } else if (isUrl(text)) {
    const url = escapeHtml(text.trim());
    document.execCommand("insertHTML", false, `<a href="${url}">${url}</a>`);
  } else {
    document.execCommand("insertText", false, text);
  }
}

function editable(
  html: string,
  className: string,
  label: string,
  onChange: (html: string) => void,
): HTMLElement {
  const node = el("div", {
    className,
    contentEditable: "true",
    ariaLabel: label,
    spellcheck: false,
    onpaste: handlePaste,
    oninput: () => onChange(cleanHtml(node.innerHTML)),
  });
  node.innerHTML = cleanHtml(html);
  return node;
}

function editablePlain(
  text: string,
  className: string,
  label: string,
  onChange: (text: string) => void,
): HTMLElement {
  const node = el("div", {
    className,
    contentEditable: "true",
    ariaLabel: label,
    spellcheck: false,
    onpaste: (event: ClipboardEvent) => {
      event.preventDefault();
      const plain = (event.clipboardData?.getData("text/plain") ?? "").replace(/\r?\n+/g, " ");
      document.execCommand("insertText", false, plain);
    },
    onkeydown: (event: KeyboardEvent) => {
      if (event.key === "Enter") {
        event.preventDefault();
        node.blur();
      }
    },
    oninput: () => onChange((node.textContent ?? "").replace(/\r?\n+/g, " ")),
  });
  node.textContent = text;
  return node;
}

type GlossarySyncResult = {
  draft: Note;
  replacements: number;
  discussionReplacements: number;
};

type SummaryProfile = {
  id: string;
  name: string;
  description: string;
};
type ProfilesConfig = {
  enabled: boolean;
  activeId: string;
  custom: SummaryProfile[];
};

type Links = {
  openSettings: () => void;
  openGlossary: () => void;
  openContext: () => void;
  back: () => void;
  flow: RecordFlowNav;
};

type NoteRunState = {
  running: boolean;
  startedAt: number | null;
  provider: Provider;
  profileId: string;
  useContext: boolean;
  previousNote?: Note;
  error: string | null;
  suggestedModel?: { provider: Provider; model: string };
  completedNote?: Note;
};

const EST_NOTE_SECS = 30;
const noteRuns = new Map<number, NoteRunState>();
const noteJobListeners = new Map<number, Set<() => void>>();
const noteGlobalListeners = new Set<(jobId: number) => void>();

export function isNoteGenerating(id: number): boolean {
  return Boolean(noteRuns.get(id)?.running);
}

export function subscribeNoteRuns(fn: (jobId: number) => void): () => void {
  noteGlobalListeners.add(fn);
  return () => noteGlobalListeners.delete(fn);
}

function notifyNoteRun(id: number) {
  const listeners = noteJobListeners.get(id);
  if (listeners) {
    for (const fn of listeners) fn();
  }
  for (const fn of noteGlobalListeners) fn(id);
}

function subscribeNoteJob(id: number, fn: () => void): () => void {
  let set = noteJobListeners.get(id);
  if (!set) {
    set = new Set();
    noteJobListeners.set(id, set);
  }
  set.add(fn);
  return () => {
    set?.delete(fn);
    if (set?.size === 0) noteJobListeners.delete(id);
  };
}

async function startSummaryRun(
  id: number,
  provider: Provider,
  profileId: string,
  useContext: boolean,
  ai: AiView,
  previousNote?: Note,
): Promise<void> {
  if (noteRuns.get(id)?.running) return;
  const run: NoteRunState = {
    running: true,
    startedAt: Date.now(),
    provider,
    profileId,
    useContext,
    previousNote,
    error: null,
    suggestedModel: undefined,
    completedNote: undefined,
  };
  noteRuns.set(id, run);
  notifyNoteRun(id);

  try {
    const draft = await invoke<Draft>("summarize_job", { id, provider, profileId, useContext });
    const note: Note = {
      greeting: previousNote?.greeting ?? "",
      intro: previousNote?.intro ?? "",
      discussion: draft.discussion ?? [],
      rows: draft.rows.map(toHtmlRow),
      actions: draft.actions ?? [],
      pending: draft.pending.map((p) => ({ ...p, row: toHtmlRow(p.row) })),
      usedModel: draft.usedModel ?? null,
      fallbackFrom: draft.fallbackFrom ?? null,
    };
    await invoke<string>("save_note", {
      id,
      draft: note,
      html: `<!doctype html><html lang="vi"><meta charset="utf-8"><body>${emailHtml(note)}</body></html>`,
    }).catch(() => {});
    run.completedNote = note;
    run.error = null;
  } catch (e) {
    const errText = String(e);
    let suggestedModel: { provider: Provider; model: string } | undefined;
    let finalError = errText;
    if (errText.includes("404") || errText.includes("không tồn tại") || errText.includes("ngừng hoạt động")) {
      try {
        const check = await invoke<{ ok: boolean; suggestion: string | null }>("validate_saved_model", {
          provider,
          model: ai.models[provider] ?? null,
        });
        if (!check.ok && check.suggestion) {
          suggestedModel = { provider, model: check.suggestion };
          finalError = `${errText} Gợi ý chuyển sang ${check.suggestion}.`;
        }
      } catch {
        // Fall through to normal error display.
      }
    }
    run.error = finalError;
    run.suggestedModel = suggestedModel;
  } finally {
    run.running = false;
    run.startedAt = null;
    notifyNoteRun(id);
  }
}

/** Fills itself in and keeps its own state, so queue updates can't disturb typing here. */
export function notePane(id: number, name: string, links: Links): HTMLElement {
  const head = el("header", { className: "reader-head" });
  const tabs = recordFlowTabs("note", links.flow);
  const scroll = el("div", { className: "note-scroll" });
  const player = el("div", { className: "player" });
  player.hidden = true;
  const audio = el("audio", { controls: true, preload: "metadata" });
  const problem = el("span", { className: "problem" });
  player.append(audio, problem);
  const pane = el("section", { className: "reader" }, head, tabs, scroll, player);

  let runningTimer: number | null = null;
  const stopRunningTimer = () => {
    if (runningTimer !== null) {
      window.clearInterval(runningTimer);
      runningTimer = null;
    }
  };

  let pendingSeek: number | null = null;

  const seekAndPlay = (seconds: number) => {
    const target = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
    if (audio.readyState >= 1) {
      const clamped = Number.isFinite(audio.duration) ? Math.min(target, audio.duration) : target;
      audio.currentTime = clamped;
      audio.play().catch(() => {});
    } else {
      pendingSeek = target;
      audio.load();
    }
  };

  audio.onloadedmetadata = () => {
    audio.hidden = false;
    problem.textContent = "";
    if (pendingSeek !== null) {
      const target = pendingSeek;
      pendingSeek = null;
      const clamped = Number.isFinite(audio.duration) ? Math.min(target, audio.duration) : target;
      audio.currentTime = clamped;
      audio.play().catch(() => {});
    }
  };

  audio.onerror = () => {
    pendingSeek = null;
    audio.hidden = true;
    const code = audio.error?.code;
    problem.textContent =
      code === 2
        ? "Lỗi kết nối nội bộ khi đọc file ghi âm."
        : code === 3
          ? "Không giải mã được file ghi âm này."
          : "Không phát được file ghi âm này. File có thể đã bị chuyển đi hoặc ở định dạng trình phát không hỗ trợ.";
  };

  const play = async (seconds: number) => {
    player.hidden = false;
    problem.textContent = "";
    audio.hidden = false;
    if (!audio.getAttribute("src")) {
      try {
        const src = await invoke<string>("allow_job_audio", { id });
        pendingSeek = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
        audio.src = src.startsWith("http://") || src.startsWith("https://") ? src : convertFileSrc(src);
        return;
      } catch (e) {
        audio.hidden = true;
        problem.textContent = String(e);
        return;
      }
    }
    seekAndPlay(seconds);
  };

  function showRunning(run: NoteRunState) {
    stopRunningTimer();
    head.replaceChildren(el("div", { className: "reader-head-top" }, el("h2", { textContent: name })));
    const fill = el("i");
    const bar = el("div", { className: "bar" }, fill);
    const meta = el("p", { className: "diagram-loading-meta" });
    const tick = () => {
      const started = run.startedAt ?? Date.now();
      const elapsedS = Math.max(0, Math.floor((Date.now() - started) / 1000));
      const pct = Math.min(94, Math.max(8, Math.round((elapsedS / EST_NOTE_SECS) * 90) + 6));
      fill.style.width = `${pct}%`;
      const remain = EST_NOTE_SECS - elapsedS;
      meta.textContent =
        remain > 2 ? `${elapsedS}s · Ước tính còn ~${remain}s` : `${elapsedS}s · Sắp hoàn tất…`;
    };
    tick();
    runningTimer = window.setInterval(() => {
      if (!pane.isConnected || !noteRuns.get(id)?.running) {
        stopRunningTimer();
        return;
      }
      tick();
    }, 1000);

    const body = el(
      "div",
      { className: "note-form" },
      el(
        "div",
        { className: "ai-loading-stage", role: "status", ariaLive: "polite" },
        el("div", { className: "diagram-spinner", ariaHidden: "true" }),
        el("p", { className: "diagram-loading-title", textContent: "Đang tạo biên bản cuộc họp…" }),
        el("p", {
          className: "diagram-loading-sub",
          textContent: `Đang chờ ${PROVIDER_NAME[run.provider]}. Bạn có thể chuyển sang tab khác trong lúc chờ — biên bản sẽ tự lưu khi xong.`,
        }),
        el("div", { className: "diagram-loading-bar" }, bar),
        meta,
        run.previousNote
          ? el("button", {
              className: "link",
              textContent: "Xem lại bản nháp cũ trong lúc chờ",
              onclick: () => showNote(run.previousNote!),
            })
          : null,
      ),
    );
    scroll.replaceChildren(body);
  }

  function showForm(
    failure = "",
    previousNote?: Note,
    selectedProfileId?: string,
    selectedUseContext?: boolean,
    suggestedModel?: { provider: Provider; model: string },
  ) {
    stopRunningTimer();
    head.replaceChildren(el("div", { className: "reader-head-top" }, el("h2", { textContent: name })));
    const body = el("div", { className: "note-form" });
    scroll.replaceChildren(body);
    Promise.all([
      invoke<AiView>("ai_settings"),
      invoke<ProfilesConfig>("summary_profiles").catch(
        (): ProfilesConfig => ({ enabled: true, activeId: "ba-open-kit", custom: [] }),
      ),
      invoke<MeetingContextView>("get_meeting_context", { id }).catch(
        (): MeetingContextView => ({
          notes: "",
          attachments: [],
          maxFileBytes: 15 * 1024 * 1024,
          maxTotalBytes: 40 * 1024 * 1024,
          acceptedExtensions: ["png", "jpg", "jpeg", "pdf", "txt", "md"],
        }),
      ),
    ])
      .then(([ai, profiles, ctx]) =>
        drawForm(body, ai, profiles, ctx, failure, previousNote, selectedProfileId, selectedUseContext, suggestedModel),
      )
      .catch((e) => body.replaceChildren(el("p", { className: "problem problem-text", textContent: String(e) })));
  }

  function drawForm(
    body: HTMLElement,
    ai: AiView,
    profiles: ProfilesConfig,
    ctx: MeetingContextView,
    failure: string,
    previousNote?: Note,
    selectedProfileId?: string,
    selectedUseContext?: boolean,
    suggestedModel?: { provider: Provider; model: string },
  ) {
    const usable = PROVIDERS.filter((p) => ai.keys[p]);
    if (usable.length === 0) {
      body.replaceChildren(
        el("h3", { textContent: "Chưa có tài khoản AI để tóm tắt" }),
        el("p", {
          className: "hint",
          textContent: "Vào Cấu hình → Tài khoản AI để thêm key Claude, ChatGPT hoặc Gemini.",
        }),
        el("button", { className: "add", textContent: "Mở Cấu hình", onclick: links.openSettings }),
      );
      return;
    }
    const initial = usable.includes(ai.defaultProvider) ? ai.defaultProvider : usable[0];
    const select = el(
      "select",
      { className: "field" },
      ...usable.map((p) =>
        el("option", { value: p, selected: p === initial, textContent: `${PROVIDER_NAME[p]} (${ai.models[p]})` }),
      ),
    );
    const defaultProfile = selectedProfileId ?? (profiles.enabled ? profiles.activeId : "__disabled__");
    const profileOptions: { value: string; label: string }[] = [
      { value: "ba-open-kit", label: "BA (Mặc định)" },
      ...profiles.custom.map((p) => ({ value: p.id, label: p.name })),
      { value: "__disabled__", label: "Không dùng profile (tóm tắt tự do)" },
    ];
    const validProfile = profileOptions.some((o) => o.value === defaultProfile)
      ? defaultProfile
      : "ba-open-kit";
    const profileSelect = el(
      "select",
      { className: "field" },
      ...profileOptions.map((o) =>
        el("option", { value: o.value, selected: o.value === validProfile, textContent: o.label }),
      ),
    );
    const ctxInfo = describeContext(ctx);
    const useCtxBox = el("input", {
      type: "checkbox",
      checked: selectedUseContext ?? ctxInfo.hasAny,
      disabled: !ctxInfo.hasAny,
    }) as HTMLInputElement;
    const label = () => `Tạo tóm tắt bằng ${PROVIDER_NAME[select.value as Provider]}`;
    const notice = () => {
      const pName = PROVIDER_NAME[select.value as Provider];
      const parts = ["văn bản", "từ vựng riêng"];
      if (useCtxBox.checked && ctxInfo.hasAny) {
        if (ctx.notes.trim()) parts.push("ghi chú tài liệu kèm");
        if (ctx.attachments.length > 0) parts.push(`${ctx.attachments.length} tệp đính kèm`);
      }
      return `Nội dung ${parts.join(", ")} sẽ được gửi đến ${pName}.`;
    };
    const sending = el("p", { className: "sending", textContent: notice() });
    const start = el("button", { className: "add", textContent: label() });
    const syncNotice = () => {
      start.textContent = label();
      sending.textContent = notice();
    };
    select.onchange = syncNotice;
    useCtxBox.onchange = syncNotice;
    start.onclick = () => {
      const provider = select.value as Provider;
      const profileId = profileSelect.value;
      const useContext = useCtxBox.checked && ctxInfo.hasAny;
      void startSummaryRun(id, provider, profileId, useContext, ai, previousNote);
    };
    const switchBtn =
      suggestedModel
        ? el("button", {
            className: "btn-primary",
            textContent: `Đổi sang ${suggestedModel.model} và thử lại`,
            onclick: async () => {
              switchBtn!.disabled = true;
              const nextModels = { ...ai.models, [suggestedModel.provider]: suggestedModel.model };
              await invoke("save_ai_settings", {
                settings: {
                  defaultProvider: ai.defaultProvider,
                  models: nextModels,
                  summaryStyle: ai.summaryStyle ?? "workEmail",
                },
              });
              showForm("", previousNote, profileSelect.value, useCtxBox.checked);
            },
          })
        : null;

    const hasCustomProfiles = profiles.custom.length > 0;
    const formChildren: (HTMLElement | null)[] = [
      el("h3", { textContent: "Tạo biên bản cuộc họp" }),
      failure ? el("p", { className: "problem problem-text", textContent: failure }) : null,
      switchBtn ? el("div", { className: "actions wrap" }, switchBtn) : null,
      el("label", {}, "AI thực hiện", select),
      hasCustomProfiles ? el("label", {}, "Mẫu bố cục", profileSelect) : null,
      ctxInfo.hasAny
        ? el(
            "div",
            { className: "context-summary-box" },
            el(
              "label",
              { className: "hw-filter-toggle" },
              useCtxBox,
              el("span", { textContent: "Dùng tài liệu kèm" }),
            ),
            el("span", { className: "style-desc", textContent: ctxInfo.label }),
            el("button", {
              className: "link",
              textContent: "Xem tài liệu kèm",
              onclick: links.openContext,
            }),
          )
        : null,
      el(
        "div",
        { className: "actions wrap" },
        start,
        previousNote
          ? el("button", {
              className: "link",
              textContent: "Quay lại bản nháp đã lưu",
              onclick: () => showNote(previousNote),
            })
          : null,
      ),
    ];
    body.replaceChildren(...formChildren.filter((n): n is HTMLElement => n !== null));
  }

  let savedPage = "";
  let timer: number | undefined;
  let templates: MeetingTemplates = { excel: null, docx: null };
  const status = el("span", { className: "note-status" });
  const setStatus = (msg: string, bad = false) => {
    status.textContent = msg;
    status.className = bad ? "note-status status-bad" : "note-status";
  };

  async function save(note: Note) {
    clearTimeout(timer);
    try {
      savedPage = await invoke<string>("save_note", { id, draft: note, html: `<!doctype html><html lang="vi"><meta charset="utf-8"><body>${emailHtml(note)}</body></html>` });
      setStatus("Đã lưu nháp");
    } catch (e) {
      setStatus(String(e), true);
    }
  }

  function showNote(note: Note, autoSyncGlossary = false) {
    let showEmailIntro =
      htmlToText(note.greeting, false).trim().length > 0 ||
      htmlToText(note.intro, false).trim().length > 0;
    let focusDiscussionIndex: number | null = null;
    const saveSoon = () => {
      setStatus("");
      clearTimeout(timer);
      timer = window.setTimeout(() => save(note), SAVE_DELAY_MS);
    };
    const changed = () => {
      save(note);
      draw();
    };
    const syncGlossary = async (announceWhenNone: boolean) => {
      try {
        const res = await invoke<GlossarySyncResult>("sync_note_glossary", { draft: note });
        if (!pane.isConnected) return;
        if (res.replacements > 0) {
          note.greeting = res.draft.greeting ?? note.greeting;
          note.intro = res.draft.intro ?? note.intro;
          note.discussion = res.draft.discussion ?? note.discussion;
          note.rows = res.draft.rows ?? note.rows;
          note.actions = res.draft.actions ?? note.actions;
          note.pending = res.draft.pending ?? note.pending;
          await save(note);
          draw();
          const detail =
            res.discussionReplacements > 0
              ? `Đã đồng bộ ${res.replacements} từ vựng (${res.discussionReplacements} trong Nội dung trao đổi chính).`
              : `Đã đồng bộ ${res.replacements} từ vựng trong biên bản.`;
          setStatus(detail);
        } else if (announceWhenNone) {
          setStatus("Biên bản đã khớp với Từ vựng riêng.");
        }
      } catch (e) {
        if (announceWhenNone) setStatus(`Chưa đồng bộ được từ vựng: ${e}`, true);
      }
    };
    invoke<MeetingTemplates>("meeting_templates")
      .then((t) => {
        templates = t;
        draw();
      })
      .catch(() => {});

    const exportRows = () =>
      note.rows.map((r) => ({
        title: htmlToText(r.title),
        agreed: htmlToText(r.agreed),
        note: htmlToText(r.note),
        fields: r.fields ?? {},
      }));

    head.replaceChildren(
      el("div", { className: "reader-head-top" }, el("h2", { textContent: name }), status),
      el(
        "div",
        { className: "reader-actions" },
        el(
          "button",
          {
            className: "btn-primary",
            title: "Chọn lại cấu hình và tạo bản tóm tắt mới",
            onclick: () => showForm("", note),
          },
          icon("summary", 15),
          el("span", { textContent: "Tạo lại tóm tắt" }),
        ),
        el(
          "button",
          {
            className: "btn-ghost",
            title: "Áp dụng Từ vựng riêng vào Nội dung trao đổi chính và toàn bộ biên bản",
            onclick: () => void syncGlossary(true),
          },
          icon("sync", 15),
          el("span", { textContent: "Đồng bộ từ vựng" }),
        ),
        el(
          "button",
          {
            className: "btn-ghost",
            title: "Chép bảng để dán vào email",
            onclick: async () => {
              await save(note);
              await writeHtml(emailHtml(note), plainText(note));
              const left = note.pending.length;
              setStatus(
                left
                  ? `Đã chép bảng. Còn ${left} mục cần xác nhận chưa đưa vào.`
                  : "Đã chép bảng.",
              );
            },
          },
          icon("copy", 15),
          el("span", { textContent: "Chép email" }),
        ),
        el(
          "button",
          {
            className: "btn-ghost",
            title: "Xuất file Excel (.xlsx)",
            onclick: async () => {
              await save(note);
              const safeName = name.replace(/[/:\\]/g, "-").trim() || "bien-ban-hop";
              const path = await saveDialog({
                defaultPath: `${safeName}.xlsx`,
                filters: [{ name: "Excel", extensions: ["xlsx"] }],
              });
              if (!path) return;
              try {
                const rows = exportRows();
                const saved = await invoke<string>("export_note_excel", { rows, path });
                const left = note.pending.length;
                setStatus(
                  left
                    ? `Đã xuất Excel (${rows.length} mục đã chốt; ${left} mục cần xác nhận không xuất).`
                    : `Đã xuất Excel (${rows.length} mục).`,
                );
                revealItemInDir(saved).catch(() => {});
              } catch (e) {
                setStatus(`Xuất Excel thất bại: ${e}`, true);
              }
            },
          },
          icon("table", 15),
          el("span", { textContent: "Excel" }),
        ),
        el(
          "button",
          {
            className: "btn-ghost",
            title: "Xuất file Word (.docx)",
            onclick: async () => {
              await save(note);
              const safeName = name.replace(/[/:\\]/g, "-").trim() || "bien-ban-hop";
              const path = await saveDialog({
                defaultPath: `${safeName}.docx`,
                filters: [{ name: "Word", extensions: ["docx"] }],
              });
              if (!path) return;
              try {
                const rows = exportRows();
                const saved = await invoke<string>("export_note_docx", {
                  title: name,
                  greeting: htmlToText(note.greeting),
                  intro: htmlToText(note.intro),
                  rows,
                  path,
                });
                const left = note.pending.length;
                setStatus(
                  left
                    ? `Đã xuất Word (${rows.length} mục đã chốt; ${left} mục cần xác nhận không xuất).`
                    : `Đã xuất Word (${rows.length} mục).`,
                );
                revealItemInDir(saved).catch(() => {});
              } catch (e) {
                setStatus(`Xuất Word thất bại: ${e}`, true);
              }
            },
          },
          icon("fileText", 15),
          el("span", { textContent: "Word" }),
        ),
        el(
          "button",
          {
            className: "btn-ghost",
            title: "Mở thư mục chứa file biên bản đã lưu",
            onclick: async () => {
              await save(note);
              if (savedPage) revealItemInDir(savedPage);
            },
          },
          icon("folder", 15),
          el("span", { textContent: "Mở thư mục" }),
        ),
      ),
    );

    const reviewBits = (row: Row) => [
      ...row.evidence.map((e) =>
        el(
          "div",
          { className: "evidence" },
          el(
            "button",
            { className: "time", title: "Nghe lại từ đoạn này", onclick: () => play(e.seconds) },
            icon("playSm", 11),
            el("span", { textContent: e.time }),
          ),
          el("span", { textContent: `“${e.quote}”` }),
        ),
      ),
      row.uncertain.length
        ? el("div", { className: "check", textContent: `Kiểm tra lại: ${row.uncertain.join(", ")}` })
        : "",
    ];

    function draw() {
      const top = scroll.scrollTop;
      const tbody = el("tbody");
      note.rows.forEach((row, index) => {
        tbody.append(
          el(
            "tr",
            {},
            el("td", { className: "num", textContent: String(index + 1) }),
            el("td", {}, editable(row.title, "cell", "Hạng mục", (h) => { row.title = h; saveSoon(); })),
            el("td", {}, editable(row.agreed, "cell", "Nội dung thống nhất", (h) => { row.agreed = h; saveSoon(); })),
            el("td", {}, editable(row.note, "cell", "Ghi chú", (h) => { row.note = h; saveSoon(); })),
          ),
          el(
            "tr",
            { className: "review" },
            el(
              "td",
              { colSpan: 4 },
              ...reviewBits(row),
              el(
                "div",
                { className: "row-actions" },
                el("button", {
                  textContent: "Chuyển sang Cần xác nhận",
                  onclick: () => {
                    note.rows.splice(index, 1);
                    note.pending.push({ row, reason: "Bạn đã chuyển mục này xuống" });
                    changed();
                  },
                }),
                el("button", {
                  textContent: "Xoá dòng",
                  onclick: () => {
                    note.rows.splice(index, 1);
                    changed();
                  },
                }),
              ),
            ),
          ),
        );
      });
      const pending = note.pending.map((item, index) =>
        el(
          "div",
          { className: "pending-item" },
          el("b", { innerHTML: cleanHtml(item.row.title) }),
          el("div", { className: "why", textContent: item.reason }),
          el("div", { className: "body", innerHTML: cleanHtml(item.row.agreed) }),
          ...reviewBits(item.row),
          el(
            "div",
            { className: "row-actions" },
            el("button", {
              className: "link",
              textContent: "Chuyển vào bảng",
              onclick: () => {
                note.pending.splice(index, 1);
                note.rows.push(item.row);
                changed();
              },
            }),
            el("button", {
              textContent: "Bỏ mục này",
              onclick: () => {
                note.pending.splice(index, 1);
                changed();
              },
            }),
          ),
        ),
      );
      const discussionItems = note.discussion ?? [];
      const discussionBlock = discussionItems.length
        ? el(
            "section",
            { className: "meeting-summary" },
            el(
              "div",
              { className: "meeting-summary-head" },
              el("h3", { textContent: "Nội dung trao đổi chính" }),
              el(
                "div",
                { className: "meeting-summary-actions" },
                el("button", {
                  className: "link",
                  textContent: "Từ vựng riêng",
                  title: "Mở Cấu hình → Từ vựng riêng để thêm hoặc điều chỉnh thuật ngữ",
                  onclick: links.openGlossary,
                }),
                el("button", {
                  className: "link",
                  textContent: "Thêm ý",
                  onclick: () => {
                    note.discussion = [...(note.discussion ?? []), ""];
                    focusDiscussionIndex = note.discussion.length - 1;
                    changed();
                  },
                }),
              ),
            ),
            el(
              "ul",
              { className: "discussion-list" },
              ...discussionItems.map((item, idx) => {
                const field = editablePlain(
                  item,
                  "discussion-text",
                  `Nội dung trao đổi chính ${idx + 1}`,
                  (val) => {
                    if (!note.discussion) note.discussion = [];
                    note.discussion[idx] = val;
                    saveSoon();
                  },
                );
                if (focusDiscussionIndex === idx) {
                  focusDiscussionIndex = null;
                  queueMicrotask(() => field.focus());
                }
                return el(
                  "li",
                  { className: "discussion-item" },
                  field,
                  el(
                    "button",
                    {
                      className: "icon-btn danger discussion-del",
                      title: "Xoá ý này",
                      ariaLabel: `Xoá ý trao đổi ${idx + 1}`,
                      onclick: () => {
                        note.discussion?.splice(idx, 1);
                        changed();
                      },
                    },
                    icon("trash", 13),
                  ),
                );
              }),
            ),
          )
        : null;

      const actionsBlock = note.actions?.length
        ? el(
            "section",
            { className: "meeting-summary" },
            el("h3", { textContent: `Việc cần làm (${note.actions.length})` }),
            el(
              "div",
              { className: "action-list" },
              ...note.actions.map((a) =>
                el(
                  "div",
                  { className: "action-item" },
                  el("strong", { textContent: a.task }),
                  el(
                    "div",
                    { className: "action-meta" },
                    a.owner ? el("span", { textContent: `Phụ trách: ${a.owner}` }) : null,
                    a.deadline ? el("span", { textContent: `Thời hạn: ${a.deadline}` }) : null,
                    ...(a.missing ?? []).map((m) =>
                      el("span", { className: "missing-tag", textContent: m }),
                    ),
                  ),
                  ...(a.evidence ?? []).map((e) =>
                    el(
                      "div",
                      { className: "evidence" },
                      el(
                        "button",
                        {
                          className: "time",
                          title: "Nghe lại từ đoạn này",
                          onclick: () => play(e.seconds),
                        },
                        icon("playSm", 11),
                        el("span", { textContent: e.time }),
                      ),
                      el("span", { textContent: `“${e.quote}”` }),
                    ),
                  ),
                ),
              ),
            ),
          )
        : null;

      const activeTplParts: string[] = [];
      if (templates.excel) activeTplParts.push(`Excel (${templates.excel})`);
      if (templates.docx) activeTplParts.push(`Word (${templates.docx})`);
      const templateBar = el(
        "div",
        { className: "template-bar" },
        el("span", {
          textContent: templates.enabled && activeTplParts.length
            ? `Đang dùng file mẫu: ${activeTplParts.join(" · ")}`
            : "File mẫu riêng: Đang tắt (dùng bố cục chuẩn)",
        }),
        el("button", {
          className: "link",
          textContent: "Chọn file mẫu (.xlsx / .docx)",
          onclick: async () => {
            try {
              const updated = await pickAndImportTemplate();
              if (!updated) return;
              templates = updated;
              setStatus("Đã bật file mẫu biên bản.");
              draw();
            } catch (e) {
              setStatus(String(e), true);
            }
          },
        }),
        templates.excel
          ? el("button", {
              className: "link danger-link",
              textContent: "Tắt Excel",
              onclick: async () => {
                try {
                  templates = await resetTemplate("excel");
                  setStatus("Đã tắt template Excel.");
                  draw();
                } catch (e) {
                  setStatus(String(e), true);
                }
              },
            })
          : null,
        templates.docx
          ? el("button", {
              className: "link danger-link",
              textContent: "Tắt Word",
              onclick: async () => {
                try {
                  templates = await resetTemplate("docx");
                  setStatus("Đã tắt template Word.");
                  draw();
                } catch (e) {
                  setStatus(String(e), true);
                }
              },
            })
          : null,
      );

      const fallbackBanner =
        note.fallbackFrom && note.usedModel
          ? el("div", {
              className: "diagram-banner warn",
              role: "status",
              textContent: `Đã dùng ${note.usedModel} do ${note.fallbackFrom} đang quá tải.`,
            })
          : null;

      scroll.replaceChildren(
        ...[
          fallbackBanner,
          discussionBlock,
          showEmailIntro
            ? editable(note.greeting, "note-text", "Lời chào", (h) => { note.greeting = h; saveSoon(); })
            : null,
          showEmailIntro
            ? editable(note.intro, "note-text", "Câu mở đầu", (h) => { note.intro = h; saveSoon(); })
            : null,
          el(
            "table",
            { className: "note-table" },
            el(
              "thead",
              {},
              el(
                "tr",
                {},
                el("th", { className: "num", textContent: "#" }),
                el("th", { className: "col-title", textContent: "Hạng mục" }),
                el("th", { textContent: "Nội dung thống nhất" }),
                el("th", { className: "col-note", textContent: "Ghi chú" }),
              ),
            ),
            tbody,
          ),
          el(
            "div",
            { className: "actions wrap" },
            el("button", {
              className: "link",
              textContent: "Thêm dòng",
              onclick: () => {
                note.rows.push({ title: "", agreed: "", note: "", evidence: [], uncertain: [] });
                changed();
              },
            }),
            !discussionItems.length
              ? el("button", {
                  className: "link",
                  textContent: "Thêm nội dung trao đổi chính",
                  onclick: () => {
                    note.discussion = [""];
                    focusDiscussionIndex = 0;
                    changed();
                  },
                })
              : null,
            !showEmailIntro
              ? el("button", {
                  className: "link",
                  textContent: "Thêm lời chào email",
                  onclick: () => {
                    showEmailIntro = true;
                    draw();
                  },
                })
              : null,
          ),
          templateBar,
          actionsBlock,
          note.pending.length
            ? el(
                "section",
                { className: "pending" },
                el("h3", { textContent: `Cần xác nhận (${note.pending.length})` }),
                ...pending,
              )
            : null,
        ].filter((n): n is HTMLElement => n !== null),
      );
      scroll.scrollTop = top;
    }
    draw();
    if (autoSyncGlossary) {
      void syncGlossary(false);
    }
  }

  const syncFromRun = () => {
    if (!pane.isConnected) return;
    const run = noteRuns.get(id);
    if (!run) return;
    if (run.running) {
      showRunning(run);
      return;
    }
    if (run.completedNote) {
      const done = run.completedNote;
      noteRuns.delete(id);
      showNote(done);
      setStatus("Đã lưu nháp");
      return;
    }
    if (run.error) {
      const { error, previousNote, profileId, useContext, suggestedModel } = run;
      noteRuns.delete(id);
      showForm(error, previousNote, profileId, useContext, suggestedModel);
    }
  };

  const unsubscribeRun = subscribeNoteJob(id, syncFromRun);
  const observer = new MutationObserver(() => {
    if (!pane.isConnected) {
      stopRunningTimer();
      unsubscribeRun();
      observer.disconnect();
    }
  });
  queueMicrotask(() => {
    if (pane.parentNode) observer.observe(pane.parentNode, { childList: true });
  });

  const existingRun = noteRuns.get(id);
  if (existingRun?.running) {
    showRunning(existingRun);
  } else if (existingRun?.completedNote) {
    const done = existingRun.completedNote;
    noteRuns.delete(id);
    showNote(done);
    setStatus("Đã lưu nháp");
  } else if (existingRun?.error) {
    const { error, previousNote, profileId, useContext, suggestedModel } = existingRun;
    noteRuns.delete(id);
    showForm(error, previousNote, profileId, useContext, suggestedModel);
  } else {
    invoke<Note | null>("load_note", { id })
      .then((saved) => {
        if (noteRuns.get(id)?.running) return;
        if (saved) showNote(saved, true);
        else showForm();
      })
      .catch(() => {
        if (!noteRuns.get(id)?.running) showForm();
      });
  }
  return pane;
}
