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
    title: "1. Thêm file ghi âm",
    body: "Thả file vào cửa sổ, bấm Thêm file hoặc lấy từ app Ghi âm.",
  },
  {
    target: '[data-tour="queue-list"]',
    title: "2. Danh sách bản ghi",
    body: "Chuyển văn bản trực tiếp tại máy, hỗ trợ gom nhóm và tìm theo tên.",
  },
  {
    target: '[data-tour="settings-btn"]',
    title: "3. Model & API key AI",
    body: "Đổi model chuyển văn bản, nhập API key (Gemini/ChatGPT/Claude) và quản lý từ vựng riêng.",
  },
  {
    target: '[data-tour="workspace"]',
    title: "4. Nghe lại & sửa trực tiếp",
    body: "Bấm mốc giờ để tua nghe đúng đoạn, bấm thẳng vào chữ để sửa và tự lưu.",
  },
  {
    target: '[data-tour="workspace"]',
    title: "5. Làm sạch, Tóm tắt & Sơ đồ",
    body: "Đi theo 4 tab: Văn bản → Làm sạch → Tóm tắt (Word/Excel/Email) → Sơ đồ.",
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
