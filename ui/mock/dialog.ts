// Mock of @tauri-apps/plugin-dialog.
export async function open(options?: unknown): Promise<null> {
  console.info("[mock dialog] open", options);
  return null;
}
export async function save(options?: unknown): Promise<null> {
  console.info("[mock dialog] save", options);
  return null;
}
export async function ask(text: string, options?: unknown): Promise<boolean> {
  console.info("[mock dialog] ask", text, options);
  return true;
}
export async function confirm(text: string, options?: unknown): Promise<boolean> {
  console.info("[mock dialog] confirm", text, options);
  return true;
}
export async function message(text: string, options?: unknown): Promise<void> {
  console.info("[mock dialog] message", text, options);
}
