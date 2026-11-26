import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { writeHtml } from "@tauri-apps/plugin-clipboard-manager";
import { ask, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { el } from "./dom";
import { icon } from "./icons";
import {
  type AiView,
  type MeetingTemplates,
  overflowMenu,
  pickAndImportTemplate,
  PROVIDER_NAME,
  PROVIDERS,
  type Provider,
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
type Draft = { discussion?: string[]; rows: Row[]; actions?: ActionItem[]; pending: Pending[] };
/** The working copy. Row text is HTML by now, because it is edited in place. */
type Note = Draft & { greeting: string; intro: string };

const DEFAULT_INTRO =
  "Em xin gửi lại meeting note đã trao đổi hôm nay ạ. Nếu có nội dung cần bổ sung, anh chị nhắn em ạ. Em cảm ơn team đã tham gia.";
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

type Links = { openSettings: () => void; back: () => void };

/** Fills itself in and keeps its own state, so queue updates can't disturb typing here. */
export function notePane(id: number, name: string, links: Links): HTMLElement {
  const head = el("header", { className: "reader-head" });
  const scroll = el("div", { className: "note-scroll" });
  const player = el("div", { className: "player" });
  player.hidden = true;
  const audio = el("audio", { controls: true, preload: "metadata" });
  const problem = el("span", { className: "problem" });
  player.append(audio, problem);
  const pane = el("section", { className: "reader" }, head, scroll, player);

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

  const back = el("button", { className: "link", textContent: "Về bản chép lời", onclick: links.back });

  function showForm(greeting: string, intro: string, failure = "", previousNote?: Note) {
    head.replaceChildren(el("h2", { textContent: name }), back);
    const body = el("div", { className: "note-form" });
    scroll.replaceChildren(body);
    invoke<AiView>("ai_settings")
      .then((ai) => drawForm(body, ai, greeting, intro, failure, previousNote))
      .catch((e) => body.replaceChildren(el("p", { className: "problem problem-text", textContent: String(e) })));
  }

  function drawForm(
    body: HTMLElement,
    ai: AiView,
    greeting: string,
    intro: string,
    failure: string,
    previousNote?: Note,
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
    const greetingBox = el("input", { className: "field", value: greeting, placeholder: "Dear anh Nguyễn, anh Quốc và team," });
    const introBox = el("textarea", { className: "field", value: intro, rows: 3 });
    const initial = usable.includes(ai.defaultProvider) ? ai.defaultProvider : usable[0];
    const select = el(
      "select",
      { className: "field" },
      ...usable.map((p) =>
        el("option", { value: p, selected: p === initial, textContent: `${PROVIDER_NAME[p]} (${ai.models[p]})` }),
      ),
    );
    const label = () => `Tạo tóm tắt bằng ${PROVIDER_NAME[select.value as Provider]}`;
    const notice = () => `Nội dung bản chép lời sẽ được gửi đến ${PROVIDER_NAME[select.value as Provider]}.`;
    const sending = el("p", { className: "sending", textContent: notice() });
    const start = el("button", { className: "add", textContent: label() });
    select.onchange = () => {
      start.textContent = label();
      sending.textContent = notice();
    };
    start.onclick = async () => {
      const provider = select.value as Provider;
      start.disabled = true;
      start.textContent = "Đang tạo tóm tắt…";
      sending.textContent = `Đang chờ ${PROVIDER_NAME[provider]}. Cuộc họp dài có thể mất vài phút.`;
      try {
        const draft = await invoke<Draft>("summarize_job", { id, provider });
        const note: Note = {
          greeting: escapeHtml(greetingBox.value),
          intro: escapeHtml(introBox.value),
          discussion: draft.discussion ?? [],
          rows: draft.rows.map(toHtmlRow),
          actions: draft.actions ?? [],
          pending: draft.pending.map((p) => ({ ...p, row: toHtmlRow(p.row) })),
        };
        showNote(note);
        save(note);
      } catch (e) {
        showForm(greetingBox.value, introBox.value, String(e), previousNote);
      }
    };
    body.replaceChildren(
      el("h3", { textContent: "Tạo biên bản cuộc họp" }),
      failure ? el("p", { className: "problem problem-text", textContent: failure }) : "",
      el("label", {}, "Lời chào", greetingBox),
      el("label", {}, "Câu mở đầu", introBox),
      el("label", {}, "Soạn bằng", select),
      sending,
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
    );
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

  function showNote(note: Note) {
    const saveSoon = () => {
      setStatus("");
      clearTimeout(timer);
      timer = window.setTimeout(() => save(note), SAVE_DELAY_MS);
    };
    const changed = () => {
      save(note);
      draw();
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
      el("h2", { textContent: name }),
      status,
      el(
        "div",
        { className: "reader-actions" },
        el(
          "button",
          {
            className: "btn-primary",
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
        overflowMenu("Thao tác khác cho biên bản", [
          {
            label: "Tạo lại biên bản",
            onClick: async () => {
              const yes = await ask("Tạo lại sẽ thay bản nháp này bằng bản mới. Nếu gặp lỗi, bản hiện tại vẫn còn nguyên.", {
                title: "Tạo lại biên bản?",
                kind: "warning",
                okLabel: "Tạo lại",
                cancelLabel: "Giữ bản này",
              });
              if (yes) showForm(htmlToText(note.greeting, false), htmlToText(note.intro, false), "", note);
            },
          },
          {
            label: "Mở trong Finder",
            onClick: async () => {
              await save(note);
              if (savedPage) revealItemInDir(savedPage);
            },
          },
          {
            label: "Về bản chép lời",
            onClick: links.back,
          },
        ]),
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
      const discussionBlock = note.discussion?.length
        ? el(
            "section",
            { className: "meeting-summary" },
            el("h3", { textContent: "Nội dung trao đổi chính" }),
            el(
              "ul",
              {},
              ...note.discussion.map((item) => el("li", { textContent: item })),
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

      scroll.replaceChildren(
        ...[
          discussionBlock,
          editable(note.greeting, "note-text", "Lời chào", (h) => { note.greeting = h; saveSoon(); }),
          editable(note.intro, "note-text", "Câu mở đầu", (h) => { note.intro = h; saveSoon(); }),
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
          ),
          templateBar,
          actionsBlock,
          note.pending.length
            ? el(
                "section",
                { className: "pending" },
                el("h3", { textContent: `Cần xác nhận (${note.pending.length})` }),
                el("p", {
                  className: "hint",
                  textContent: "Các mục này chưa đủ căn cứ để ghi là đã thống nhất và không có trong bản gửi đi. Hãy nghe lại rồi quyết định.",
                }),
                ...pending,
              )
            : null,
        ].filter((n): n is HTMLElement => n !== null),
      );
      scroll.scrollTop = top;
    }
    draw();
  }

  invoke<Note | null>("load_note", { id })
    .then((saved) => (saved ? showNote(saved) : showForm("", DEFAULT_INTRO)))
    .catch(() => showForm("", DEFAULT_INTRO));
  return pane;
}
