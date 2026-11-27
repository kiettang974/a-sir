// Mock of @tauri-apps/api/core: answers every command the UI invokes from in-memory state.
//
// URL switches (combine freely):
//   ?state=first-run   models not downloaded yet (first-run screen; "Tải model" simulates a download)
//   ?state=empty       models ready, no jobs
//   (default)          populated queue: done, running, queued, failed
//   ?memos=denied|failed|empty   what list_voice_memos returns
//   ?keys=none         no API keys (note pane asks for one)
//   ?note=saved        load_note returns a saved note instead of null
//   ?paused=1          queue starts paused
//   ?tick=0            freeze progress (stable screenshots)
//   ?open=reader|note|settings|memos   click through to that screen after load (see autopilot.ts)
import { autopilot } from "./autopilot";
import { emitLocal } from "./event";
import {
  AUDIO_EXTENSIONS,
  DRAFT,
  GLOSSARY,
  MODELS,
  OUTPUT_DIR,
  RECORDINGS,
  TRANSCRIPT_LINES,
  TRANSCRIPT_TEXT,
  aiView,
  initialJobs,
  type Draft,
  type Job,
  type Provider,
  type Term,
} from "./fixtures";

const params = new URLSearchParams(location.search);
const screen = params.get("state") ?? "populated";

const state = {
  modelsReady: screen !== "first-run",
  selectedLocalModel: "whisper-large-v3-turbo",
  resourceProfile: "balanced",
  readyLocalModels: new Set<string>(screen !== "first-run" ? ["whisper-large-v3-turbo"] : []),
  paused: params.get("paused") === "1",
  jobs: screen === "populated" ? initialJobs() : ([] as Job[]),
  nextId: 100,
  ai: aiView(params.get("keys") !== "none"),
  glossary: structuredClone(GLOSSARY) as Term[],
  notes: new Map<number, unknown>(),
  diagrams: new Map<number, unknown>(),
};

const LOCAL_MODEL_CATALOG = [
  {
    id: "whisper-large-v3-turbo",
    name: "Whisper Large V3 Turbo",
    description: "Bản chuẩn hiện tại, cân bằng độ chính xác tiếng Việt và code-switching Anh–Việt.",
    source: "ggerganov/whisper.cpp",
    engine: "whisper.cpp",
    bytes: 1_624_555_275,
  },
  {
    id: "whisper-large-v3",
    name: "Whisper Large V3 (Full 1.55B)",
    description: "Bản đầy đủ 1.55B tham số, độ chính xác cao nhất họ Whisper cho họp dài và thuật ngữ tiếng Anh (phù hợp Mac 32GB+ RAM).",
    source: "ggerganov/whisper.cpp",
    engine: "whisper.cpp",
    bytes: 3_095_033_483,
  },
  {
    id: "qwen3-asr-1.7b",
    name: "Qwen3-ASR-1.7B",
    description: "Model thế hệ mới của Qwen (Apache-2.0), mạnh về tiếng Việt và code-switching Anh–Việt, chạy thuần Rust (Accelerate/vDSP).",
    source: "Qwen/Qwen3-ASR-1.7B",
    engine: "qwen-asr (Rust)",
    bytes: 4_703_041_213,
  },
  {
    id: "phowhisper-medium-q5_0",
    name: "PhoWhisper Medium (Q5_0)",
    description: "Model tinh chỉnh chuyên tiếng Việt của VinAI (bản GGML cộng đồng).",
    source: "nhgpv11/PhoWhisper-ggml",
    engine: "whisper.cpp",
    bytes: 539_212_484,
  },
];

function localModelsView() {
  return {
    selected: state.selectedLocalModel,
    resourceProfile: state.resourceProfile,
    cpuCores: 10,
    models: LOCAL_MODEL_CATALOG.map((m) => ({
      ...m,
      ready: state.readyLocalModels.has(m.id),
      selected: m.id === state.selectedLocalModel,
    })),
  };
}

if (params.get("note") === "saved") {
  const html = (t: string) => t.replace(/\*\*(.+?)\*\*/g, "<b>$1</b>").replace(/\n/g, "<br>");
  const row = (r: Draft["rows"][number]) => ({ ...r, title: html(r.title), agreed: html(r.agreed), note: html(r.note) });
  state.notes.set(1, {
    greeting: "Dear chị Hạnh, anh Tuấn và team,",
    intro: "Em xin gửi lại meeting note đã trao đổi hôm nay ạ. Nếu có nội dung cần bổ sung, anh chị nhắn em ạ.",
    rows: DRAFT.rows.map(row),
    pending: DRAFT.pending.map((p) => ({ ...p, row: row(p.row) })),
  });
}

