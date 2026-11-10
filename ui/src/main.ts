import "@fontsource/be-vietnam-pro/vietnamese-400.css";
import "@fontsource/be-vietnam-pro/latin-400.css";
import "@fontsource/be-vietnam-pro/vietnamese-500.css";
import "@fontsource/be-vietnam-pro/latin-500.css";
import "@fontsource/be-vietnam-pro/vietnamese-600.css";
import "@fontsource/be-vietnam-pro/latin-600.css";
import "@fontsource/literata/vietnamese-400.css";
import "@fontsource/literata/latin-400.css";
import "./style.css";

import { el } from "./dom";
import { notePane } from "./note";
import { settingsPane } from "./settings";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { message, open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

type JobState =
  | { kind: "queued" }
  | { kind: "running"; percent: number; eta_s: number | null }
  | { kind: "done"; txt: string; srt: string }
  | { kind: "failed"; message: string }
  | { kind: "cancelled" };
type Job = { id: number; source: string; title: string | null; state: JobState };
type Recording = {
  path: string;
  title: string;
  recordedAt: number | null;
  durationS: number | null;
  onDisk: boolean;
};
type MemoList =
  | { status: "ok"; recordings: Recording[] }
  | { status: "noPermission" }
  | { status: "failed"; message: string };
type Queue = { jobs: Job[]; paused: boolean; busy: boolean };
type Snapshot = Queue & { modelsReady: boolean; outputDir: string; audioExtensions: string[] };
type Line = { start: string; text: string };

/** Formats the core can read; it refuses anything else, so the picker and drop zone match. */
let audioExtensions: string[] = [];
const root = document.querySelector<HTMLDivElement>("#app")!;

let jobs: Job[] = [];
let paused = false;
/** Work is waiting or running and not paused, as the core reports it. */
let busy = false;
let modelsReady = false;
let selected: number | null = null;
let transcript: { id: number; text: string; lines: Line[] } | null = null;
let readerNote = "";
let view: "reader" | "memos" | "settings" | "note" = "reader";
let memos: MemoList | null = null;
let memoFilter = "";
const memoChosen = new Set<string>();
let download: { percent: number } | { error: string } | null = null;

function bar(percent: number): HTMLElement {
  const fill = el("i");
  fill.style.width = `${percent}%`;
  return el("div", { className: "bar" }, fill);
}

const jobName = (job: Job) => job.title ?? baseName(job.source);

const baseName = (path: string) => path.split("/").pop() ?? path;

function duration(seconds: number): string {
  if (seconds < 60) return "dưới 1 phút";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} phút`;
  return `${Math.floor(minutes / 60)} giờ ${minutes % 60} phút`;
}

function statusText(state: JobState): string {
  switch (state.kind) {
    case "queued":
      return paused ? "Đang tạm dừng" : "Đang chờ";
    case "running": {
      if (paused) return `Đang tạm dừng ở ${state.percent}%`;
      const eta = state.eta_s === null ? "" : `, còn khoảng ${duration(state.eta_s)}`;
      return `${state.percent}%${eta}`;
    }
    case "done":
      return "Đã xong";
    case "failed":
      return state.message;
    case "cancelled":
      return "Đã huỷ";
  }
}

function jobRow(job: Job): HTMLElement {
  const { state } = job;
  const open = el(
    "button",
    { className: "job-open", onclick: () => select(job.id) },
    el("span", { className: "job-name", textContent: jobName(job) }),
    el("span", { className: "job-status", textContent: statusText(state) }),
    state.kind === "running"
      ? bar(state.percent)
      : null,
  );
  const action =
    state.kind === "queued" || state.kind === "running"
      ? { label: "Huỷ", run: () => invoke("cancel", { id: job.id }) }
      : state.kind === "failed" || state.kind === "cancelled"
        ? { label: "Thử lại", run: () => invoke("retry", { id: job.id }) }
        : { label: "Bỏ", run: () => forget(job.id) };
  return el(
    "li",
    {
      className: `job${job.id === selected ? " selected" : ""}${state.kind === "failed" ? " failed" : ""}`,
    },
    open,
    el("button", { className: "job-action", textContent: action.label, onclick: action.run }),
  );
}

function forget(id: number) {
  if (selected === id) {
    selected = null;
    transcript = null;
  }
  invoke("remove", { id });
}

async function select(id: number) {
  view = "reader";
  selected = id;
  readerNote = "";
  transcript = null;
  render();
  const job = jobs.find((j) => j.id === id);
  if (job?.state.kind !== "done") return;
  try {
    const read = await invoke<{ text: string; lines: Line[] }>("read_transcript", { id });
    if (selected === id) transcript = { id, ...read };
  } catch (e) {
    readerNote = String(e);
  }
  render();
}

async function addFiles() {
  const picked = await open({
    multiple: true,
    filters: [{ name: "Âm thanh và video", extensions: audioExtensions }],
  });
  if (picked) await enqueue(picked);
}

/** The core refuses files already waiting or in a format it can't read, and says why. */
async function reportSkipped(skipped: string[]) {
  if (skipped.length) {
    await message(skipped.join("\n"), { title: "Có file không được thêm", kind: "warning" });
  }
}

async function enqueue(paths: string[]) {
  await reportSkipped(await invoke<string[]>("enqueue", { paths }));
}

function queuePane(): HTMLElement {
  return el(
    "section",
    { className: "queue" },
    el(
      "header",
      { className: "queue-head" },
      el("h1", { textContent: "Bản ghi âm" }),
      busy || paused
        ? el("button", {
            className: "link",
            textContent: paused ? "Tiếp tục" : "Tạm dừng",
            onclick: () => invoke("set_paused", { paused: !paused }),
          })
        : null,
    ),
    el("ul", { className: "jobs" }, ...jobs.map(jobRow)),
    el(
      "footer",
      { className: "queue-foot" },
      el("button", { className: "add", textContent: "Thêm file", onclick: addFiles }),
      el("button", { className: "link", textContent: "Từ app Ghi âm", onclick: openMemos }),
      el("button", {
        className: "link",
        textContent: "Cấu hình",
        onclick: () => {
          view = "settings";
          selected = null;
          render();
        },
      }),
    ),
  );
}

async function openMemos() {
  view = "memos";
  selected = null;
  memos = null;
  render();
  memos = await invoke<MemoList>("list_voice_memos");
  render();
}

function clock(seconds: number): string {
  const s = Math.round(seconds);
  const mm = String(Math.floor(s / 60) % 60).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return s >= 3600 ? `${Math.floor(s / 3600)}:${mm}:${ss}` : `${Math.floor(s / 60)}:${ss}`;
}

const inQueue = (path: string) =>
  jobs.some((j) => j.source === path && j.state.kind !== "failed" && j.state.kind !== "cancelled");

function matchesFilter(r: Recording): boolean {
  const query = memoFilter.trim().normalize("NFC").toLowerCase();
  return !query || r.title.normalize("NFC").toLowerCase().includes(query);
}

function memoRow(r: Recording): HTMLElement {
  const blocked = !r.onDisk || inQueue(r.path);
  const box = el("input", {
    type: "checkbox",
    checked: memoChosen.has(r.path) && !blocked,
    disabled: blocked,
    onchange: () => {
      if (box.checked) memoChosen.add(r.path);
      else memoChosen.delete(r.path);
      refreshMemoControls();
    },
  });
  const when = r.recordedAt === null ? "" : new Date(r.recordedAt * 1000).toLocaleDateString("vi-VN");
  const hint = !r.onDisk ? "Chưa tải về từ iCloud" : inQueue(r.path) ? "Đã có trong danh sách" : "";
  return el(
    "li",
    {},
    el(
      "label",
      { className: `memo${blocked ? " blocked" : ""}` },
      box,
      el("span", { className: "memo-title", textContent: r.title }),
      el("span", { className: "memo-meta", textContent: hint || when }),
      el("span", { className: "memo-length", textContent: r.durationS === null ? "" : clock(r.durationS) }),
    ),
  );
}

function memoPick(recordings: Recording[]): Recording[] {
  return recordings.filter((r) => memoChosen.has(r.path) && r.onDisk && !inQueue(r.path));
}

/** Updates the rows and buttons only. The filter box stays, so Vietnamese typing isn't interrupted. */
function refreshMemoControls() {
  if (memos?.status !== "ok") return;
  const list = root.querySelector(".memos");
  const start = root.querySelector<HTMLButtonElement>(".memo-start");
  const all = root.querySelector<HTMLButtonElement>(".memo-all");
  if (!list || !start || !all) return;
  const visible = memos.recordings.filter(matchesFilter);
  const top = list.scrollTop;
  list.replaceChildren(...visible.map(memoRow));
  list.scrollTop = top;
  const picked = memoPick(memos.recordings).length;
  start.textContent = picked ? `Chép lời ${picked} bản ghi` : "Chọn bản ghi để chép lời";
  start.disabled = picked === 0;
  const selectable = visible.filter((r) => r.onDisk && !inQueue(r.path));
  all.textContent =
    selectable.length && selectable.every((r) => memoChosen.has(r.path)) ? "Bỏ chọn" : "Chọn tất cả";
}

function memosPane(): HTMLElement {
  if (memos === null) {
    return el("section", { className: "invite" }, el("div", {}, el("p", { textContent: "Đang đọc danh sách bản ghi." })));
  }
  if (memos.status === "noPermission") {
    return el(
      "section",
      { className: "invite" },
      el(
        "div",
        {},
        el("h2", { textContent: "Cho phép app đọc bản ghi của Ghi âm" }),
        el("p", {
          textContent:
            "macOS chỉ cho đọc thư mục của app Ghi âm khi có quyền Truy cập toàn bộ ổ đĩa. Quyền này cho ASR Studio đọc được gần như mọi dữ liệu trên máy. App chỉ dùng nó để đọc bản ghi bạn chọn, và âm thanh không rời khỏi máy.",
        }),
        el(
          "ol",
          { className: "steps" },
          el("li", { textContent: "Mở Cài đặt hệ thống bằng nút bên dưới." }),
          el("li", { textContent: "Bật ASR Studio trong danh sách. Nếu chưa có, bấm dấu + và chọn app." }),
          el("li", { textContent: "Quay lại đây và mở lại app." }),
        ),
        el(
          "div",
          { className: "actions" },
          el("button", {
            className: "add",
            textContent: "Mở Cài đặt hệ thống",
            onclick: () => invoke("open_full_disk_settings"),
          }),
          el("button", { className: "link", textContent: "Mở lại app", onclick: () => invoke("relaunch") }),
        ),
      ),
    );
  }
  if (memos.status === "failed") {
    return el(
      "section",
      { className: "invite" },
      el(
        "div",
        {},
        el("h2", { textContent: "Không đọc được danh sách bản ghi" }),
        el("p", { className: "problem problem-text", textContent: memos.message }),
        el("button", { className: "add", textContent: "Thử lại", onclick: openMemos }),
      ),
    );
  }
  if (memos.recordings.length === 0) {
    return el(
      "section",
      { className: "invite" },
      el(
        "div",
        {},
        el("h2", { textContent: "App Ghi âm chưa có bản ghi nào trên máy này" }),
        el("p", { textContent: "Bản ghi mới sẽ hiện ở đây sau khi đồng bộ về máy." }),
      ),
    );
  }
  const recordings = memos.recordings;
  const pane = el(
    "section",
    { className: "reader" },
    el(
      "header",
      { className: "reader-head memo-head" },
      el("h2", { textContent: `Bản ghi trong app Ghi âm (${recordings.length})` }),
      el("input", {
        className: "memo-filter",
        type: "search",
        placeholder: "Tìm theo tên",
        ariaLabel: "Tìm bản ghi theo tên",
        value: memoFilter,
        oninput: (e: Event) => {
          memoFilter = (e.target as HTMLInputElement).value;
          refreshMemoControls();
        },
      }),
      el("button", {
        className: "link memo-all",
        onclick: () => {
          const selectable = recordings.filter((r) => matchesFilter(r) && r.onDisk && !inQueue(r.path));
          const clear = selectable.every((r) => memoChosen.has(r.path));
          for (const r of selectable) {
            if (clear) memoChosen.delete(r.path);
            else memoChosen.add(r.path);
          }
          refreshMemoControls();
        },
      }),
    ),
    el("ul", { className: "memos" }),
    el(
      "footer",
      { className: "memo-foot" },
      el("button", {
        className: "add memo-start",
        onclick: async () => {
          const choices = memoPick(recordings).map(({ path, title }) => ({ path, title }));
          for (const { path } of choices) memoChosen.delete(path);
          await reportSkipped(await invoke<string[]>("enqueue_voice_memos", { choices }));
          refreshMemoControls();
        },
      }),
    ),
  );
  return pane;
}

function transcriptLines(lines: Line[]): HTMLElement[] {
  return lines.map(({ start, text }) =>
    el(
      "div",
      { className: "line" },
      el("time", { textContent: start.replace(/^00:/, "") }),
      el("p", { textContent: text }),
    ),
  );
}

function readerPane(): HTMLElement {
  const job = jobs.find((j) => j.id === selected);
  if (!job) return invite();
  const state = job.state;
  if (state.kind !== "done") {
    const title =
      state.kind === "failed"
        ? "Chưa chép lời được file này"
        : state.kind === "cancelled"
          ? "File này đã được huỷ"
          : "Bản chép lời sẽ hiện ở đây khi xong";
    return el(
      "section",
      { className: "invite" },
      el(
        "div",
        {},
        el("h2", { textContent: title }),
        el("p", { className: state.kind === "failed" ? "problem problem-text" : "", textContent: `${jobName(job)}: ${statusText(state)}` }),
      ),
    );
  }
  const shown = transcript?.id === job.id ? transcript : null;
  const text = shown?.text ?? null;
  return el(
    "section",
    { className: "reader" },
    el(
      "header",
      { className: "reader-head" },
      el("h2", { textContent: jobName(job) }),
      el("button", {
        className: "link",
        textContent: "Bảng tóm tắt",
        onclick: () => {
          view = "note";
          render();
        },
      }),
      el("button", {
        className: "link",
        textContent: "Chép toàn bộ",
        disabled: text === null,
        onclick: async () => {
          await writeText(text ?? "");
          note("Đã chép toàn bộ vào bộ nhớ tạm.");
        },
      }),
      el("button", {
        className: "link",
        textContent: "Tạo bản đã làm sạch",
        title: "Bỏ dòng quảng cáo kênh mà máy nghe nhầm, gộp từ lặp lại nhiều lần",
        onclick: async () => {
          try {
            const path = await invoke<string>("clean_transcript", { id: job.id });
            note(`Đã tạo ${baseName(path)}, cùng thư mục với bản gốc.`);
          } catch (e) {
            note(String(e));
          }
        },
      }),
      el("button", {
        className: "link",
        textContent: "Mở trong Finder",
        onclick: () => revealItemInDir(state.txt),
      }),
    ),
    readerNote ? el("p", { className: "note", textContent: readerNote }) : null,
    el("article", { className: "transcript" }, ...(shown === null ? [] : transcriptLines(shown.lines))),
  );
}

function note(message: string) {
  readerNote = message;
  render();
}

function invite(): HTMLElement {
  return el(
    "section",
    { className: "invite" },
    el(
      "div",
      {},
      el("h2", { textContent: jobs.length ? "Chọn một file để đọc" : "Thả file ghi âm vào đây" }),
      el("p", {
        textContent:
          "Chép lời tiếng Việt ngay trên máy này, không gửi âm thanh đi đâu. Mỗi file cho ra một bản .txt và một bản phụ đề .srt.",
      }),
      el("button", { className: "add", textContent: "Thêm file", onclick: addFiles }),
    ),
  );
}

function firstRun(): HTMLElement {
  const failed = download && "error" in download ? download.error : null;
  const running = download && "percent" in download ? download.percent : null;
  return el(
    "section",
    { className: "invite" },
    el(
      "div",
      {},
      el("h2", { textContent: "Tải model nhận dạng giọng nói" }),
      el("p", {
        textContent:
          "App cần tải 1,6 GB một lần duy nhất. Sau đó mọi thứ chạy trên máy, không cần mạng. Nếu mất mạng giữa chừng, lần tải sau sẽ chạy tiếp.",
      }),
      failed ? el("p", { className: "problem problem-text", textContent: failed }) : null,
      running === null
        ? el("button", {
            className: "add",
            textContent: failed ? "Thử tải lại" : "Tải model",
            onclick: startDownload,
          })
        : el(
            "div",
            {},
            el("p", { textContent: `Đang tải: ${running}%` }),
            bar(running),
          ),
    ),
  );
}

async function startDownload() {
  download = { percent: 0 };
  render();
  try {
    await invoke("download_models");
    modelsReady = true;
    download = null;
  } catch (e) {
    download = { error: String(e) };
  }
  render();
}

function rightPane(): HTMLElement {
  if (view === "memos") return memosPane();
  if (view === "settings") return settingsPane();
  const job = jobs.find((j) => j.id === selected);
  if (view === "note" && job) {
    return notePane(job.id, jobName(job), {
      openSettings: () => {
        view = "settings";
        selected = null;
        render();
      },
      back: () => select(job.id),
    });
  }
  return readerPane();
}

function render() {
  const scroller = root.querySelector(".transcript");
  const scrollTop = scroller?.scrollTop ?? 0;
  const list = root.querySelector(".jobs");
  const listTop = list?.scrollTop ?? 0;
  const focused = document.activeElement?.textContent;

  root.replaceChildren(
    modelsReady
      ? el("div", { className: "layout" }, queuePane(), rightPane())
      : firstRun(),
  );
  refreshMemoControls();

  const nextScroller = root.querySelector(".transcript");
  if (nextScroller) nextScroller.scrollTop = scrollTop;
  const nextList = root.querySelector(".jobs");
  if (nextList) nextList.scrollTop = listTop;
  if (focused) {
    [...root.querySelectorAll("button")].find((b) => b.textContent === focused)?.focus();
  }
}

async function main() {
  const initial = await invoke<Snapshot>("snapshot");
  jobs = initial.jobs;
  paused = initial.paused;
  busy = initial.busy;
  modelsReady = initial.modelsReady;
  audioExtensions = initial.audioExtensions;
  render();

  await listen<Queue>("queue", ({ payload }) => {
    const wasDone = jobs.find((j) => j.id === selected)?.state.kind === "done";
    jobs = payload.jobs;
    paused = payload.paused;
    busy = payload.busy;
    if (view !== "reader" && modelsReady) {
      // Leave the right pane alone: rebuilding it would lose what is being typed there.
      root.querySelector(".queue")?.replaceWith(queuePane());
      refreshMemoControls();
      return;
    }
    // The job on screen just finished: show its transcript straight away.
    if (selected !== null && !wasDone && jobs.find((j) => j.id === selected)?.state.kind === "done") {
      select(selected);
    } else {
      render();
    }
  });
  await listen<number>("model-progress", ({ payload }) => {
    if (download && "percent" in download && download.percent !== payload) {
      download = { percent: payload };
      render();
    }
  });
  await getCurrentWebview().onDragDropEvent(({ payload }) => {
    document.body.classList.toggle("dropping", payload.type === "enter" || payload.type === "over");
    if (payload.type === "drop" && modelsReady) {
      // Folders and stray files dropped along with recordings are left out quietly.
      const paths = payload.paths.filter((p) =>
        audioExtensions.includes(p.split(".").pop()?.toLowerCase() ?? ""),
      );
      if (paths.length) enqueue(paths);
    }
  });
}

main();
