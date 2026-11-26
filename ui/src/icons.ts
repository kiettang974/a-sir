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
  | "folder"
  | "folderPlus"
  | "chevronDown"
  | "chevronRight"
  | "mac"
  | "windows"
  | "linux";

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
  folder: '<path d="M2.2 4.4a1.2 1.2 0 0 1 1.2-1.2h3l1.4 1.5h4.8a1.2 1.2 0 0 1 1.2 1.2v5.7a1.2 1.2 0 0 1-1.2 1.2H3.4a1.2 1.2 0 0 1-1.2-1.2Z"/>',
  folderPlus: '<path d="M2.2 4.4a1.2 1.2 0 0 1 1.2-1.2h3l1.4 1.5h4.8a1.2 1.2 0 0 1 1.2 1.2v5.7a1.2 1.2 0 0 1-1.2 1.2H3.4a1.2 1.2 0 0 1-1.2-1.2Z"/><path d="M8 7v3.6M6.2 8.8h3.6"/>',
  chevronDown: '<path d="M4.2 6.2 8 10l3.8-3.8"/>',
  chevronRight: '<path d="M6.2 4.2 10 8l-3.8 3.8"/>',
  mac: '<rect x="2.6" y="3.2" width="10.8" height="7.2" rx="1.2"/><path d="M1.8 13h12.4M6.2 10.4v2.6M9.8 10.4v2.6"/>',
  windows: '<rect x="2.6" y="2.8" width="4.7" height="4.7" rx=".5"/><rect x="8.7" y="2.8" width="4.7" height="4.7" rx=".5"/><rect x="2.6" y="8.9" width="4.7" height="4.7" rx=".5"/><rect x="8.7" y="8.9" width="4.7" height="4.7" rx=".5"/>',
  linux: '<rect x="2.2" y="3" width="11.6" height="10" rx="1.6"/><path d="m5 6.6 2 1.6-2 1.6M8.6 10.2h2.4"/>',
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