const busy = () =>
  !state.paused && state.jobs.some((j) => j.state.kind === "queued" || j.state.kind === "running");

function queue() {
  return { jobs: structuredClone(state.jobs), paused: state.paused, busy: busy() };
}

function broadcast() {
  emitLocal("queue", queue());
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const baseName = (p: string) => p.split("/").pop() ?? p;
const stem = (p: string) => baseName(p).replace(/\.[^.]+$/, "");

function job(id: number): Job {
  const found = state.jobs.find((j) => j.id === id);
  if (!found) throw "Không tìm thấy file này trong danh sách.";
  return found;
}

/** Moves the running job forward; when it finishes, the next queued one starts. */
function tick() {
  if (state.paused || !state.modelsReady) return;
  let running = state.jobs.find((j) => j.state.kind === "running");
  if (!running) {
    running = state.jobs.find((j) => j.state.kind === "queued");
    if (!running) return;
    running.state = { kind: "running", percent: 0, eta_s: null };
  } else if (running.state.kind === "running") {
    const percent = Math.min(100, running.state.percent + 2 + Math.floor(Math.random() * 3));
    if (percent >= 100) {
      const name = running.title ?? stem(running.source);
      running.state = { kind: "done", txt: `${OUTPUT_DIR}/${name}.txt`, srt: `${OUTPUT_DIR}/${name}.srt` };
    } else {
      running.state = { kind: "running", percent, eta_s: Math.round((100 - percent) * 9) };
    }
  }
  broadcast();
}
if (params.get("tick") !== "0") setInterval(tick, 2000);

function addJob(source: string, title: string | null) {
  state.jobs.push({ id: state.nextId++, source, title, state: { kind: "queued" } });
}

const inQueue = (path: string) =>
  state.jobs.some((j) => j.source === path && j.state.kind !== "failed" && j.state.kind !== "cancelled");

type Args = Record<string, any>;

const commands: Record<string, (a: Args) => unknown | Promise<unknown>> = {
  snapshot: () => ({
    ...queue(),
    modelsReady: state.modelsReady,
    outputDir: OUTPUT_DIR,
    audioExtensions: AUDIO_EXTENSIONS,
  }),
  download_models: async () => {
    for (let p = 0; p <= 100; p += 5) {
      emitLocal("model-progress", p);
      await sleep(150);
    }
    state.modelsReady = true;
    state.readyLocalModels.add(state.selectedLocalModel);
  },
  local_models: () => localModelsView(),
  download_local_model: async ({ modelId }: Args) => {
    for (let p = 0; p <= 100; p += 10) {
      emitLocal("local-model-progress", { modelId, percent: p });
      await sleep(100);
    }
    state.readyLocalModels.add(String(modelId));
    state.modelsReady = true;
  },
  select_local_model: ({ modelId }: Args) => {
    const id = String(modelId);
    if (!state.readyLocalModels.has(id)) {
      throw "Model này chưa được tải và xác minh SHA-256 trên máy.";
    }
    state.selectedLocalModel = id;
    return localModelsView();
  },
  delete_local_model: ({ modelId }: Args) => {
    const id = String(modelId);
    if (state.selectedLocalModel === id) {
      throw "Hãy chọn model khác trước khi xoá model đang dùng.";
    }
    state.readyLocalModels.delete(id);
    return localModelsView();
  },
  set_resource_profile: ({ profile }: Args) => {
    state.resourceProfile = String(profile);
    return localModelsView();
  },
  enqueue: ({ paths }: Args) => {
    const skipped: string[] = [];
    for (const p of paths as string[]) {
      const ext = p.split(".").pop()?.toLowerCase() ?? "";
      if (!AUDIO_EXTENSIONS.includes(ext)) skipped.push(`${baseName(p)}: định dạng không hỗ trợ`);
      else if (inQueue(p)) skipped.push(`${baseName(p)}: đã có trong danh sách`);
      else addJob(p, null);
    }
    broadcast();
    return skipped;
  },
  enqueue_voice_memos: ({ choices }: Args) => {
    const skipped: string[] = [];
    for (const { path, title } of choices as { path: string; title: string }[]) {
      if (inQueue(path)) skipped.push(`${title}: đã có trong danh sách`);
      else addJob(path, title);
    }
    broadcast();
    return skipped;
  },
  cancel: ({ id }: Args) => {
    job(id).state = { kind: "cancelled" };
    broadcast();
  },
  retry: ({ id }: Args) => {
    job(id).state = { kind: "queued" };
    broadcast();
  },
  rerun: ({ id }: Args) => {
    const src = job(id);
    if (inQueue(src.source)) throw "File này đang chờ hoặc đang chạy.";
    const newId = state.nextId++;
    state.jobs.push({ id: newId, source: src.source, title: src.title, state: { kind: "queued" } });
    broadcast();
    return newId;
  },
  remove: ({ id }: Args) => {
    state.jobs = state.jobs.filter((j) => j.id !== id);
    broadcast();
  },
  set_paused: ({ paused }: Args) => {
    state.paused = paused;
    broadcast();
  },
  read_transcript: ({ id }: Args) => {
    if (job(id).state.kind !== "done") throw "File này chưa chép lời xong.";
    return { text: TRANSCRIPT_TEXT, lines: TRANSCRIPT_LINES };
  },
  clean_transcript: ({ id }: Args) => {
    const j = job(id);
    return `${OUTPUT_DIR}/${j.title ?? stem(j.source)} (đã làm sạch).txt`;
  },
  allow_job_audio: ({ id }: Args) => job(id).source,
  list_voice_memos: () => {
    switch (params.get("memos")) {
      case "denied":
        return { status: "noPermission" };
      case "failed":
        return { status: "failed", message: "Không mở được cơ sở dữ liệu của app Ghi âm (CloudRecordings.db bị khoá)." };
      case "empty":
        return { status: "ok", recordings: [] };
      default:
        return { status: "ok", recordings: RECORDINGS };
    }
  },
  open_full_disk_settings: () => console.info("[mock] open_full_disk_settings"),
  relaunch: () => location.reload(),

  ai_settings: () => ({ summaryStyle: "workEmail", ...structuredClone(state.ai) }),
  save_ai_settings: ({ settings }: Args) => {
    state.ai.defaultProvider = settings.defaultProvider;
    state.ai.models = { ...settings.models };
    if (settings.summaryStyle) (state.ai as Record<string, unknown>).summaryStyle = settings.summaryStyle;
  },
  list_models: ({ provider }: Args) => MODELS[provider as Provider] ?? [],
  validate_saved_model: ({ provider, model }: Args) => {
    const models = MODELS[provider as Provider] ?? [];
    const saved = String(model ?? state.ai.models[provider as Provider] ?? "");
    const ok = models.includes(saved);
    return { ok, models, suggestion: ok ? null : (models[0] ?? null) };
  },
  probe_accessible_models: ({ candidates, model }: Args) => {
    const checked = [...new Set<string>([model, ...candidates].filter(Boolean))].slice(0, 2);
    return { checked, accessible: checked.slice(0, 1) };
  },
  set_api_key: ({ provider, key }: Args) => {
    const k = String(key).trim();
    if (k.length < 8) throw "API key quá ngắn.";
    state.ai.keys[provider as Provider] = `${k.slice(0, 6)}…${k.slice(-4)}`;
  },
  delete_api_key: ({ provider }: Args) => {
    delete state.ai.keys[provider as Provider];
  },
  test_provider: async ({ provider }: Args) => {
    await sleep(600);
    if (!state.ai.keys[provider as Provider]) throw "Chưa có API key cho nhà cung cấp này.";
    return { ok: true, transient: false, message: "Kết nối thành công." };
  },
  get_glossary: () => {
    if (params.get("glossary") === "error") throw "Không đọc được danh sách thuật ngữ (mô phỏng lỗi).";
    return structuredClone(state.glossary);
  },
  save_glossary: ({ terms }: Args) => {
    state.glossary = structuredClone(terms);
  },
  import_glossary: ({ terms }: Args) => terms,
  export_glossary: () => undefined,

  summarize_job: async ({ id }: Args) => {
    await sleep(1050); // plus the common latency below, about 1.2 s in total
    if (job(id).state.kind !== "done") throw "File này chưa chép lời xong.";
    return structuredClone(DRAFT);
  },
  load_note: ({ id }: Args) => structuredClone(state.notes.get(id) ?? null),
  save_note: ({ id, draft }: Args) => {
    state.notes.set(id, structuredClone(draft));
    const j = job(id);
    return `${OUTPUT_DIR}/${j.title ?? stem(j.source)} - meeting note.html`;
  },
  generate_diagram: async ({ id }: Args) => {
    await sleep(900);
    if (job(id).state.kind !== "done") throw "File này chưa chép lời xong.";
    const providers: Provider[] = ["claude", "chatGpt", "gemini"];
    const usable = providers.filter((p) => Boolean(state.ai.keys[p]));
    if (usable.length === 0) throw "Chưa có API key cho nhà cung cấp nào";
    const chosen = usable.includes(state.ai.defaultProvider) ? state.ai.defaultProvider : usable[0];
    const providerLabel = chosen === "claude" ? "Claude" : chosen === "chatGpt" ? "ChatGPT" : "Gemini";
    return {
      title: "Quy trình duyệt thanh toán (mô phỏng)",
      summary: "Kiểm tra hoá đơn, phân nhánh theo ngưỡng 2 triệu và đường trả lại hồ sơ thiếu chứng từ.",
      provider: providerLabel,
      model: state.ai.models[chosen] ?? "gemini-3.8-flash",
      nodes: [
        { id: "n1", kind: "start", label: "Tiếp nhận hồ sơ đề nghị thanh toán", role: "Nhân viên" },
        { id: "n2", kind: "decision", label: "Hồ sơ đủ hoá đơn hợp lệ?", role: "Kế toán" },
        { id: "n3", kind: "step", label: "Trả lại hồ sơ để bổ sung hoá đơn", role: "Kế toán" },
        { id: "n4", kind: "decision", label: "Giá trị khoản chi >= 2 triệu?", role: "" },
        { id: "n5", kind: "step", label: "Trình duyệt bổ sung", role: "" },
        { id: "n6", kind: "end", label: "Chuyển kế toán thực hiện thanh toán", role: "Kế toán" },
      ],
      edges: [
        { from: "n1", to: "n2", label: "" },
        { from: "n2", to: "n3", label: "Thiếu hoá đơn" },
        { from: "n3", to: "n1", label: "Nộp lại" },
        { from: "n2", to: "n4", label: "Đủ hoá đơn" },
        { from: "n4", to: "n6", label: "< 2 triệu" },
        { from: "n4", to: "n5", label: ">= 2 triệu" },
        { from: "n5", to: "n6", label: "Đã duyệt" },
      ],
      uncertainties: [
        { item: "Người duyệt bổ sung", reason: "Bản chép lời chưa chốt rõ ai duyệt khoản từ 2 triệu trở lên." },
      ],
    };
  },
  load_diagram: ({ id }: Args) => structuredClone(state.diagrams.get(id) ?? null),
  save_diagram: ({ id, diagram }: Args) => {
    state.diagrams.set(id, structuredClone(diagram));
    const j = job(id);
    return `${OUTPUT_DIR}/${j.title ?? stem(j.source)}_diagram.json`;
  },
  export_diagram_file: ({ path }: Args) => String(path),
  meeting_templates: () => ({ excel: null, docx: null }),
  import_meeting_template: ({ path }: Args) => {
    const name = baseName(String(path));
    return name.toLowerCase().endsWith(".docx")
      ? { excel: null, docx: name }
      : { excel: name, docx: null };
  },
  reset_meeting_template: () => ({ excel: null, docx: null }),
  excel_template: () => null,
  import_excel_template: ({ path }: Args) => baseName(String(path)),
  reset_excel_template: () => undefined,
  export_note_excel: ({ path }: Args) => String(path),
  export_note_docx: ({ path }: Args) => String(path),
};

const openTarget = params.get("open");
if (openTarget) window.addEventListener("load", () => autopilot(openTarget));

export async function invoke<T>(cmd: string, args: Args = {}): Promise<T> {
  await sleep(150);
  const run = commands[cmd];
  if (!run) {
    console.warn(`[mock invoke] unknown command ${cmd}`, args);
    throw `Lệnh ${cmd} chưa có trong bản mô phỏng.`;
  }
  try {
    const result = (await run(args)) as T;
    console.debug(`[mock invoke] ${cmd}`, args, result);
    return result;
  } catch (e) {
    console.debug(`[mock invoke] ${cmd} failed`, args, e);
    throw e;
  }
}

export function convertFileSrc(_path: string, _protocol = "asset"): string {
  return "";
}

export function isTauri(): boolean {
  return false;
}
