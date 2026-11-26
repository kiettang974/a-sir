// Invented sample data for the browser preview. None of this comes from a real recording.

export type JobState =
  | { kind: "queued" }
  | { kind: "running"; percent: number; eta_s: number | null }
  | { kind: "done"; txt: string; srt: string }
  | { kind: "failed"; message: string }
  | { kind: "cancelled" };
export type Job = { id: number; source: string; title: string | null; state: JobState };
export type Recording = {
  path: string;
  title: string;
  recordedAt: number | null;
  durationS: number | null;
  onDisk: boolean;
};
export type Provider = "claude" | "chatGpt" | "gemini";
export type AiView = {
  defaultProvider: Provider;
  models: Partial<Record<Provider, string>>;
  keys: Partial<Record<Provider, string>>;
};
export type Term = { correct: string; heardAs: string; note: string };
export type Evidence = { time: string; quote: string; seconds: number };
export type Row = { title: string; agreed: string; note: string; evidence: Evidence[]; uncertain: string[] };
export type Draft = { rows: Row[]; pending: { row: Row; reason: string }[] };
export type Line = { start: string; text: string };

const HOME = "/Users/demo";
const OUT = `${HOME}/Documents/ASR Studio`;

export const AUDIO_EXTENSIONS = ["m4a", "mp3", "wav", "aac", "flac", "ogg", "opus", "mp4", "mov", "webm"];
export const OUTPUT_DIR = OUT;

export function initialJobs(): Job[] {
  return [
    {
      id: 1,
      source: `${HOME}/Downloads/hop-ke-toan-t11.m4a`,
      title: "Họp kế toán tháng 11",
      state: { kind: "done", txt: `${OUT}/Họp kế toán tháng 11.txt`, srt: `${OUT}/Họp kế toán tháng 11.srt` },
    },
    {
      id: 2,
      source: `${HOME}/Downloads/phong-van-ung-vien-marketing.mp3`,
      title: "Phỏng vấn ứng viên marketing",
      state: { kind: "running", percent: 62, eta_s: 340 },
    },
    {
      id: 3,
      source: `${HOME}/Desktop/Workshop sản phẩm Q1.mov`,
      title: null,
      state: { kind: "queued" },
    },
    {
      id: 4,
      source: `${HOME}/Downloads/goi-dien-nha-cung-cap.ogg`,
      title: "Gọi điện nhà cung cấp",
      state: { kind: "failed", message: "Không đọc được file âm thanh: file bị hỏng hoặc không có kênh tiếng." },
    },
  ];
}

