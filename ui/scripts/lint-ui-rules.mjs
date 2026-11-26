#!/usr/bin/env node
/**
 * Level-5 Build Gate for `design-decisions` UI rules (`UI-01`, `UI-02`, `UI-03`).
 * Runs before `tsc --noEmit && vite build` in `ui/package.json`.
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const uiDir = path.resolve(here, "..");
const repoRoot = path.resolve(uiDir, "..");

const errors = [];

// UI-01: No hardcoded hex or rgb() color literals outside :root in ui/src/style.css.
const cssPath = path.join(uiDir, "src", "style.css");
const cssLines = fs.readFileSync(cssPath, "utf8").split("\n");
let inRoot = false;
let tokenCount = 0;
for (let i = 0; i < cssLines.length; i++) {
  const line = cssLines[i];
  if (line.trim().startsWith(":root")) {
    inRoot = true;
    continue;
  }
  if (inRoot) {
    if (/^\s*--[a-z0-9-]+\s*:/.test(line)) tokenCount++;
    if (line.trim() === "}") inRoot = false;
    continue;
  }
  const stripped = line.replace(/\/\*.*?\*\//g, "");
  if (/#[0-9a-fA-F]{3,8}\b/.test(stripped) || /\brgb\(/.test(stripped)) {
    errors.push(`UI-01 ${path.relative(repoRoot, cssPath)}:${i + 1} hardcoded color outside :root -> ${line.trim()}`);
  }
}

// UI-02: No inline `style:` strings in ui/src/*.ts.
const srcDir = path.join(uiDir, "src");
for (const entry of fs.readdirSync(srcDir)) {
  if (!entry.endsWith(".ts")) continue;
  const filePath = path.join(srcDir, entry);
  const lines = fs.readFileSync(filePath, "utf8").split("\n");
  for (let i = 0; i < lines.length; i++) {
    if (/\bstyle\s*:/.test(lines[i])) {
      errors.push(`UI-02 ${path.relative(repoRoot, filePath)}:${i + 1} inline style attribute -> ${lines[i].trim()}`);
    }
  }
}

// UI-03: Every rule row in AI/rules/DESIGN-DECISIONS.md must carry date, owner, and status.
const rulesPath = path.join(repoRoot, "AI", "rules", "DESIGN-DECISIONS.md");
let ruleCount = 0;
let level5Count = 0;
if (!fs.existsSync(rulesPath)) {
  errors.push("UI-03 missing AI/rules/DESIGN-DECISIONS.md");
} else {
  const ruleLines = fs.readFileSync(rulesPath, "utf8").split("\n");
  for (let i = 0; i < ruleLines.length; i++) {
    const m = ruleLines[i].match(/^\|\s*`(UI-\d+)`\s*\|(.*)\|$/);
    if (!m) continue;
    ruleCount++;
    const cols = ruleLines[i]
      .split("|")
      .slice(1, -1)
      .map((c) => c.trim());
    const [code, _desc, date, owner, _scope, status, level] = cols;
    if (!/^\d{4}-\d{2}-\d{2}$/.test(date ?? "")) {
      errors.push(`UI-03 ${code} missing valid YYYY-MM-DD date`);
    }
    if (!owner) {
      errors.push(`UI-03 ${code} missing owner`);
    }
    if (!/^(Còn hiệu lực|Đã thay)/.test(status ?? "")) {
      errors.push(`UI-03 ${code} missing valid status`);
    }
    if ((level ?? "").includes("Bậc 5")) level5Count++;
  }
}

if (errors.length > 0) {
  console.error("UI design-decisions gate FAILED:");
  for (const e of errors) console.error(`  - ${e}`);
  process.exit(1);
}

console.log(
  `UI design-decisions gate OK: ${tokenCount} tokens, ${ruleCount} rules (${level5Count} at Level 5).`,
);
