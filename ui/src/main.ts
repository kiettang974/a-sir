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
import { icon } from "./icons";
import { type LocalModelProgress, overflowMenu, progressBar } from "./ipc";
import { notePane } from "./note";
import { settingsPane } from "./settings";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ask, message, open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

type JobState =
  | { kind: "queued" }
  | { kind: "running"; percent: number; eta_s: number | null }
  | { kind: "done"; txt: string; srt: string }
  | { kind: "failed"; message: string }
  | { kind: "cancelled" };
type Job = {
  id: number;
  source: string;
  title: string | null;
  model?: string | null;
  group?: string | null;
  state: JobState;
};
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
type Queue = { jobs: Job[]; groups: string[]; paused: boolean; busy: boolean };
type Snapshot = Queue & {
  modelsReady: boolean;
  outputDir: string;
  audioExtensions: string[];
  modelLabels: Record<string, string>;
};
type Line = { start: string; text: string };

/** Formats the core can read; it refuses anything else, so the picker and drop zone match. */
let audioExtensions: string[] = [];
let modelLabels: Record<string, string> = {};
const root = document.querySelector<HTMLDivElement>("#app")!;

let jobs: Job[] = [];
let groups: string[] = [];
let paused = false;
/** Work is waiting or running and not paused, as the core reports it. */
let busy = false;
let modelsReady = false;
let selected: number | null = null;
let openMenuJobId: number | null = null;
let creatingGroup = false;
let newGroupName = "";
const collapsedGroups = new Set<string>();
type PointerDrag = {
  jobId: number;
  label: string;
  startX: number;
  startY: number;
  active: boolean;
  sourceEl: HTMLElement;
  ghost: HTMLElement | null;
  targetGroup: string | null | undefined;
};
let pointerDrag: PointerDrag | null = null;
let suppressClickOnce = false;
let pendingQueueRender = false;
let transcript: { id: number; text: string; lines: Line[] } | null = null;
let readerNote = "";
let view: "reader" | "memos" | "settings" | "note" = "reader";
let pendingSettingsTab: "speech" | "ai" | "minutes" | "glossary" | "update" | undefined;
let memos: MemoList | null = null;
let memoFilter = "";
const memoChosen = new Set<string>();
let download: { percent: number } | { error: string } | null = null;

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

function jobStatusLine(job: Job): string {
  const base = statusText(job.state);
  const tag = job.model ? modelLabels[job.model] ?? job.model : "";
  return tag ? `${base} · ${tag}` : base;
}

async function rerunJob(id: number) {
  try {
    const newId = await invoke<number>("rerun", { id });
    select(newId);
  } catch (e) {
    await message(String(e), { title: "Chưa chạy lại được", kind: "warning" });
  }
}