const T: [string, string][] = [
  ["00:00:02", "Rồi, mọi người vào đủ chưa nhỉ? Mình bắt đầu luôn nhé."],
  ["00:00:07", "Hôm nay mình chốt lại mấy việc kế toán tháng mười một, chủ yếu là hoá đơn với hạn nộp báo cáo."],
  ["00:00:16", "Dạ, bên em đã gom xong hoá đơn đầu vào đến hết ngày mười lăm rồi ạ."],
  ["00:00:21", "Tổng cộng khoảng một trăm bốn mươi hai hoá đơn, trong đó có mười một cái bị sai mã số thuế."],
  ["00:00:30", "Mười một cái là nhiều đấy. Sai bên mình nhập hay bên nhà cung cấp xuất sai?"],
  ["00:00:36", "Phần lớn là bên nhà cung cấp ạ. Em đã gửi email nhờ họ xuất lại, nhưng mới có bốn bên trả lời."],
  ["00:00:45", "Ừ, vậy em lập giúp anh một danh sách, ghi rõ bên nào chưa phản hồi, mình gọi điện trực tiếp."],
  ["00:00:53", "Dạ vâng."],
  ["00:00:55", "Chị Hạnh ơi, phần công nợ phải thu thì sao chị?"],
  ["00:00:59", "Công nợ thì ổn hơn tháng trước. Còn ba khách hàng quá hạn trên sáu mươi ngày, tổng khoảng bốn trăm hai mươi triệu."],
  ["00:01:10", "Trong đó công ty Minh Phát chiếm gần một nửa. Chị đã nhắc hai lần rồi mà họ cứ hẹn cuối tháng."],
  ["00:01:19", "Cái này anh nghĩ phải gửi công văn chính thức thôi, chứ nhắc miệng mãi không được."],
  ["00:01:25", "Em đồng ý. Để em soạn mẫu công văn, chiều mai gửi chị Hạnh xem trước."],
  ["00:01:32", "Ok. Giờ sang hạn nộp nhé. Tờ khai thuế giá trị gia tăng tháng này hạn là ngày hai mươi."],
  ["00:01:40", "Mà ngày hai mươi rơi vào thứ bảy, nên mình nộp trước thứ sáu cho chắc."],
  ["00:01:46", "Vậy chậm nhất thứ năm phải khoá sổ đầu vào, không nhận thêm hoá đơn nào nữa."],
  ["00:01:53", "Hoá đơn nào về sau thứ năm thì để kê khai bổ sung kỳ sau, đúng không anh?"],
  ["00:01:58", "Đúng rồi. Nhưng nhớ ghi chú lại để không bị sót."],
  ["00:02:03", "À còn một việc, phần mềm hoá đơn điện tử sắp hết hạn gói, cuối tháng phải gia hạn."],
  ["00:02:11", "Báo giá năm nay tăng khoảng mười lăm phần trăm. Em đang hỏi thêm hai nhà cung cấp khác để so sánh."],
  ["00:02:19", "Ừ, em so sánh xong thì gửi anh bảng tóm tắt, mình quyết trước ngày hai mươi lăm."],
  ["00:02:26", "Chi phí tiếp khách tháng này cũng hơi cao, có mấy khoản chưa có chứng từ đầy đủ."],
  ["00:02:33", "Khoản nào thiếu chứng từ thì trả lại cho phòng kinh doanh bổ sung, không thì không hạch toán được."],
  ["00:02:41", "Dạ. Em sẽ gửi danh sách cho anh Tuấn bên kinh doanh trong hôm nay."],
  ["00:02:47", "Được rồi, vậy tóm lại: danh sách hoá đơn sai, công văn Minh Phát, khoá sổ thứ năm, với báo giá phần mềm."],
  ["00:02:58", "Có ai cần bổ sung gì không? Không thì mình kết thúc ở đây nhé. Cảm ơn mọi người."],
];

export const TRANSCRIPT_LINES: Line[] = T.map(([start, text]) => ({ start, text }));
export const TRANSCRIPT_TEXT = TRANSCRIPT_LINES.map((l) => l.text).join("\n");

const day = (iso: string) => Math.floor(new Date(iso).getTime() / 1000);
const MEMOS = `${HOME}/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings`;

export const RECORDINGS: Recording[] = [
  { path: `${MEMOS}/20261109 091500.m4a`, title: "Họp giao ban sáng thứ hai", recordedAt: day("2026-11-09T09:15:00"), durationS: 1835, onDisk: true },
  { path: `${MEMOS}/20261107 143000.m4a`, title: "Trao đổi với khách hàng Minh Phát", recordedAt: day("2026-11-07T14:30:00"), durationS: 2710, onDisk: true },
  { path: `${MEMOS}/20261105 100000.m4a`, title: "Phỏng vấn ứng viên marketing", recordedAt: day("2026-11-05T10:00:00"), durationS: 3420, onDisk: true },
  { path: `${MEMOS}/20261103 160500.m4a`, title: "Ý tưởng chiến dịch Tết", recordedAt: day("2026-11-03T16:05:00"), durationS: 412, onDisk: false },
  { path: `${MEMOS}/20261030 083000.m4a`, title: "Đào tạo phần mềm kế toán mới", recordedAt: day("2026-10-30T08:30:00"), durationS: 5480, onDisk: true },
  { path: `${MEMOS}/20261025 190000.m4a`, title: "Ghi chú nhanh", recordedAt: day("2026-10-25T19:00:00"), durationS: 48, onDisk: true },
  { path: `${MEMOS}/20261018 110000.m4a`, title: "Họp tổng kết quý ba", recordedAt: day("2026-10-18T11:00:00"), durationS: 4150, onDisk: false },
  { path: `${MEMOS}/20261002 153000.m4a`, title: "Gọi điện đối tác vận chuyển", recordedAt: null, durationS: null, onDisk: true },
];

export function aiView(withKeys: boolean): AiView {
  return {
    defaultProvider: "claude",
    models: { claude: "claude-sonnet-4-5", chatGpt: "gpt-5", gemini: "gemini-2.5-pro" },
    keys: withKeys ? { claude: "sk-ant…7f2a", gemini: "AIza…c91e" } : {},
  };
}

