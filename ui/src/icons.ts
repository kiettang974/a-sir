//! Closed SVG icon registry (`design-decisions` action -> icon mapping).
//! Uniform 16x16 viewBox, 1.65px stroke, rounded caps/joins, `currentColor`.

export type IconName =
  | "plus"
  | "mic"
  | "sliders"
  | "pause"
  | "play"
  | "playSm"
  | "trash"
  | "more"
  | "copy"
  | "fileText"
  | "table"
  | "summary"
  | "sync"
  | "diagram"
  | "folder"
  | "folderPlus"
  | "chevronDown"
  | "chevronRight"
  | "search"
  | "close"
  | "expand"
  | "help"
  | "mac"
  | "windows"
  | "linux"
  | "mobile";

const PATHS: Record<IconName, string> = {
  plus: '<path d="M8 3.2v9.6M3.2 8h9.6"/>',
  mic: '<rect x="5.7" y="2.3" width="4.6" height="7.4" rx="2.3"/><path d="M3.6 7.8a4.4 4.4 0 0 0 8.8 0M8 12.2v1.8M5.8 14h4.4"/>',
  sliders: '<path d="M2.5 4.5h11M2.5 11.5h11"/><circle cx="5.5" cy="4.5" r="1.6" fill="var(--paper)"/><circle cx="10.5" cy="11.5" r="1.6" fill="var(--paper)"/>',
  pause: '<path d="M5.5 3.5v9M10.5 3.5v9"/>',
  play: '<path d="M5 3.4l7.4 4.6L5 12.6Z" fill="currentColor" stroke="none"/>',
  playSm: '<path d="M5.5 4.2l6.2 3.8-6.2 3.8Z" fill="currentColor" stroke="none"/>',
  trash: '<path d="M3.2 4.5h9.6M6 4.5V3.1c0-.4.3-.7.7-.7h2.6c.4 0 .7.3.7.7v1.4M4.6 4.5l.6 8.3c0 .5.4.9.9.9h3.8c.5 0 .9-.4.9-.9l.6-8.3"/>',
  more: '<circle cx="3.2" cy="8" r="1.25" fill="currentColor" stroke="none"/><circle cx="8" cy="8" r="1.25" fill="currentColor" stroke="none"/><circle cx="12.8" cy="8" r="1.25" fill="currentColor" stroke="none"/>',
  copy: '<rect x="5.6" y="5.6" width="7.6" height="7.6" rx="1.4"/><path d="M3.6 10.4H3.2a1.2 1.2 0 0 1-1.2-1.2V3.2A1.2 1.2 0 0 1 3.2 2h6a1.2 1.2 0 0 1 1.2 1.2v.4"/>',
  fileText: '<path d="M4 2.2h5.2L12.5 5.5v8.3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V3.2a1 1 0 0 1 1-1Z"/><path d="M9 2.4v3.3h3.3M5.5 8.4h5M5.5 11.2h3.4"/>',
  table: '<rect x="2.4" y="3" width="11.2" height="10" rx="1.4"/><path d="M2.4 6.5h11.2M6.4 6.5V13"/>',
  summary: '<path d="M3.2 4h9.6M3.2 8h9.6M3.2 12h6"/>',
  sync: '<path d="M13.2 8a5.2 5.2 0 0 1-9.1 3.4M2.8 8a5.2 5.2 0 0 1 9.1-3.4"/><path d="M12.2 2.4v2.6H9.6M3.8 13.6V11h2.6"/>',
  diagram: '<rect x="5.5" y="1.8" width="5" height="3.2" rx="1"/><rect x="1.8" y="11" width="4.8" height="3.2" rx="1"/><rect x="9.4" y="11" width="4.8" height="3.2" rx="1"/><path d="M8 5v3M4.2 11V8h7.6v3"/>',
  folder: '<path d="M2.2 4.4a1.2 1.2 0 0 1 1.2-1.2h3l1.4 1.5h4.8a1.2 1.2 0 0 1 1.2 1.2v5.7a1.2 1.2 0 0 1-1.2 1.2H3.4a1.2 1.2 0 0 1-1.2-1.2Z"/>',
  folderPlus: '<path d="M2.2 4.4a1.2 1.2 0 0 1 1.2-1.2h3l1.4 1.5h4.8a1.2 1.2 0 0 1 1.2 1.2v5.7a1.2 1.2 0 0 1-1.2 1.2H3.4a1.2 1.2 0 0 1-1.2-1.2Z"/><path d="M8 7v3.6M6.2 8.8h3.6"/>',
  chevronDown: '<path d="M4.2 6.2 8 10l3.8-3.8"/>',
  chevronRight: '<path d="M6.2 4.2 10 8l-3.8 3.8"/>',
  search: '<circle cx="7.2" cy="7.2" r="4.4"/><path d="m10.5 10.5 3.1 3.1"/>',
  close: '<path d="M4.2 4.2 11.8 11.8M11.8 4.2 4.2 11.8"/>',
  expand: '<path d="M9.8 2.6h3.6v3.6M13.4 2.6 9.4 6.6M6.2 13.4H2.6V9.8M2.6 13.4l4-4"/>',
  help: '<circle cx="8" cy="8" r="6"/><path d="M6.2 6.3a1.9 1.9 0 0 1 3.6.7c0 1.2-1.8 1.4-1.8 2.6"/><circle cx="8" cy="11.8" r="0.6" fill="currentColor" stroke="none"/>',
  mac: '<path fill="currentColor" stroke="none" d="M11.6 8.4c0-1.5 1.2-2.2 1.3-2.3-.7-1-1.8-1.2-2.2-1.2-.9-.1-1.8.6-2.3.6-.5 0-1.2-.5-2-.5-1 0-2 .6-2.5 1.5-1.1 1.9-.3 4.6.8 6.1.5.7 1.1 1.6 1.9 1.5.8 0 1.1-.5 2-.5.9 0 1.2.5 2 .5.8 0 1.4-.7 1.9-1.5.6-.9.8-1.7.9-1.8 0 0-1.8-.7-1.8-2.4ZM10.1 3.7c.4-.5.7-1.2.6-2-.6 0-1.4.4-1.8 1-.4.5-.7 1.2-.6 1.9.7.1 1.4-.3 1.8-.9Z"/>',
  windows: '<path fill="currentColor" stroke="none" d="M2 3.6 7.5 2.8v4.8H2V3.6Zm6.3-.9L14 1.8v5.8H8.3V2.7ZM2 8.4h5.5v4.8L2 12.4V8.4Zm6.3 0H14v5.8l-5.7-.9V8.4Z"/>',
  linux: '<path fill="currentColor" fill-rule="evenodd" stroke="none" d="M8 1.2c1.9 0 2.6 1.3 2.6 3.3 0 1 .4 1.8 1.2 3 .7 1.1 1.2 2.6.8 4 1 .5 1.5 1.4 1.1 2.1-.5.8-2.1 1.2-3.5 1-.9-.2-1.3-.7-1.5-1.2-.5.1-.9.1-1.4 0-.2.5-.6 1-1.5 1.2-1.4.2-3-.2-3.5-1-.4-.7.1-1.6 1.1-2.1-.4-1.4.1-2.9.8-4 .8-1.2 1.2-2 1.2-3 0-2 .7-3.3 2.6-3.3Zm0 5.3c-1.5 0-2.7 1.7-2.7 3.7 0 1.6 1.1 2.4 2.7 2.4s2.7-.8 2.7-2.4c0-2-1.2-3.7-2.7-3.7ZM4.2 12.2c-.6.1-1.1.3-1.1.8 0 .5.8.9 1.9.9.9 0 1.4-.5 1.2-1-.3-.6-1.2-.9-2-.7Zm7.6 0c-.8-.2-1.7.1-2 .7-.2.5.3 1 1.2 1 1.1 0 1.9-.4 1.9-.9 0-.5-.5-.7-1.1-.8ZM6.9 3.1a.65.8 0 1 0 0 1.6.65.8 0 1 0 0-1.6Zm2.2 0a.65.8 0 1 0 0 1.6.65.8 0 1 0 0-1.6ZM6.6 5.2c.6-.5 2.2-.5 2.8 0-.3.6-.9.9-1.4.9s-1.1-.3-1.4-.9Z"/>',
  mobile: '<rect x="4.5" y="1.8" width="7" height="12.4" rx="1.4" stroke-width="1.45"/><path d="M7.1 11.9h1.8" stroke-width="1.4"/>',
};

export function icon(name: IconName, size = 15): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.6");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  svg.classList.add("ico");
  svg.innerHTML = PATHS[name];
  return svg;
}
