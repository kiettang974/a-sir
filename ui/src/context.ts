//! Per-recording Meeting Context tab (notes + file attachments) and AI-assisted cleaning preview.

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import { el } from "./dom";
import { type AiView, progressBar, PROVIDER_NAME, PROVIDERS, type RecordFlowNav, recordFlowTabs } from "./ipc";

export type ContextAttachment = {
  id: string;
  name: string;
  mimeType: string;
  bytes: number;
  storedFile: string;
};

export type MeetingContextView = {
  notes: string;
  attachments: ContextAttachment[];
  maxFileBytes: number;
  maxTotalBytes: number;
  acceptedExtensions: string[];
};

type AddContextFilesView = {
  context: MeetingContextView;
  errors: string[];
};

type AiCleanEdit = {
  line: number;
  time: string;
  original: string;
  suggested: string;
  reason: string;
};

type AiCleanProposal = {
  edits: AiCleanEdit[];
  cleanedText: string;
};

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

export function describeContext(ctx: MeetingContextView): { hasAny: boolean; label: string } {
  const hasNotes = Boolean(ctx.notes.trim());
  const fileCount = ctx.attachments.length;
  const parts: string[] = [];
  if (hasNotes) parts.push("có ghi chú");
  if (fileCount > 0) parts.push(`${fileCount} tệp đính kèm`);
  if (parts.length === 0) return { hasAny: false, label: "Chưa có ghi chú hoặc tệp đính kèm." };
  return { hasAny: true, label: parts.join(" · ") };
}

export function contextTabBody(jobId: number): HTMLElement {
  const host = el("div", { className: "context-pane" });
  host.append(el("p", { className: "hint", textContent: "Đang tải ngữ cảnh cuộc họp…" }));

  const draw = (ctx: MeetingContextView, fileErrors: string[] = [], savedMsg = "") => {
    const notesInput = el("textarea", {
      className: "field context-notes",
      rows: 7,
      value: ctx.notes,
      ariaLabel: "Ghi chú ngữ cảnh cuộc họp",
      placeholder:
        "Nhập mục tiêu cuộc họp, người tham gia, tên hệ thống, chữ viết tắt và cách viết đúng của các từ hay bị nghe nhầm…",
    }) as HTMLTextAreaElement;

    const noteStatus = el("span", {
      className: "provider-status status-inline",
      textContent: savedMsg,
    });
    const saveNotesBtn = el("button", {
      className: "add",
      textContent: "Lưu ghi chú",
      onclick: async () => {
        saveNotesBtn.disabled = true;
        noteStatus.className = "provider-status status-inline";
        noteStatus.textContent = "Đang lưu…";
        try {
          const updated = await invoke<MeetingContextView>("save_meeting_context_notes", {
            id: jobId,
            notes: notesInput.value,
          });
          draw(updated, [], "Đã lưu ghi chú.");
        } catch (e) {
          noteStatus.className = "provider-status status-inline bad";
          noteStatus.textContent = `Chưa lưu được: ${e}`;
          saveNotesBtn.disabled = false;
        }
      },
    });

    const maxTotalMb = Math.round(ctx.maxTotalBytes / (1024 * 1024));
    const usedBytes = ctx.attachments.reduce((sum, a) => sum + a.bytes, 0);

    const addFileBtn = el("button", {
      className: "btn-ghost",
      textContent: "Thêm tệp đính kèm",
      onclick: async () => {
        const picked = await open({
          multiple: true,
          filters: [
            {
              name: "Ảnh & tài liệu (PNG, JPG, PDF, TXT, MD)",
              extensions: ctx.acceptedExtensions,
            },
          ],
        });
        if (!picked) return;
        const paths = Array.isArray(picked) ? picked : [picked];
        if (paths.length === 0) return;
        addFileBtn.disabled = true;
        try {
          const res = await invoke<AddContextFilesView>("add_meeting_context_files", {
            id: jobId,
            paths,
          });
          draw(res.context, res.errors);
        } catch (e) {
          draw(ctx, [String(e)]);
        }
      },
    });

    const fileList =
      ctx.attachments.length === 0
        ? el("p", {
            className: "hint",
            textContent: "Chưa có tệp nào. Ảnh sơ đồ, slide PDF hoặc ghi chú TXT/Markdown giúp AI hiểu đúng thuật ngữ.",
          })
        : el(
            "ul",
            { className: "context-file-list" },
            ...ctx.attachments.map((att) =>
              el(
                "li",
                { className: "context-file-row" },
                el(
                  "div",
                  { className: "context-file-info" },
                  el("strong", { textContent: att.name }),
                  el("span", {
                    className: "context-file-meta",
                    textContent: `${formatBytes(att.bytes)} · ${att.mimeType}`,
                  }),
                ),
                el("button", {
                  className: "link danger-link",
                  textContent: "Gỡ",
                  title: `Gỡ "${att.name}" khỏi bản ghi này (không xoá tệp gốc trên máy)`,
                  onclick: async () => {
                    try {
                      const updated = await invoke<MeetingContextView>(
                        "remove_meeting_context_file",
                        { id: jobId, attachmentId: att.id },
                      );
                      draw(updated);
                    } catch (e) {
                      draw(ctx, [String(e)]);
                    }
                  },
                }),
              ),
            ),
          );

    const errorBox =
      fileErrors.length > 0
        ? el(
            "div",
            { className: "context-errors" },
            ...fileErrors.map((err) => el("p", { className: "problem problem-text", textContent: err })),
          )
        : null;

    host.replaceChildren(
      el(
        "div",
        { className: "context-card" },
        el("h3", { textContent: "Ghi chú cuộc họp" }),
        notesInput,
        el("div", { className: "actions wrap" }, saveNotesBtn, noteStatus),
      ),
      el(
        "div",
        { className: "context-card" },
        el(
          "div",
          { className: "hf-head" },
          el("h3", { textContent: "Ảnh & tài liệu đính kèm" }),
          addFileBtn,
        ),
        el("p", {
          className: "hint",
          textContent: `PNG, JPG, PDF, TXT, MD · ${formatBytes(usedBytes)} / ${maxTotalMb} MB`,
        }),
        errorBox,
        fileList,
      ),
    );
  };

  invoke<MeetingContextView>("get_meeting_context", { id: jobId })
    .then((ctx) => draw(ctx))
    .catch((e) => host.replaceChildren(el("p", { className: "problem problem-text", textContent: String(e) })));

  return host;
}

