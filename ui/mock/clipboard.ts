// Mock of @tauri-apps/plugin-clipboard-manager.
export async function writeText(text: string): Promise<void> {
  console.info("[mock clipboard] writeText", text.length, "chars");
  await navigator.clipboard?.writeText(text).catch(() => {});
}
export async function writeHtml(html: string, altText?: string): Promise<void> {
  console.info("[mock clipboard] writeHtml", html.length, "chars", altText?.slice(0, 80));
}
export async function readText(): Promise<string> {
  return "";
}
