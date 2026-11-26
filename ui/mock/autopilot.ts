// ?open=reader|note|settings|memos : clicks through the UI after load so a screenshot tool lands on that screen.
// It finds controls by class or visible text, so if the UI is redesigned and labels change, update LABELS below.
const LABELS = {
  jobOpen: ".job-open",
  note: "Bảng tóm tắt",
  settings: "Cấu hình",
  memos: "Từ app Ghi âm",
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function find(test: () => HTMLElement | null | undefined, timeout = 4000): Promise<HTMLElement | null> {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const found = test();
    if (found) return found;
    await sleep(50);
  }
  console.warn("[mock autopilot] control not found");
  return null;
}

const byText = (text: string) => () =>
  [...document.querySelectorAll<HTMLElement>("button, a")].find((b) => b.textContent?.trim() === text);

export async function autopilot(target: string) {
  if (target === "reader" || target === "note") {
    (await find(() => document.querySelector<HTMLElement>(LABELS.jobOpen)))?.click();
    if (target === "note") (await find(byText(LABELS.note)))?.click();
  } else if (target === "settings") {
    (await find(byText(LABELS.settings)))?.click();
  } else if (target === "memos") {
    (await find(byText(LABELS.memos)))?.click();
  }
}