type AiCleanCallbacks = {
  back: () => void;
  openSettings: () => void;
  openContext: () => void;
  openSummary: () => void;
  onSaved: (path: string) => void;
  flow: RecordFlowNav;
};

type SavedAiClean = {
  edits: AiCleanEdit[];
  savedFileName: string;
  providerName: string;
  usedContext: boolean;
};

const savedCleanCache = new Map<number, SavedAiClean>();
const runningCleanJobs = new Set<number>();
const cleanStartedAt = new Map<number, number>();
const cleanEstimatedSecs = new Map<number, number>();
const cleanErrorCache = new Map<number, string>();
const cleanListeners = new Map<number, Set<() => void>>();

function formatCleanEta(elapsedS: number, estS: number): string {
  const remain = Math.max(0, estS - elapsedS);
  if (remain > 0) {
    return `${elapsedS}s · Ước tính còn ~${remain}s`;
  }
  return `${elapsedS}s · Sắp xong…`;
}

function estimateCleanSeconds(charCount: number): number {
  return Math.max(10, Math.min(90, Math.round(charCount / 400) + 8));
}

function subscribeClean(jobId: number, fn: () => void): () => void {
  let set = cleanListeners.get(jobId);
  if (!set) {
    set = new Set();
    cleanListeners.set(jobId, set);
  }
  set.add(fn);
  return () => {
    set?.delete(fn);
    if (set?.size === 0) cleanListeners.delete(jobId);
  };
}

function notifyClean(jobId: number) {
  const set = cleanListeners.get(jobId);
  if (!set) return;
  for (const fn of set) fn();
}

