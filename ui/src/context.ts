//! Per-recording Meeting Context tab (notes + file attachments) and AI-assisted cleaning preview.

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import { el } from "./dom";
import { type AiView, PROVIDER_NAME, PROVIDERS, type Provider } from "./ipc";

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

    const maxFileMb = Math.round(ctx.maxFileBytes / (1024 * 1024));
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
        el("p", {
          className: "hint",
          textContent:
            "Lưu riêng cho bản ghi này. Ghi chú chỉ lưu tại máy và chỉ gửi cho AI khi bạn chủ động bấm chạy AI có dùng ngữ cảnh.",
        }),
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
          textContent: `Hỗ trợ PNG, JPG/JPEG, PDF, TXT, Markdown · tối đa ${maxFileMb} MB/tệp, tổng ${maxTotalMb} MB (${formatBytes(usedBytes)} đã dùng). App lưu bản sao riêng nên tệp vẫn dùng được nếu tệp gốc đổi tên hoặc di chuyển.`,
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
  onSaved: (path: string) => void;
};

export function aiCleanPane(jobId: number, title: string, cb: AiCleanCallbacks): HTMLElement {
  const head = el(
    "header",
    { className: "reader-head" },
    el("h2", { textContent: `Làm sạch bằng AI — ${title}` }),
    el("button", { className: "link", textContent: "Về bản chép lời", onclick: cb.back }),
  );
  const scroll = el("div", { className: "note-scroll" });
  const body = el("div", { className: "note-form" });
  body.append(el("p", { className: "hint", textContent: "Đang tải cấu hình AI và ngữ cảnh…" }));
  scroll.append(body);

  const renderSetup = (
    ai: AiView,
    ctx: MeetingContextView,
    failure = "",
    initialProvider?: Provider,
    initialUseContext?: boolean,
  ) => {
    const usable = PROVIDERS.filter((p) => ai.keys[p]);
    if (usable.length === 0) {
      body.replaceChildren(
        el("h3", { textContent: "Chưa có tài khoản AI để làm sạch" }),
        el("p", {
          className: "hint",
          textContent: "Vào Cấu hình → Tài khoản AI để thêm key Claude, ChatGPT hoặc Gemini.",
        }),
        el("button", { className: "add", textContent: "Mở Cấu hình", onclick: cb.openSettings }),
      );
      return;
    }

    const pickedProvider =
      initialProvider && usable.includes(initialProvider)
        ? initialProvider
        : usable.includes(ai.defaultProvider)
          ? ai.defaultProvider
          : usable[0];

    const providerSelect = el(
      "select",
      { className: "field" },
      ...usable.map((p) =>
        el("option", {
          value: p,
          selected: p === pickedProvider,
          textContent: `${PROVIDER_NAME[p]} (${ai.models[p]})`,
        }),
      ),
    ) as HTMLSelectElement;

    const ctxInfo = describeContext(ctx);
    const useCtxBox = el("input", {
      type: "checkbox",
      checked: initialUseContext ?? ctxInfo.hasAny,
      disabled: !ctxInfo.hasAny,
    }) as HTMLInputElement;

    const disclosure = el("p", { className: "sending" });
    const updateDisclosure = () => {
      const pName = PROVIDER_NAME[providerSelect.value as Provider];
      const sendingParts = ["bản chép lời", "từ vựng riêng"];
      if (useCtxBox.checked && ctxInfo.hasAny) {
        if (ctx.notes.trim()) sendingParts.push("ghi chú ngữ cảnh");
        if (ctx.attachments.length > 0) sendingParts.push(`${ctx.attachments.length} tệp đính kèm`);
      }
      disclosure.textContent = `Sẽ gửi ${sendingParts.join(", ")} đến ${pName}. Bản gốc không bị ghi đè.`;
    };
    providerSelect.onchange = updateDisclosure;
    useCtxBox.onchange = updateDisclosure;
    updateDisclosure();

    const runBtn = el("button", {
      className: "add",
      textContent: "Chạy làm sạch bằng AI",
      onclick: async () => {
        const provider = providerSelect.value as Provider;
        const useContext = useCtxBox.checked && ctxInfo.hasAny;
        runBtn.disabled = true;
        runBtn.textContent = "Đang rà soát bản chép lời…";
        try {
          const proposal = await invoke<AiCleanProposal>("preview_ai_clean", {
            id: jobId,
            provider,
            useContext,
          });
          renderDiff(ai, ctx, proposal, provider, useContext);
        } catch (e) {
          renderSetup(ai, ctx, String(e), provider, useContext);
        }
      },
    });

    body.replaceChildren(
      el("h3", { textContent: "Làm sạch bản chép lời bằng AI" }),
      el("p", {
        className: "hint",
        textContent:
          "AI đề xuất sửa tên riêng, từ viết tắt và thuật ngữ nghe nhầm có căn cứ, giữ nguyên mốc giờ, số liệu và phủ định. Bạn sẽ đối chiếu trước khi lưu bản sạch riêng.",
      }),
      failure ? el("p", { className: "problem problem-text", textContent: failure }) : "",
      el("label", {}, "Xử lý bằng", providerSelect),
      el(
        "div",
        { className: "context-summary-box" },
        el(
          "label",
          { className: "hw-filter-toggle" },
          useCtxBox,
          el("span", { textContent: "Dùng ngữ cảnh cuộc họp" }),
        ),
        el("span", { className: "style-desc", textContent: ctxInfo.label }),
        el("button", {
          className: "link",
          textContent: "Mở tab Ngữ cảnh",
          onclick: cb.openContext,
        }),
      ),
      disclosure,
      el("div", { className: "actions wrap" }, runBtn),
    );
  };

  const renderDiff = (
    ai: AiView,
    ctx: MeetingContextView,
    proposal: AiCleanProposal,
    provider: Provider,
    useContext: boolean,
  ) => {
    const saveStatus = el("span", { className: "provider-status status-inline" });
    const saveBtn = el("button", {
      className: "add",
      textContent: "Lưu bản sạch riêng",
      disabled: proposal.edits.length === 0,
      onclick: async () => {
        saveBtn.disabled = true;
        try {
          const path = await invoke<string>("save_ai_clean", {
            id: jobId,
            cleanedText: proposal.cleanedText,
          });
          cb.onSaved(path);
        } catch (e) {
          saveStatus.className = "provider-status status-inline bad";
          saveStatus.textContent = String(e);
          saveBtn.disabled = false;
        }
      },
    });

    const diffNodes =
      proposal.edits.length === 0
        ? el("p", {
            className: "hint",
            textContent: "AI không phát hiện lỗi tên riêng hay thuật ngữ nào cần sửa thêm.",
          })
        : el(
            "div",
            { className: "clean-diff-list" },
            ...proposal.edits.map((edit) =>
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
        textContent: `Đề xuất chỉnh sửa (${proposal.edits.length} đoạn thay đổi)`,
      }),
      diffNodes,
      el(
        "div",
        { className: "actions wrap" },
        saveBtn,
        el("button", {
          className: "link",
          textContent: "Chạy lại",
          onclick: () => renderSetup(ai, ctx, "", provider, useContext),
        }),
        saveStatus,
      ),
    );
  };

  Promise.all([
    invoke<AiView>("ai_settings"),
    invoke<MeetingContextView>("get_meeting_context", { id: jobId }),
  ])
    .then(([ai, ctx]) => renderSetup(ai, ctx))
    .catch((e) => body.replaceChildren(el("p", { className: "problem problem-text", textContent: String(e) })));

  return el("section", { className: "reader" }, head, scroll);
}
