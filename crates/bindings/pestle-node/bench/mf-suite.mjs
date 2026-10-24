// The official microformats test suite: pestle's microformats2 JSON against each case's expected JSON
// (and against microformats-parser, for scale). node bench/mf-suite.mjs [filter]
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { extractStructured } = require("../dist/index.js");
const { mf2 } = require("microformats-parser");

const root = new URL("../../../../bench/.work/suites/mf-tests/tests/", import.meta.url).pathname;
const files = [];
const walk = (d) => { for (const f of readdirSync(d)) { const p = join(d, f); if (statSync(p).isDirectory()) walk(p); else if (f.endsWith(".html")) files.push(p); } };
walk(root);
const filter = process.argv[2];
const canon = (v) => JSON.stringify(v, (k, x) => (x && typeof x === "object" && !Array.isArray(x) ? Object.fromEntries(Object.keys(x).sort().map((k) => [k, x[k]])) : x));
// Timezone offsets compared as instants, not spellings: the suite's README asks for `+01:00`,
// some older files expect `+0100`.
const zones = (v) => JSON.parse(JSON.stringify(v).replace(/(\d\d:\d\d(?::\d\d)?)([+-]\d\d):?(\d\d)"/g, "$1$2$3\""));
let ours = 0, oursZones = 0, theirs = 0, total = 0; const fails = [];
for (const f of files) {
  if (filter && !f.includes(filter)) continue;
  const html = readFileSync(f, "utf8");
  let expected; try { expected = JSON.parse(readFileSync(f.replace(/\.html$/, ".json"), "utf8")); } catch { continue; }
  total++;
  const base = f.includes("v2-unit") ? "http://example.test" : "http://example.com/";
  const got = extractStructured(html, { url: base, raw: true, jsonLd: false, microdata: false, rdfa: false }).raw.microformats;
  const ref = mf2(html, { baseUrl: base });
  const strip = (x) => ({ items: x.items, rels: x.rels, "rel-urls": x["rel-urls"] });
  const ok = canon(strip(got)) === canon(strip(expected));
  if (ok) ours++; else fails.push(f.slice(root.length));
  if (ok || canon(zones(strip(got))) === canon(zones(strip(expected)))) oursZones++;
  if (canon(strip(ref)) === canon(strip(expected))) theirs++;
  if (!ok && process.argv[3] === "-v") { console.log("== " + f.slice(root.length)); console.log("want", canon(strip(expected))); console.log("got ", canon(strip(got))); }
}
console.log(`microformats suite: pestle ${ours}/${total} (${oursZones} with timezone spellings equal), microformats-parser ${theirs}/${total}`);
if (process.argv[3] !== "-v") console.log(fails.join("\n"));
