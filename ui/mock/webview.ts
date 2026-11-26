// Mock of @tauri-apps/api/webview: drag and drop never fires in the browser.
export function getCurrentWebview() {
  return {
    label: "main",
    onDragDropEvent: async (_handler: unknown): Promise<() => void> => () => {},
  };
}
