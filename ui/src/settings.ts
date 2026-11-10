import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

import { el } from "./dom";

export type Provider = "claude" | "chatGpt" | "gemini";
export const PROVIDERS: Provider[] = ["claude", "chatGpt", "gemini"];
export const PROVIDER_NAME: Record<Provider, string> = {
  claude: "Claude",
  chatGpt: "ChatGPT",
  gemini: "Gemini",
};

export type AiView = {
  defaultProvider: Provider;
  models: Partial<Record<Provider, string>>;
  keys: Partial<Record<Provider, string>>;
};
type Term = { correct: string; heardAs: string; note: string };

/** Fills itself in and keeps its own state, so the rest of the app can leave it alone. */
export function settingsPane(): HTMLElement {
  const body = el("div", { className: "settings" });
  const load = async () => {
    try {
      const [ai, terms] = await Promise.all([
        invoke<AiView>("ai_settings"),
        invoke<Term[]>("get_glossary"),
      ]);
      body.replaceChildren(aiSection(ai, load), glossarySection(terms));
    } catch (e) {
      body.replaceChildren(el("p", { className: "problem problem-text", textContent: String(e) }));
    }
  };
  load();
  return el(
    "section",
    { className: "reader" },
    el("header", { className: "reader-head" }, el("h2", { textContent: "Cấu hình" })),
    body,
  );
}

function aiSection(ai: AiView, reload: () => void): HTMLElement {
  const persist = () =>
    invoke("save_ai_settings", {
      settings: { defaultProvider: ai.defaultProvider, models: ai.models },
    });
  return el(
    "section",
    {},
    el("h3", { textContent: "Nhà cung cấp AI" }),
    el("p", {
      className: "hint",
      textContent:
        "Bảng tóm tắt được soạn bởi nhà cung cấp bạn chọn. Nội dung cuộc họp được gửi đến họ khi bạn bấm tạo bảng, và phí tính theo lượng dùng của API key, tách khỏi gói đăng ký. API key lưu trong thư mục dữ liệu của app trên máy này.",
    }),
    ...PROVIDERS.map((p) => providerBlock(p, ai, persist, reload)),
  );
}

function providerBlock(
  provider: Provider,
  ai: AiView,
  persist: () => Promise<unknown>,
  reload: () => void,
): HTMLElement {
  const name = PROVIDER_NAME[provider];
  const hint = ai.keys[provider];
  const status = el("p", { className: "provider-status" });
  const show = (text: string, bad = false) => {
    status.textContent = text;
    status.className = `provider-status${bad ? " bad" : ""}`;
  };
  const key = el("input", {
    className: "field",
    type: "password",
    placeholder: hint ? `Đang dùng ${hint}. Nhập để thay key` : "Dán API key",
    autocomplete: "off",
    ariaLabel: `API key của ${name}`,
  });
  let modelName = ai.models[provider] ?? "";
  const saveModel = async (value: string) => {
    modelName = value.trim();
    ai.models[provider] = modelName;
    await persist();
    show("Đã lưu model.");
  };
  // Without a key the list can't be fetched, so the name is typed; with one it is picked.
  const modelSlot = el("div");
  const typed = () =>
    el("input", {
      className: "field",
      value: modelName,
      ariaLabel: `Tên model của ${name}`,
      onchange: (e: Event) => saveModel((e.target as HTMLInputElement).value),
    });
  const loadModels = async () => {
    if (!hint) {
      modelSlot.replaceChildren(typed());
      return;
    }
    const waiting = el("select", { className: "field", disabled: true }, el("option", { textContent: "Đang tải danh sách model…" }));
    modelSlot.replaceChildren(waiting);
    try {
      const models = await invoke<string[]>("list_models", { provider });
      const options = models.includes(modelName) || !modelName ? models : [modelName, ...models];
      const select = el(
        "select",
        {
          className: "field",
          ariaLabel: `Model của ${name}`,
          onchange: () => saveModel(select.value),
        },
        ...options.map((m) =>
          el("option", {
            value: m,
            selected: m === modelName,
            textContent: models.includes(m) ? m : `${m} (key này không dùng được)`,
          }),
        ),
      );
      modelSlot.replaceChildren(select);
      if (!modelName && models[0]) saveModel(models[0]);
    } catch (e) {
      modelSlot.replaceChildren(typed());
      show(`Không tải được danh sách model: ${e}`, true);
    }
  };
  loadModels();
  return el(
    "div",
    { className: "provider" },
    el(
      "div",
      { className: "provider-head" },
      el("span", { textContent: name }),
      el(
        "label",
        {},
        el("input", {
          type: "radio",
          name: "default-provider",
          checked: ai.defaultProvider === provider,
          onchange: async () => {
            ai.defaultProvider = provider;
            await persist();
          },
        }),
        "Dùng mặc định",
      ),
    ),
    el(
      "div",
      { className: "field-grid" },
      el("span", { textContent: "API key" }),
      key,
      el("button", {
        className: "link",
        textContent: "Lưu key",
        onclick: async () => {
          if (!key.value.trim()) return show("Hãy dán API key trước.", true);
          try {
            await invoke("set_api_key", { provider, key: key.value });
            reload();
          } catch (e) {
            show(String(e), true);
          }
        },
      }),
      hint
        ? el("button", {
            className: "link",
            textContent: "Xoá key",
            onclick: async () => {
              await invoke("delete_api_key", { provider });
              reload();
            },
          })
        : el("span"),
      el("span", { textContent: "Model" }),
      modelSlot,
      el("button", {
        className: "link",
        textContent: "Thử kết nối",
        onclick: async () => {
          show("Đang thử kết nối…");
          try {
            await invoke("test_provider", { provider, model: modelName });
            show(`Kết nối được với ${name} bằng model này.`);
          } catch (e) {
            show(String(e), true);
          }
        },
      }),
      el("span"),
    ),
    status,
  );
}