export const MODELS: Record<Provider, string[]> = {
  claude: ["claude-opus-4-1", "claude-sonnet-4-5", "claude-haiku-4-5"],
  chatGpt: ["gpt-5", "gpt-5-mini", "gpt-4.1"],
  gemini: ["gemini-2.5-pro", "gemini-2.5-flash"],
};

export const GLOSSARY: Term[] = [
  { correct: "Minh Phát", heardAs: "minh phác, minh phát", note: "Khách hàng công nợ lớn" },
  { correct: "VAT", heardAs: "vê a tê, va", note: "Thuế giá trị gia tăng" },
  { correct: "hoá đơn điện tử", heardAs: "hoá đơn điện từ", note: "" },
  { correct: "MST", heardAs: "em ét tê", note: "Mã số thuế" },
  { correct: "chị Hạnh", heardAs: "chị hành", note: "Kế toán công nợ" },
];

export const DRAFT: Draft = {
  rows: [
    {
      title: "Hoá đơn đầu vào sai MST",
      agreed: "Có **11/142** hoá đơn sai mã số thuế, phần lớn do nhà cung cấp.\nLập danh sách bên chưa phản hồi và gọi điện trực tiếp.",
      note: "Mới 4 nhà cung cấp trả lời email.",
      evidence: [
        { time: "00:21", quote: "trong đó có mười một cái bị sai mã số thuế", seconds: 21 },
        { time: "00:45", quote: "ghi rõ bên nào chưa phản hồi, mình gọi điện trực tiếp", seconds: 45 },
      ],
      uncertain: [],
    },
    {
      title: "Công nợ quá hạn",
      agreed: "Ba khách hàng quá hạn trên 60 ngày, tổng khoảng **420 triệu**.\nGửi công văn chính thức cho Minh Phát.",
      note: "Mẫu công văn gửi chị Hạnh xem trước vào chiều mai.",
      evidence: [
        { time: "00:59", quote: "còn ba khách hàng quá hạn trên sáu mươi ngày", seconds: 59 },
        { time: "01:19", quote: "phải gửi công văn chính thức thôi", seconds: 79 },
      ],
      uncertain: ["Minh Phát", "bốn trăm hai mươi triệu"],
    },
    {
      title: "Tờ khai VAT tháng 11",
      agreed: "Khoá sổ đầu vào **chậm nhất thứ năm**, nộp tờ khai trước thứ sáu.\nHoá đơn về sau thứ năm kê khai bổ sung kỳ sau.",
      note: "",
      evidence: [
        { time: "01:40", quote: "ngày hai mươi rơi vào thứ bảy, nên mình nộp trước thứ sáu", seconds: 100 },
        { time: "01:46", quote: "chậm nhất thứ năm phải khoá sổ đầu vào", seconds: 106 },
      ],
      uncertain: [],
    },
    {
      title: "Gia hạn phần mềm hoá đơn điện tử",
      agreed: "So sánh báo giá với hai nhà cung cấp khác, quyết trước ngày **25**.",
      note: "Báo giá năm nay tăng khoảng 15%.",
      evidence: [
        { time: "02:11", quote: "Báo giá năm nay tăng khoảng mười lăm phần trăm", seconds: 131 },
      ],
      uncertain: [],
    },
  ],
  pending: [
    {
      row: {
        title: "Chứng từ chi phí tiếp khách",
        agreed: "Trả lại các khoản thiếu chứng từ cho phòng kinh doanh bổ sung.",
        note: "Gửi danh sách cho anh Tuấn trong hôm nay.",
        evidence: [{ time: "02:33", quote: "trả lại cho phòng kinh doanh bổ sung", seconds: 153 }],
        uncertain: ["anh Tuấn"],
      },
      reason: "Chưa rõ ai chịu trách nhiệm theo dõi việc bổ sung.",
    },
    {
      row: {
        title: "Kê khai bổ sung kỳ sau",
        agreed: "Ghi chú các hoá đơn về sau thứ năm để không bị sót.",
        note: "",
        evidence: [{ time: "01:58", quote: "nhớ ghi chú lại để không bị sót", seconds: 118 }],
        uncertain: [],
      },
      reason: "Có thể trùng với mục Tờ khai VAT tháng 11.",
    },
  ],
};