function jobRow(job: Job): HTMLElement {
  const { state } = job;
  const open = el(
    "button",
    {
      className: "job-open",
      onclick: () => {
        if (suppressClickOnce) return;
        select(job.id);
      },
    },
    el("span", { className: "job-name", textContent: jobName(job) }),
    el("span", { className: "job-status", textContent: jobStatusLine(job) }),
    state.kind === "running"
      ? progressBar(state.percent)
      : null,
  );
  const menuItems: { label: string; danger?: boolean; run: () => void }[] = [];
  if (state.kind === "running") {
    menuItems.push({ label: "Huỷ", danger: true, run: () => invoke("cancel", { id: job.id }) });
  } else if (state.kind === "queued") {
    menuItems.push({ label: "Xoá", danger: true, run: () => forget(job.id) });
  } else if (state.kind === "failed" || state.kind === "cancelled") {
    menuItems.push({ label: "Thử lại", run: () => invoke("retry", { id: job.id }) });
    menuItems.push({ label: "Xoá", danger: true, run: () => forget(job.id) });
  } else {
    menuItems.push({ label: "Chép lời lại", run: () => rerunJob(job.id) });
    menuItems.push({ label: "Xoá", danger: true, run: () => forget(job.id) });
  }

  const isOpen = openMenuJobId === job.id;
  const menuBtn = el(
    "button",
    {
      className: `job-more${isOpen ? " open" : ""}`,
      title: "Thao tác",
      ariaLabel: `Thao tác cho ${jobName(job)}`,
      onclick: (e: MouseEvent) => {
        e.stopPropagation();
        openMenuJobId = isOpen ? null : job.id;
        const q = root.querySelector(".queue");
        if (q) q.replaceWith(queuePane());
      },
    },
    icon("more", 15),
  );
  menuBtn.setAttribute("aria-expanded", String(isOpen));

  const menu = isOpen
    ? el(
        "div",
        { className: "job-menu", role: "menu" },
        ...menuItems.map((item) =>
          el("button", {
            className: `job-menu-item${item.danger ? " danger" : ""}`,
            role: "menuitem",
            textContent: item.label,
            onclick: (e: MouseEvent) => {
              e.stopPropagation();
              openMenuJobId = null;
              item.run();
            },
          }),
        ),
      )
    : null;

  const row = el(
    "li",
    {
      className: `job${job.id === selected ? " selected" : ""}${state.kind === "failed" ? " failed" : ""}`,
      onpointerdown: (e: PointerEvent) => {
        if (e.button !== 0 || groups.length === 0) return;
        if ((e.target as Element | null)?.closest(".job-menu-wrap")) return;
        pointerDrag = {
          jobId: job.id,
          label: jobName(job),
          startX: e.clientX,
          startY: e.clientY,
          active: false,
          sourceEl: row,
          ghost: null,
          targetGroup: undefined,
        };
      },
    },
    open,
    el("div", { className: "job-menu-wrap" }, menuBtn, menu),
  );
  return row;
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

async function addFiles(group?: string) {
  const picked = await open({
    multiple: true,
    filters: [{ name: "Âm thanh và video", extensions: audioExtensions }],
  });
  if (picked) await enqueue(picked, group);
}

/** The core refuses files already waiting or in a format it can't read, and says why. */
async function reportSkipped(skipped: string[]) {
  if (skipped.length) {
    await message(skipped.join("\n"), { title: "Vài file chưa thêm được", kind: "warning" });
  }
}

async function enqueue(paths: string[], group?: string) {
  await reportSkipped(await invoke<string[]>("enqueue", { paths, group: group ?? null }));
}

function refreshQueueOnly() {
  const q = root.querySelector(".queue");
  if (q) q.replaceWith(queuePane());
}

async function submitNewGroup() {
  const trimmed = newGroupName.trim();
  if (!trimmed) return;
  try {
    await invoke<string>("create_group", { name: trimmed });
    creatingGroup = false;
    newGroupName = "";
  } catch (e) {
    await message(String(e), { title: "Chưa tạo được nhóm", kind: "warning" });
  }
}

async function moveJobToGroup(id: number, group: string | null) {
  try {
    await invoke("set_job_group", { id, group });
  } catch (e) {
    await message(String(e), { title: "Chưa chuyển nhóm được", kind: "warning" });
  }
}

async function removeGroup(name: string) {
  const yes = await ask(
    `Xoá nhóm "${name}"? Các bản ghi bên trong sẽ chuyển về mục Chưa phân nhóm; file đã lưu trên máy vẫn được giữ nguyên.`,
    { title: "Xoá nhóm?", kind: "warning", okLabel: "Xoá nhóm", cancelLabel: "Giữ lại" },
  );
  if (!yes) return;
  try {
    await invoke("delete_group", { name });
    collapsedGroups.delete(name);
  } catch (e) {
    await message(String(e), { title: "Chưa xoá được nhóm", kind: "warning" });
  }
}

async function clearQueue() {
  const removable = jobs.filter((j) => j.state.kind !== "running");
  if (removable.length === 0) return;
  const yes = await ask(
    `Xoá ${removable.length} bản ghi khỏi danh sách? File bản chép lời đã lưu trên máy vẫn được giữ nguyên.`,
    { title: "Dọn danh sách bản ghi?", kind: "warning", okLabel: "Xoá", cancelLabel: "Giữ lại" },
  );
  if (!yes) return;
  const removed = await invoke<number[]>("clear_queue");
  if (selected !== null && removed.includes(selected)) {
    selected = null;
    transcript = null;
  }
}

function clearDragHighlight() {
  root.querySelectorAll(".group-block.drag-over").forEach((n) => n.classList.remove("drag-over"));
}

function setDragHighlight(target: HTMLElement | null) {
  root.querySelectorAll(".group-block.drag-over").forEach((n) => {
    if (n !== target) n.classList.remove("drag-over");
  });
  if (target) target.classList.add("drag-over");
}

function queueList(): HTMLElement {
  if (groups.length === 0) {
    return el("ul", { className: "jobs" }, ...jobs.map(jobRow));
  }

  const container = el("div", { className: "jobs grouped-jobs" });

  for (const name of groups) {
    const groupJobs = jobs.filter((j) => j.group === name);
    const collapsed = collapsedGroups.has(name);
    const block = el(
      "div",
      { className: "group-block" },
      el(
        "div",
        { className: "group-head" },
        el(
          "button",
          {
            className: "group-toggle",
            onclick: () => {
              if (collapsed) collapsedGroups.delete(name);
              else collapsedGroups.add(name);
              refreshQueueOnly();
            },
          },
          icon(collapsed ? "chevronRight" : "chevronDown", 13),
          icon("folder", 14),
          el("span", { className: "group-name", textContent: name }),
          el("span", { className: "group-count", textContent: String(groupJobs.length) }),
        ),
        el(
          "div",
          { className: "group-actions" },
          el(
            "button",
            {
              className: "icon-btn-sm",
              title: `Thêm file vào nhóm "${name}"`,
              ariaLabel: `Thêm file vào nhóm ${name}`,
              onclick: () => addFiles(name),
            },
            icon("plus", 13),
          ),
          el(
            "button",
            {
              className: "icon-btn-sm",
              title: `Mở thư mục "${name}" trong Finder`,
              ariaLabel: `Mở thư mục ${name}`,
              onclick: () => invoke("open_group_dir", { name }),
            },
            icon("folder", 13),
          ),
          el(
            "button",
            {
              className: "icon-btn-sm danger",
              title: `Xoá nhóm "${name}"`,
              ariaLabel: `Xoá nhóm ${name}`,
              onclick: () => removeGroup(name),
            },
            icon("trash", 13),
          ),
        ),
      ),
      collapsed
        ? null
        : groupJobs.length > 0
          ? el("ul", { className: "group-jobs" }, ...groupJobs.map(jobRow))
          : el("p", { className: "group-empty", textContent: "Kéo bản ghi hoặc thả file vào đây" }),
    );
    block.dataset.dropGroup = name;
    container.append(block);
  }

  const ungrouped = jobs.filter((j) => !j.group || !groups.includes(j.group));
  if (ungrouped.length > 0 || jobs.length > 0) {
    const ungroupedBlock = el(
      "div",
      { className: "group-block ungrouped" },
      el(
        "div",
        { className: "group-head ungrouped-head" },
        el("span", { className: "group-name", textContent: "Chưa phân nhóm" }),
        el("span", { className: "group-count", textContent: String(ungrouped.length) }),
      ),
      ungrouped.length > 0
        ? el("ul", { className: "group-jobs" }, ...ungrouped.map(jobRow))
        : null,
    );
    ungroupedBlock.dataset.dropGroup = "";
    container.append(ungroupedBlock);
  }

  return container;
}

function queuePane(): HTMLElement {
  const hasRemovable = jobs.some((j) => j.state.kind !== "running");
  let groupInput: HTMLInputElement | null = null;
  if (creatingGroup) {
    groupInput = el("input", {
      className: "field group-input",
      placeholder: "Tên nhóm / thư mục…",
      value: newGroupName,
      ariaLabel: "Tên nhóm hoặc thư mục mới",
      oninput: (e: Event) => {
        newGroupName = (e.target as HTMLInputElement).value;
      },
      onkeydown: (e: KeyboardEvent) => {
        if (e.key === "Enter") {
          e.preventDefault();
          submitNewGroup();
        } else if (e.key === "Escape") {
          creatingGroup = false;
          newGroupName = "";
          refreshQueueOnly();
        }
      },
    });
    queueMicrotask(() => groupInput?.focus());
  }

  return el(
    "section",
    { className: "queue" },
    el(
      "header",
      { className: "queue-head" },
      el(
        "div",
        { className: "queue-title" },
        el("h1", { textContent: "Bản ghi âm" }),
        el(
          "button",
          {
            className: "icon-btn-sm",
            title: "Thêm file âm thanh",
            ariaLabel: "Thêm file âm thanh",
            onclick: () => addFiles(),
          },
          icon("plus", 15),
        ),
      ),
      el(
        "button",
        {
          className: `icon-btn${creatingGroup ? " active" : ""}`,
          title: "Tạo nhóm / thư mục mới",
          ariaLabel: "Tạo nhóm hoặc thư mục mới",
          onclick: () => {
            creatingGroup = !creatingGroup;
            if (!creatingGroup) newGroupName = "";
            refreshQueueOnly();
          },
        },
        icon("folderPlus", 15),
      ),
      busy || paused
        ? el(
            "button",
            {
              className: "icon-btn",
              title: paused ? "Tiếp tục" : "Tạm dừng",
              ariaLabel: paused ? "Tiếp tục hàng chờ" : "Tạm dừng hàng chờ",
              onclick: () => invoke("set_paused", { paused: !paused }),
            },
            icon(paused ? "play" : "pause", 15),
          )
        : null,
      hasRemovable
        ? el(
            "button",
            {
              className: "icon-btn danger",
              title: "Xoá hàng chờ",
              ariaLabel: "Xoá hàng chờ bản ghi",
              onclick: clearQueue,
            },
            icon("trash", 15),
          )
        : null,
    ),
    creatingGroup && groupInput
      ? el(
          "div",
          { className: "group-create" },
          groupInput,
          el("button", { className: "link", textContent: "Tạo", onclick: submitNewGroup }),
          el("button", {
            className: "link",
            textContent: "Huỷ",
            onclick: () => {
              creatingGroup = false;
              newGroupName = "";
              refreshQueueOnly();
            },
          }),
        )
      : null,
    queueList(),
    el(
      "footer",
      { className: "queue-foot" },
      el(
        "button",
        {
          className: "add",
          title: "Mở ứng dụng Ghi âm mặc định của máy",
          onclick: async () => {
            try {
              await invoke("open_voice_recorder");
            } catch (e) {
              await message(String(e), { title: "Chưa mở được app Ghi âm", kind: "warning" });
            }
          },
        },
        icon("mic", 15),
        el("span", { textContent: "Mở app Ghi âm" }),
      ),
      el(
        "button",
        {
          className: `icon-btn${view === "memos" ? " active" : ""}`,
          title: "Nhập từ thư viện Ghi âm",
          ariaLabel: "Mở danh sách từ thư viện Ghi âm",
          onclick: openMemos,
        },
        icon("folder", 16),
      ),
      el(
        "button",
        {
          className: `icon-btn push-right${view === "settings" ? " active" : ""}`,
          title: "Cấu hình",
          ariaLabel: "Mở Cấu hình",
          onclick: () => {
            view = "settings";
            selected = null;
            render();
          },
        },
        icon("sliders", 16),
      ),
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
            "Hệ thống yêu cầu quyền Truy cập toàn bộ ổ đĩa để đọc bản ghi từ app Ghi âm. Ứng dụng chỉ đọc các bản ghi bạn chọn và xử lý hoàn toàn trên máy.",
        }),
        el(
          "ol",
          { className: "steps" },
          el("li", { textContent: "Mở Cài đặt hệ thống bằng nút bên dưới." }),
          el("li", { textContent: "Bật A-Sir trong danh sách. Nếu chưa có, bấm dấu + và chọn app." }),
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
        el("h2", { textContent: "Chưa có bản ghi nào trong app Ghi âm" }),
        el("p", { textContent: "Bản ghi mới sẽ hiện ở đây sau khi đồng bộ." }),
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
      el(
        "div",
        { className: "reader-actions" },
        el(
          "button",
          {
            className: "btn-primary",
            title: "Mở bảng tóm tắt biên bản",
            onclick: () => {
              view = "note";
              render();
            },
          },
          icon("summary", 15),
          el("span", { textContent: "Tóm tắt" }),
        ),
        el(
          "button",
          {
            className: "btn-ghost",
            title: "Chép toàn bộ bản chép lời",
            disabled: text === null,
            onclick: async () => {
              await writeText(text ?? "");
              note("Đã chép toàn bộ bản chép lời.");
            },
          },
          icon("copy", 15),
          el("span", { textContent: "Chép" }),
        ),
        overflowMenu("Thao tác khác cho bản chép lời", [
          {
            label: "Chép lời lại",
            title: "Chép lời lại bằng model đang chọn, vẫn giữ bản cũ",
            onClick: () => rerunJob(job.id),
          },
          {
            label: "Làm sạch bản chép",
            title: "Bỏ câu rác, gộp từ lặp và sửa từ nghe nhầm theo danh sách từ vựng riêng",
            onClick: async () => {
              try {
                const path = await invoke<string>("clean_transcript", { id: job.id });
                note(`Đã tạo ${baseName(path)} cùng thư mục với bản gốc.`);
              } catch (e) {
                note(String(e));
              }
            },
          },
          {
            label: "Mở trong Finder",
            onClick: () => revealItemInDir(state.txt),
          },
        ]),
      ),
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
          "Chép lời tiếng Việt ngay trên máy, không cần mạng. Mỗi bản ghi cho ra một file chữ (.txt) và một file phụ đề (.srt).",
      }),
      el("button", { className: "add", textContent: "Thêm file", onclick: () => addFiles() }),
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
      el("h2", { textContent: "Tải gói nhận dạng giọng nói" }),
      el("p", {
        textContent:
          "Cần tải dữ liệu một lần đầu để máy tự chép lời không cần mạng. Nếu rớt mạng giữa chừng, lần sau sẽ tải tiếp.",
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
            progressBar(running),
          ),
    ),
  );
}

