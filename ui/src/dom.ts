/** Builds an element. All text goes through textContent, so file names can't inject markup. */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string | null)[]
): HTMLElementTagNameMap[K] {
  const { title, ...rest } = props;
  const node = Object.assign(document.createElement(tag), rest);
  if (typeof title === "string" && title.trim()) {
    if (tag === "button" || tag === "summary") {
      node.dataset.tip = title.trim();
      if (!node.getAttribute("aria-label") && !node.textContent?.trim()) {
        node.setAttribute("aria-label", title.trim());
      }
    } else {
      node.title = title;
    }
  }
  node.append(...children.filter((c) => c !== null));
  return node;
}
