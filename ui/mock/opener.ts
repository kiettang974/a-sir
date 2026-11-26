// Mock of @tauri-apps/plugin-opener.
export async function revealItemInDir(path: string): Promise<void> {
  console.info("[mock opener] revealItemInDir", path);
}
export async function openPath(path: string): Promise<void> {
  console.info("[mock opener] openPath", path);
}
export async function openUrl(url: string): Promise<void> {
  console.info("[mock opener] openUrl", url);
  window.open(url, "_blank");
}
