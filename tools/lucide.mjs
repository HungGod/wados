// Writes the lucide icons the shell and the apps draw (assets/lucide/<name>.svg,
// stroke in currentColor), from a pinned lucide-static (tools/package.json):
//   (cd tools && npm install) && node tools/lucide.mjs [name ...]
// Takes the icons already listed in assets/lucide/NAMES plus the names given
// (lucide's file names: "circle-check"). wad-gtk compiles them in.
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const lucide = join(root, "tools/node_modules/lucide-static");
const out = join(root, "assets/lucide");
const listed = join(out, "NAMES");

const names = new Set(process.argv.slice(2));
if (existsSync(listed)) {
  for (const line of readFileSync(listed, "utf8").split("\n")) if (line && !line.startsWith("#")) names.add(line.trim());
}

mkdirSync(out, { recursive: true });
const written = [];
for (const name of [...names].sort()) {
  const src = join(lucide, "icons", `${name}.svg`);
  if (!existsSync(src)) throw new Error(`lucide-static has no ${name}`);
  // Keep the drawing; drop lucide's comment and class, so every icon reads the same.
  const svg = readFileSync(src, "utf8")
    .replace(/<!--[\s\S]*?-->\s*/g, "")
    .replace(/\s+class="[^"]*"/, "")
    .replace(/\s*\n\s*/g, " ")
    .replace(/>\s+</g, "><")
    .trim();
  writeFileSync(join(out, `${name}.svg`), svg + "\n");
  written.push(name);
}
const version = JSON.parse(readFileSync(join(lucide, "package.json"), "utf8")).version;
writeFileSync(listed, `# lucide-static ${version} (ISC license): the icons wad-gtk compiles in\n${written.join("\n")}\n`);
console.log(`wrote ${written.length} icons to assets/lucide`);