async function startDownload() {
  download = { percent: 0 };
  render();
  try {
    await invoke("download_local_model", { modelId: null });
    modelsReady = true;
    download = null;
  } catch (e) {
    download = { error: String(e) };
  }
  render();
}

function rightPane(): HTMLElement {
  if (view === "memos") return memosPane();
  if (view === "settings") {
    const tab = pendingSettingsTab;
    pendingSettingsTab = undefined;
    return settingsPane(tab);
  }
  const job = jobs.find((j) => j.id === selected);
  if (view === "note" && job) {
    return notePane(job.id, jobName(job), {
      openSettings: () => {
        view = "settings";
        selected = null;
        pendingSettingsTab = "ai";
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
  groups = initial.groups ?? [];
  paused = initial.paused;
  busy = initial.busy;
  modelsReady = initial.modelsReady;
  audioExtensions = initial.audioExtensions;
  modelLabels = initial.modelLabels ?? {};
  render();

  await listen<Queue>("queue", ({ payload }) => {
    const wasDone = jobs.find((j) => j.id === selected)?.state.kind === "done";
    jobs = payload.jobs;
    groups = payload.groups ?? [];
    paused = payload.paused;
    busy = payload.busy;
    if (pointerDrag?.active) {
      pendingQueueRender = true;
      return;
    }
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
  await listen<LocalModelProgress>("local-model-progress", ({ payload }) => {
    if (download && "percent" in download && download.percent !== payload.percent) {
      download = { percent: payload.percent };
      render();
    }
  });
  const groupBlockAtPoint = (pos?: { x: number; y: number }): HTMLElement | null => {
    if (!pos) return null;
    const dpr = window.devicePixelRatio || 1;
    return (
      document.elementFromPoint(pos.x / dpr, pos.y / dpr)?.closest<HTMLElement>("[data-drop-group]") ??
      document.elementFromPoint(pos.x, pos.y)?.closest<HTMLElement>("[data-drop-group]") ??
      null
    );
  };
  await getCurrentWebview().onDragDropEvent(({ payload }) => {
    const hovering = payload.type === "enter" || payload.type === "over";
    document.body.classList.toggle("dropping", hovering);
    if (hovering && modelsReady) {
      setDragHighlight(groupBlockAtPoint(payload.position));
    } else if (payload.type === "leave") {
      clearDragHighlight();
    } else if (payload.type === "drop" && modelsReady) {
      const hit = groupBlockAtPoint(payload.position);
      const targetGroup = hit?.dataset.dropGroup || undefined;
      clearDragHighlight();
      // Folders and stray files dropped along with recordings are left out quietly.
      const paths = payload.paths.filter((p) =>
        audioExtensions.includes(p.split(".").pop()?.toLowerCase() ?? ""),
      );
      if (paths.length) enqueue(paths, targetGroup);
    }
  });
}

  window.addEventListener("pointermove", (e: PointerEvent) => {
    if (!pointerDrag) return;
    if (!pointerDrag.active) {
      if (Math.hypot(e.clientX - pointerDrag.startX, e.clientY - pointerDrag.startY) < 6) return;
      pointerDrag.active = true;
      pointerDrag.sourceEl.classList.add("is-dragging");
      document.body.classList.add("dragging-job");
      const ghost = el("div", { className: "drag-ghost", textContent: pointerDrag.label });
      document.body.append(ghost);
      pointerDrag.ghost = ghost;
    }
    if (pointerDrag.ghost) {
      pointerDrag.ghost.style.transform = `translate(${e.clientX + 12}px, ${e.clientY + 12}px)`;
    }
    const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-drop-group]") ?? null;
    setDragHighlight(hit);
    pointerDrag.targetGroup = hit ? (hit.dataset.dropGroup || null) : undefined;
  });

  const finishPointerDrag = (applyDrop: boolean) => {
    if (!pointerDrag) return;
    const drag = pointerDrag;
    pointerDrag = null;
    if (!drag.active) return;
    suppressClickOnce = true;
    setTimeout(() => {
      suppressClickOnce = false;
    }, 0);
    drag.sourceEl.classList.remove("is-dragging");
    drag.ghost?.remove();
    document.body.classList.remove("dragging-job");
    clearDragHighlight();
    if (applyDrop && drag.targetGroup !== undefined) {
      const current = jobs.find((j) => j.id === drag.jobId);
      if ((current?.group ?? null) !== drag.targetGroup) {
        moveJobToGroup(drag.jobId, drag.targetGroup);
      }
    }
    if (pendingQueueRender) {
      pendingQueueRender = false;
      if (view !== "reader" && modelsReady) {
        refreshQueueOnly();
        refreshMemoControls();
      } else {
        render();
      }
    }
  };
  window.addEventListener("pointerup", () => finishPointerDrag(true));
  window.addEventListener("pointercancel", () => finishPointerDrag(false));

  document.addEventListener("click", () => {
    if (openMenuJobId !== null) {
      openMenuJobId = null;
      const q = root.querySelector(".queue");
      if (q) q.replaceWith(queuePane());
    }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && openMenuJobId !== null) {
      openMenuJobId = null;
      const q = root.querySelector(".queue");
      if (q) q.replaceWith(queuePane());
    }
  });
  window.addEventListener("asr-model-label", (e) => {
    const detail = (e as CustomEvent<{ id: string; label: string }>).detail;
    if (detail?.id && detail?.label) {
      modelLabels[detail.id] = detail.label;
      const q = root.querySelector(".queue");
      if (q) q.replaceWith(queuePane());
    }
  });

  main();
