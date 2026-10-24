/**
 * Two native addons in one process, each with its own allocator: here two
 * copies of this one (a second path is a second dlopen), as an app has when
 * it loads pestle beside @wikidata-rs/node. mimalloc on macOS keeps its heaps
 * in fixed slots of the process, so two copies of it crashed (exit 139).
 */
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { expect, it } from "vitest";

const ROOT = join(__dirname, "..");
const binary = readdirSync(ROOT).find((f) => /^pestle-node\..+\.node$/.test(f));

it.skipIf(!binary)("two copies of the addon live in one process", () => {
  const dir = mkdtempSync(join(tmpdir(), "pestle-coexist-"));
  try {
    const copy = join(dir, "copy.node");
    copyFileSync(join(ROOT, binary!), copy);
    const script = `
      const a = require(process.argv[1]), b = require(process.argv[2]);
      const html = "<ul>" + "<li><a href=/x>hi</a></li>".repeat(20000) + "</ul>";
      Promise.all([a, b, a, b].map((m) => new m.NativeExtractor({ l: ["li a@href"] }, {}).extractAsync(html, null, null)))
        .then((outs) => {
          for (const m of [a, b]) new m.NativeExtractor({ l: ["li a@href"] }, {}).extract(html, null, null);
          console.log(outs.map((o) => o.data.l.length).join(","));
        });
    `;
    const run = spawnSync(process.execPath, ["-e", script, join(ROOT, binary!), copy], { encoding: "utf8" });
    expect({ status: run.status, signal: run.signal, stderr: run.stderr }).toEqual({ status: 0, signal: null, stderr: "" });
    expect(run.stdout.trim()).toBe("20000,20000,20000,20000");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
