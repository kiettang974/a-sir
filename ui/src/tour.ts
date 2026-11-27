//! First-run 5-step Spotlight Tour so new users understand A-Sir's workflow in 30 seconds.

import { el } from "./dom";

const TOUR_STORAGE_KEY = "asir-onboarding-tour-v1";

type TourStep = {
  target: string;
  title: string;
  body: string;
};

const STEPS: TourStep[] = [
  {
    target: '[data-tour="add-files"]',
    title: "1. Thêm file ghi âm hoặc lấy từ Ghi âm",
    body: "Kéo thả file âm thanh/video vào cửa sổ, bấm Thêm file, hoặc lấy trực tiếp các bản thu có sẵn từ app Ghi âm trên Mac.",
  },
  {
    target: '[data-tour="queue-list"]',
    title: "2. Chép lời tiếng Việt ngay trên máy",
    body: "File được xếp vào danh sách này và chép lời hoàn toàn tại máy. Bạn có thể gom theo nhóm thư mục hoặc tìm nhanh theo tên.",
  },
  {
    target: '[data-tour="settings-btn"]',
    title: "3. Gắn tài khoản AI khi cần tóm tắt & vẽ sơ đồ",
    body: "Mở Cấu hình để nhập API key (Claude, ChatGPT hoặc Gemini). Chép lời không cần mạng; AI chỉ chạy khi bạn chủ động bấm Tóm tắt, Làm sạch hoặc Vẽ sơ đồ.",
  },
  {
    target: '[data-tour="workspace"]',
    title: "4. Đọc bản chép, nghe lại theo giờ & sửa trực tiếp",
    body: "Chọn một bản ghi đã xong để đọc: bấm vào mốc thời gian (00:54) để tua nghe lại đúng đoạn đó, hoặc bấm thẳng vào chữ để chỉnh sửa và tự lưu.",
  },
  {
    target: '[data-tour="workspace"]',
    title: "5. Ra Biên bản & Sơ đồ trên cùng 1 màn hình",
    body: "Chuyển qua lại 1 click giữa 1. Bản chép → 2. Làm sạch → 3. Tóm tắt (xuất Word/Excel/Email) → 4. Sơ đồ (16 mẫu hình, xuất PNG & draw.io).",
  },
];

let activeBackdrop: HTMLElement | null = null;

export function hasSeenTour(): boolean {
  try {
    return localStorage.getItem(TOUR_STORAGE_KEY) === "done";
  } catch {
    return true;
  }
}

function markTourSeen() {
  try {
    localStorage.setItem(TOUR_STORAGE_KEY, "done");
  } catch {
    // Ignore storage quota issues.
  }
}

export function startOnboardingTour() {
  if (activeBackdrop) {
    activeBackdrop.remove();
    activeBackdrop = null;
  }
  let idx = 0;

  const spotlight = el("div", { className: "tour-spotlight" });
  const stepBadge = el("span", { className: "tour-step-badge" });
  const heading = el("h3", { className: "tour-title" });
  const body = el("p", { className: "tour-body" });
  const prevBtn = el("button", { className: "btn-ghost", textContent: "Trước" });
  const nextBtn = el("button", { className: "btn-primary" });
  const skipBtn = el("button", { className: "link", textContent: "Bỏ qua" });

  const card = el(
    "div",
    { className: "tour-card", role: "dialog", ariaLabel: "Hướng dẫn nhanh A-Sir" },
    el("div", { className: "tour-card-top" }, stepBadge, skipBtn),
    heading,
    body,
    el("div", { className: "tour-actions" }, prevBtn, nextBtn),
  );

  const backdrop = el("div", { className: "tour-backdrop" }, spotlight, card);
  activeBackdrop = backdrop;

  const close = () => {
    markTourSeen();
    document.removeEventListener("keydown", onKey);
    window.removeEventListener("resize", position);
    backdrop.remove();
    if (activeBackdrop === backdrop) activeBackdrop = null;
  };

  const onKey = (ev: KeyboardEvent) => {
    if (ev.key === "Escape") close();
    else if (ev.key === "ArrowRight") go(idx + 1);
    else if (ev.key === "ArrowLeft" && idx > 0) go(idx - 1);
  };

  const position = () => {
    const step = STEPS[idx];
    const target = document.querySelector<HTMLElement>(step.target);
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const cardW = Math.min(360, vw - 32);

    if (target) {
      const rect = target.getBoundingClientRect();
      const pad = 6;
      const sx = Math.max(8, rect.left - pad);
      const sy = Math.max(8, rect.top - pad);
      const sw = Math.min(vw - sx - 8, rect.width + pad * 2);
      const sh = Math.min(vh - sy - 8, rect.height + pad * 2);
      spotlight.hidden = false;
      spotlight.style.left = `${sx}px`;
      spotlight.style.top = `${sy}px`;
      spotlight.style.width = `${sw}px`;
      spotlight.style.height = `${sh}px`;

      // Prefer placing card to the right of sidebar targets, or centered-top inside wide panes.
      let cx = rect.right + 16;
      let cy = rect.top;
      if (cx + cardW > vw - 16) {
        cx = Math.max(16, Math.min(vw - cardW - 16, rect.left + (rect.width - cardW) / 2));
        cy = rect.bottom + 14;
      }
      if (cy + 210 > vh - 16) {
        cy = Math.max(16, Math.min(vh - 220, rect.top + 28));
      }
      card.style.left = `${Math.round(cx)}px`;
      card.style.top = `${Math.round(cy)}px`;
    } else {
      spotlight.hidden = true;
      card.style.left = `${Math.round((vw - cardW) / 2)}px`;
      card.style.top = `${Math.round(vh * 0.28)}px`;
    }
  };

  const go = (nextIdx: number) => {
    if (nextIdx >= STEPS.length) {
      close();
      return;
    }
    idx = Math.max(0, nextIdx);
    const step = STEPS[idx];
    stepBadge.textContent = `Bước ${idx + 1} / ${STEPS.length}`;
    heading.textContent = step.title;
    body.textContent = step.body;
    prevBtn.hidden = idx === 0;
    nextBtn.textContent = idx === STEPS.length - 1 ? "Bắt đầu dùng" : "Tiếp";
    position();
  };

  prevBtn.onclick = () => go(idx - 1);
  nextBtn.onclick = () => go(idx + 1);
  skipBtn.onclick = close;
  backdrop.onclick = (e: MouseEvent) => {
    if (e.target === backdrop) close();
  };

  document.addEventListener("keydown", onKey);
  window.addEventListener("resize", position);
  document.body.append(backdrop);
  go(0);
}
