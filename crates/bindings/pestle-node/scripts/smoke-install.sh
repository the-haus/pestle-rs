#!/bin/bash
# Pack @pestle-rs/node, install the tarball into a fresh project, and use it
# from ESM, CommonJS and TypeScript (strict, NodeNext).
set -euo pipefail
PKG=$(cd "$(dirname "$0")/.." && pwd)
W=$(mktemp -d)
V=$(node -p "require('$PKG/package.json').version")
cd "$PKG"
rm -f "pestle-rs-node-$V.tgz"
npm pack --ignore-scripts >/dev/null
mv "pestle-rs-node-$V.tgz" "$W/"
cd "$W"
npm init -y >/dev/null
npm install "./pestle-rs-node-$V.tgz" typescript@5 @types/node@22 >/dev/null 2>&1
node -e 'require.resolve("@pestle-rs/node/package.json")'
ls node_modules/@pestle-rs/node
cp node_modules/@pestle-rs/node/examples/agenda.html .
cat > esm.mjs <<'JS'
import { readFileSync } from "node:fs";
import { scrape, parseRobots } from "@pestle-rs/node";
const { page, structured } = await scrape(readFileSync("agenda.html"), { url: "https://venue.example/agenda" });
if (structured.events.length !== 2) throw new Error(`expected 2, got ${structured.events.length}`);
if (parseRobots("User-agent: *\nDisallow: /", "bot").allows("/a")) throw new Error("robots.txt not read");
console.log("esm", page.title, structured.events.length);
JS
cat > cjs.cjs <<'JS'
const { extractStructured } = require("@pestle-rs/node");
const { events } = extractStructured(require("fs").readFileSync("agenda.html"));
if (events.length !== 2) throw new Error(`expected 2, got ${events.length}`);
console.log("cjs", events.length);
JS
cat > app.ts <<'TS'
import { readFileSync } from "node:fs";
import { Extractor, type ExtractResult, type StructuredEvent } from "@pestle-rs/node";

interface Agenda {
  events: { title: string; price: number | null; acts: string[] }[];
}
const x = new Extractor<Agenda>(
  { events: { select: "article.event", all: true, fields: { title: "h2", price: { select: ".price", number: true }, acts: ["li"] } } },
  { structured: true },
);
const out: ExtractResult<Agenda> = x.extract(readFileSync("agenda.html"), { url: "https://venue.example/agenda" });
const first: StructuredEvent | undefined = out.structured?.events[0];
console.log("ts", out.data.events.length, out.data.events[0]?.acts.length, first?.name);
TS
echo '{"compilerOptions":{"module":"NodeNext","moduleResolution":"NodeNext","target":"ES2022","strict":true,"outDir":"out","rootDir":".","types":["node"]}}' > tsconfig.json
node esm.mjs
node cjs.cjs
npx tsc -p .
node out/app.js
rm -rf "$W"