export function getAiCleanButtonState(jobId: number): "idle" | "running" | "ready" | "error" {
  if (runningCleanJobs.has(jobId)) return "running";
  if (savedCleanCache.has(jobId)) return "ready";
  if (cleanErrorCache.has(jobId)) return "error";
  return "idle";
}

/**
 * Opens an existing cleaned transcript immediately, or runs AI/local cleaning in the background
 * while keeping the user on the Reader screen. Never auto-navigates when finished; the user
 * clicks the button again to inspect the cleaned result.
 */
export async function openOrStartAiClean(
  jobId: number,
  callbacks: {
    onStateChange: () => void;
    onOpenClean: (id: number) => void;
    onFinished?: (id: number, error: string | null) => void;
  },
): Promise<void> {
  if (runningCleanJobs.has(jobId)) return;
  if (savedCleanCache.has(jobId) || cleanErrorCache.has(jobId)) {
    callbacks.onOpenClean(jobId);
    return;
  }

  try {
    const diskSaved = await invoke<SavedAiClean | null>("load_ai_clean", { id: jobId });
    if (diskSaved && Array.isArray(diskSaved.edits)) {
      savedCleanCache.set(jobId, diskSaved);
      callbacks.onOpenClean(jobId);
      return;
    }
  } catch {
    // No saved cleaned copy yet; run cleaning in background.
  }

  runningCleanJobs.add(jobId);
  cleanErrorCache.delete(jobId);
  callbacks.onStateChange();

  try {
    const [ai, ctx] = await Promise.all([
      invoke<AiView>("ai_settings"),
      invoke<MeetingContextView>("get_meeting_context", { id: jobId }),
    ]);
    const usable = PROVIDERS.filter((p) => ai.keys[p]);
    if (usable.length === 0) {
      const path = await invoke<string>("clean_transcript", { id: jobId });
      const fileName = path.split(/[/\\]/).pop() ?? path;
      savedCleanCache.set(jobId, {
        edits: [],
        savedFileName: fileName,
        providerName: "Làm sạch tại máy",
        usedContext: false,
      });
    } else {
      const provider = usable.includes(ai.defaultProvider) ? ai.defaultProvider : usable[0];
      const useContext = describeContext(ctx).hasAny;
      const pName = PROVIDER_NAME[provider];
      const proposal = await invoke<AiCleanProposal>("preview_ai_clean", {
        id: jobId,
        provider,
        useContext,
      });
      const saved: SavedAiClean = {
        edits: proposal.edits,
        savedFileName: "",
        providerName: pName,
        usedContext: useContext,
      };
      const savedPath = await invoke<string>("save_ai_clean", {
        id: jobId,
        cleanedText: proposal.cleanedText,
        meta: saved,
      });
      saved.savedFileName = savedPath.split(/[/\\]/).pop() ?? savedPath;
      invoke("save_ai_clean", {
        id: jobId,
        cleanedText: proposal.cleanedText,
        meta: saved,
      }).catch(() => {});
      savedCleanCache.set(jobId, saved);
    }
    callbacks.onFinished?.(jobId, null);
  } catch (e) {
    const err = String(e);
    cleanErrorCache.set(jobId, err);
    callbacks.onFinished?.(jobId, err);
  } finally {
    runningCleanJobs.delete(jobId);
    callbacks.onStateChange();
  }
}

