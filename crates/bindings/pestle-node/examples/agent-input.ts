// A page as input for an AI agent: its accessibility snapshot in
// Playwright's `ai` mode (roles, names, states, a ref on everything that can
// be acted on, each with a CSS selector for a browser to act on it), with
// text the page hides from sight left out, since hidden text is where
// instructions get slipped in. No browser is started.
//
//   npx tsx agent-input.ts https://venue.example/agenda
//   npx tsx agent-input.ts page.html

import { readFileSync } from "node:fs";

import { ariaSnapshotAsync, charsetOf, type AriaSnapshot } from "@pestle-rs/node";

const [input] = process.argv.slice(2);
if (!input) throw new Error("usage: agent-input.ts <url | file>");

let html: Uint8Array;
let charset: string | undefined;
if (/^https?:\/\//.test(input)) {
  const res = await fetch(input);
  html = new Uint8Array(await res.arrayBuffer());
  charset = charsetOf(res.headers.get("content-type"));
} else {
  html = readFileSync(input);
}

// On the thread pool: an agent serving many pages keeps its JS thread free.
const { snapshot, refs }: AriaSnapshot = await ariaSnapshotAsync(html, { mode: "ai", concealed: true, charset });

console.log(snapshot);
console.log("\nrefs:");
for (const [ref, selector] of Object.entries(refs)) console.log(`  ${ref}: ${selector}`);
