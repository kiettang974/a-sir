import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { writeHtml } from "@tauri-apps/plugin-clipboard-manager";
import { ask } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { el } from "./dom";
import { type AiView, PROVIDER_NAME, PROVIDERS, type Provider } from "./settings";

type Evidence = { time: string; quote: string; seconds: number };
type Row = { title: string; agreed: string; note: string; evidence: Evidence[]; uncertain: string[] };
type Pending = { row: Row; reason: string };
type Draft = { rows: Row[]; pending: Pending[] };
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

function plainText(note: Note): string {
  const text = (html: string) => {
    const holder = document.createElement("div");
    holder.innerHTML = cleanHtml(html).replace(/<br>/g, "\n");
    return holder.textContent ?? "";
  };
  const rows = note.rows.map(
    (r, i) => `${i + 1}. ${text(r.title)}\n${text(r.agreed)}${r.note ? `\nGhi chú: ${text(r.note)}` : ""}`,
  );
  return [text(note.greeting), text(note.intro), ...rows].join("\n\n");
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

  audio.onerror = () => {
    audio.hidden = true;
    problem.textContent = "Không phát được file ghi âm này. File có thể đã bị chuyển đi hoặc ở định dạng trình phát không đọc được.";
  };
  const play = async (seconds: number) => {
    player.hidden = false;
    if (!audio.getAttribute("src")) {
      try {
        const path = await invoke<string>("allow_job_audio", { id });
        audio.src = convertFileSrc(path);
      } catch (e) {
        problem.textContent = String(e);
        return;
      }
    }
    audio.currentTime = seconds;
    audio.play().catch(() => {});
  };

  const back = el("button", { className: "link", textContent: "Về bản chép lời", onclick: links.back });

  function showForm(greeting: string, intro: string, failure = "") {
    head.replaceChildren(el("h2", { textContent: name }), back);
    const body = el("div", { className: "note-form" });
    scroll.replaceChildren(body);
    invoke<AiView>("ai_settings")
      .then((ai) => drawForm(body, ai, greeting, intro, failure))
      .catch((e) => body.replaceChildren(el("p", { className: "problem problem-text", textContent: String(e) })));
  }

  function drawForm(body: HTMLElement, ai: AiView, greeting: string, intro: string, failure: string) {
    const usable = PROVIDERS.filter((p) => ai.keys[p]);
    if (usable.length === 0) {
      body.replaceChildren(
        el("h3", { textContent: "Cần API key để tạo bảng tóm tắt" }),
        el("p", {
          className: "hint",
          textContent: "Thêm API key của Claude, ChatGPT hoặc Gemini trong phần Cấu hình.",
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
    const label = () => `Tạo bảng bằng ${PROVIDER_NAME[select.value as Provider]}`;
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
      start.textContent = "Đang tạo bảng…";
      sending.textContent = `Đang chờ ${PROVIDER_NAME[provider]}. Cuộc họp dài có thể mất vài phút.`;
      try {
        const draft = await invoke<Draft>("summarize_job", { id, provider });
        const note: Note = {
          greeting: escapeHtml(greetingBox.value),
          intro: escapeHtml(introBox.value),
          rows: draft.rows.map(toHtmlRow),
          pending: draft.pending.map((p) => ({ ...p, row: toHtmlRow(p.row) })),
        };
        showNote(note);
        save(note);
      } catch (e) {
        showForm(greetingBox.value, introBox.value, String(e));
      }
    };
    body.replaceChildren(
      el("h3", { textContent: "Tạo bảng tóm tắt" }),
      failure ? el("p", { className: "problem problem-text", textContent: failure }) : "",
      el("label", {}, "Lời chào", greetingBox),
      el("label", {}, "Câu mở đầu", introBox),
      el("label", {}, "Soạn bằng", select),
      sending,
      start,
    );
  }

  let savedPage = "";
  let timer: number | undefined;
  const status = el("span", { className: "hint", style: "margin:0" } as never);

  async function save(note: Note) {
    clearTimeout(timer);
    try {
      savedPage = await invoke<string>("save_note", { id, draft: note, html: `<!doctype html><html lang="vi"><meta charset="utf-8"><body>${emailHtml(note)}</body></html>` });
      status.textContent = "Đã lưu nháp";
    } catch (e) {
      status.textContent = String(e);
    }
  }

  function showNote(note: Note) {
    const saveSoon = () => {
      status.textContent = "";
      clearTimeout(timer);
      timer = window.setTimeout(() => save(note), SAVE_DELAY_MS);
    };
    const changed = () => {
      save(note);
      draw();
    };
    head.replaceChildren(
      el("h2", { textContent: name }),
      status,
      el("button", {
        className: "link",
        textContent: "Chép để dán vào email",
        onclick: async () => {
          await save(note);
          await writeHtml(emailHtml(note), plainText(note));
          const left = note.pending.length;
          status.textContent = left
            ? `Đã chép bảng. Còn ${left} mục cần xác nhận chưa đưa vào.`
            : "Đã chép bảng.";
        },
      }),
      el("button", {
        className: "link",
        textContent: "Tạo lại",
        onclick: async () => {
          const yes = await ask("Tạo lại sẽ thay thế bản nháp này, kể cả phần bạn đã sửa.", {
            title: "Tạo lại bảng tóm tắt?",
            kind: "warning",
            okLabel: "Tạo lại",
            cancelLabel: "Giữ bản này",
          });
          if (yes) showForm(textOf(note.greeting), textOf(note.intro));
        },
      }),
      el("button", {
        className: "link",
        textContent: "Mở trong Finder",
        onclick: async () => {
          await save(note);
          if (savedPage) revealItemInDir(savedPage);
        },
      }),
      back,
    );

    const reviewBits = (row: Row) => [
      ...row.evidence.map((e) =>
        el(
          "div",
          { className: "evidence" },
          el("button", { className: "time", textContent: e.time, title: "Nghe lại từ đoạn này", onclick: () => play(e.seconds) }),
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
      scroll.replaceChildren(
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
        el("button", {
          className: "link",
          textContent: "Thêm dòng",
          onclick: () => {
            note.rows.push({ title: "", agreed: "", note: "", evidence: [], uncertain: [] });
            changed();
          },
        }),
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
          : "",
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

function textOf(html: string): string {
  const holder = document.createElement("div");
  holder.innerHTML = cleanHtml(html);
  return holder.textContent ?? "";
}