export function aiCleanPane(jobId: number, title: string, cb: AiCleanCallbacks): HTMLElement {
  const head = el("header", { className: "reader-head" });
  const scroll = el("div", { className: "note-scroll" });
  const body = el("div", { className: "note-form" });
  scroll.append(body);

  const renderHead = (busy = false) => {
    head.replaceChildren(
      el("div", { className: "reader-head-top" }, el("h2", { textContent: title })),
      el(
        "div",
        { className: "reader-actions" },
        el("button", {
          className: "btn-primary",
          textContent: busy ? "Đang làm sạch…" : "Chạy lại làm sạch",
          disabled: busy,
          onclick: () => void runAuto(),
        }),
      ),
    );
  };

  const runLocalOnly = async () => {
    renderHead(true);
    body.replaceChildren(el("h3", { textContent: "Đang làm sạch văn bản…" }));
    try {
      const path = await invoke<string>("clean_transcript", { id: jobId });
      cb.onSaved(path);
    } catch (e) {
      renderHead(false);
      body.replaceChildren(
        el("h3", { textContent: "Chưa làm sạch được" }),
        el("p", { className: "problem problem-text", textContent: String(e) }),
      );
    }
  };

  const renderSaved = (saved: SavedAiClean) => {
    renderHead(false);
    const diffNodes =
      saved.edits.length === 0
        ? el("p", {
            className: "hint",
            textContent:
              "Không phát hiện thêm lỗi tên riêng hay thuật ngữ nào. Đã lưu bản làm sạch tại máy.",
          })
        : el(
            "div",
            { className: "clean-diff-list" },
            ...saved.edits.map((edit) =>
              el(
                "div",
                { className: "clean-diff-item" },
                el(
                  "div",
                  { className: "clean-diff-head" },
                  el("span", {
                    className: "model-badge",
                    textContent: edit.time ? `${edit.time} · Dòng ${edit.line}` : `Dòng ${edit.line}`,
                  }),
                  edit.reason
                    ? el("span", { className: "clean-diff-reason", textContent: edit.reason })
                    : null,
                ),
                el("p", { className: "clean-diff-old", textContent: edit.original }),
                el("p", { className: "clean-diff-new", textContent: edit.suggested }),
              ),
            ),
          );

    body.replaceChildren(
      el("h3", {
        textContent: `Đã lưu bản sạch (${saved.edits.length} đoạn thay đổi)`,
      }),
      el("p", {
        className: "hint",
        textContent: `${saved.savedFileName} · ${saved.providerName}${saved.usedContext ? " · có dùng Ngữ cảnh" : ""}`,
      }),
      diffNodes,
    );
  };

  let elapsedTimer: number | undefined;
  const stopTimer = () => {
    if (elapsedTimer !== undefined) {
      window.clearInterval(elapsedTimer);
      elapsedTimer = undefined;
    }
  };

  const showError = (heading: string, errText: string) => {
    stopTimer();
    renderHead(false);
    body.replaceChildren(
      el("h3", { textContent: heading }),
      el("p", { className: "problem problem-text", textContent: errText }),
      el(
        "div",
        { className: "actions wrap" },
        el("button", { className: "add", textContent: "Thử lại", onclick: () => void runAuto() }),
        el("button", { className: "link", textContent: "Mở Cấu hình AI", onclick: cb.openSettings }),
      ),
    );
  };

  const renderEmpty = () => {
    stopTimer();
    renderHead(false);
    body.replaceChildren(
      el(
        "div",
        { className: "actions wrap" },
        el("button", {
          className: "btn-primary",
          textContent: "Chạy làm sạch",
          onclick: () => void runAuto(),
        }),
      ),
    );
  };

  const renderRunning = () => {
    stopTimer();
    renderHead(true);
    const startedAt = cleanStartedAt.get(jobId) ?? Date.now();
    if (!cleanStartedAt.has(jobId)) cleanStartedAt.set(jobId, startedAt);
    const estS = cleanEstimatedSecs.get(jobId) ?? 20;

    const barHost = el("div", { className: "diagram-loading-bar" });
    const elapsedLine = el("p", { className: "diagram-loading-meta" });

    const updateProgress = () => {
      const elapsedS = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
      const pct = Math.min(95, Math.max(4, Math.round((elapsedS / Math.max(1, estS)) * 92)));
      barHost.replaceChildren(progressBar(pct));
      elapsedLine.textContent = formatCleanEta(elapsedS, estS);
    };

    updateProgress();
    body.replaceChildren(
      el(
        "div",
        { className: "ai-loading-stage", role: "status", ariaLive: "polite" },
        el("div", { className: "diagram-spinner", ariaHidden: "true" }),
        el("p", { className: "diagram-loading-title", textContent: "Đang làm sạch văn bản…" }),
        el("p", {
          className: "diagram-loading-sub",
          textContent: "AI đang soát lỗi chính tả và thuật ngữ. Bạn có thể chuyển sang tab khác trong lúc chờ.",
        }),
        barHost,
        elapsedLine,
      ),
    );
    elapsedTimer = window.setInterval(() => {
      if (!runningCleanJobs.has(jobId) || !body.isConnected) {
        stopTimer();
        return;
      }
      updateProgress();
    }, 500);
  };

  const runAuto = async () => {
    if (runningCleanJobs.has(jobId)) {
      renderRunning();
      return;
    }
    runningCleanJobs.add(jobId);
    cleanStartedAt.set(jobId, Date.now());
    cleanErrorCache.delete(jobId);
    if (!cleanEstimatedSecs.has(jobId)) {
      cleanEstimatedSecs.set(jobId, 20);
    }
    notifyClean(jobId);
    renderRunning();

    try {
      const [ai, ctx, tr] = await Promise.all([
        invoke<AiView>("ai_settings"),
        invoke<MeetingContextView>("get_meeting_context", { id: jobId }),
        invoke<{ text: string }>("read_transcript", { id: jobId }).catch(() => ({ text: "" })),
      ]);
      cleanEstimatedSecs.set(jobId, estimateCleanSeconds(tr.text.length));
      notifyClean(jobId);
      const usable = PROVIDERS.filter((p) => ai.keys[p]);
      if (usable.length === 0) {
        await runLocalOnly();
        return;
      }
      const provider = usable.includes(ai.defaultProvider) ? ai.defaultProvider : usable[0];
      const useContext = describeContext(ctx).hasAny;
      const pName = PROVIDER_NAME[provider];
      const proposal = await invoke<AiCleanProposal>("preview_ai_clean", {
        id: jobId,
        provider,
        useContext,
      });
      const saved: SavedAiClean = {
        edits: proposal.edits,
        savedFileName: "",
        providerName: pName,
        usedContext: useContext,
      };
      const savedPath = await invoke<string>("save_ai_clean", {
        id: jobId,
        cleanedText: proposal.cleanedText,
        meta: saved,
      });
      saved.savedFileName = savedPath.split(/[/\\]/).pop() ?? savedPath;
      invoke("save_ai_clean", {
        id: jobId,
        cleanedText: proposal.cleanedText,
        meta: saved,
      }).catch(() => {});
      savedCleanCache.set(jobId, saved);
    } catch (e) {
      cleanErrorCache.set(jobId, String(e));
    } finally {
      runningCleanJobs.delete(jobId);
      cleanStartedAt.delete(jobId);
      notifyClean(jobId);
    }
  };

  const syncView = () => {
    if (runningCleanJobs.has(jobId)) {
      renderRunning();
      return;
    }
    const latest = savedCleanCache.get(jobId);
    const latestErr = cleanErrorCache.get(jobId);
    if (latestErr) {
      showError("AI báo lỗi khi làm sạch", latestErr);
    } else if (latest) {
      renderSaved(latest);
    } else {
      renderEmpty();
    }
  };

  const pane = el("section", { className: "reader" }, head, recordFlowTabs("aiClean", cb.flow), scroll);
  const unsub = subscribeClean(jobId, () => {
    if (pane.isConnected) syncView();
  });
  const observer = new MutationObserver(() => {
    if (!pane.isConnected) {
      stopTimer();
      unsub();
      observer.disconnect();
    }
  });
  queueMicrotask(() => {
    if (pane.parentNode) observer.observe(pane.parentNode, { childList: true });
  });

  if (runningCleanJobs.has(jobId)) {
    renderRunning();
  } else if (savedCleanCache.has(jobId) || cleanErrorCache.has(jobId)) {
    syncView();
  } else {
    renderHead(false);
    invoke<SavedAiClean | null>("load_ai_clean", { id: jobId })
      .then((diskSaved) => {
        if (!pane.isConnected) return;
        if (diskSaved && Array.isArray(diskSaved.edits)) {
          savedCleanCache.set(jobId, diskSaved);
          renderSaved(diskSaved);
        } else {
          renderEmpty();
        }
      })
      .catch(() => {
        if (pane.isConnected) renderEmpty();
      });
  }
  return pane;
}