function glossarySection(initial: Term[]): HTMLElement {
  let terms = initial;
  const rows = el("tbody");
  const status = el("p", { className: "provider-status", style: "margin-left:0" } as never);
  const persist = async () => {
    try {
      await invoke("save_glossary", { terms });
    } catch (e) {
      status.textContent = String(e);
    }
  };
  const input = (term: Term, field: keyof Term, label: string) =>
    el("input", {
      className: "field",
      value: term[field],
      ariaLabel: label,
      onchange: (e: Event) => {
        term[field] = (e.target as HTMLInputElement).value;
        persist();
      },
    });
  const draw = () => {
    rows.replaceChildren(
      ...terms.map((term) =>
        el(
          "tr",
          {},
          el("td", {}, input(term, "correct", "Viết đúng")),
          el("td", {}, input(term, "heardAs", "Hay bị nghe thành")),
          el("td", {}, input(term, "note", "Giải thích")),
          el(
            "td",
            {},
            el("button", {
              className: "link",
              textContent: "Xoá",
              onclick: () => {
                terms = terms.filter((t) => t !== term);
                persist();
                draw();
              },
            }),
          ),
        ),
      ),
    );
  };
  draw();
  return el(
    "section",
    {},
    el("h3", { textContent: "Thuật ngữ" }),
    el("p", {
      className: "hint",
      textContent:
        "AI chỉ sửa từ nghe nhầm theo danh sách này khi tạo bảng tóm tắt. Bản .txt và .srt vẫn giữ nguyên lời gốc.",
    }),
    el(
      "table",
      { className: "terms" },
      el(
        "thead",
        {},
        el(
          "tr",
          {},
          el("th", { textContent: "Viết đúng" }),
          el("th", { textContent: "Hay bị nghe thành" }),
          el("th", { textContent: "Giải thích" }),
          el("th"),
        ),
      ),
      rows,
    ),
    el(
      "div",
      { className: "actions wrap" },
      el("button", {
        className: "add",
        textContent: "Thêm từ",
        onclick: () => {
          terms = [...terms, { correct: "", heardAs: "", note: "" }];
          draw();
          rows.querySelector<HTMLInputElement>("tr:last-child input")?.focus();
        },
      }),
      el("button", {
        className: "link",
        textContent: "Nhập từ CSV",
        onclick: async () => {
          const path = await open({ filters: [{ name: "CSV", extensions: ["csv"] }] });
          if (!path) return;
          try {
            terms = await invoke<Term[]>("import_glossary", { terms, path });
            draw();
            status.textContent = `Danh sách hiện có ${terms.length} từ.`;
          } catch (e) {
            status.textContent = String(e);
          }
        },
      }),
      el("button", {
        className: "link",
        textContent: "Xuất ra CSV",
        onclick: async () => {
          const path = await save({ defaultPath: "thuat-ngu.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
          if (!path) return;
          try {
            await invoke("export_glossary", { terms, path });
            status.textContent = "Đã xuất danh sách.";
          } catch (e) {
            status.textContent = String(e);
          }
        },
      }),
    ),
    status,
  );
}
