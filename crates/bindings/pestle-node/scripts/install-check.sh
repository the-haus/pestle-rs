#!/bin/sh
# Install the packed @pestle-rs/node from packages/ (see release-node.yml)
# into an empty project, with the platform package for one triple, and use it
# from ESM and CommonJS. Run from the repository root:
#
#   sh crates/bindings/pestle-node/scripts/install-check.sh linux-x64-gnu
set -eu
TRIPLE="$1"
rm -rf install-check
mkdir install-check
cd install-check
npm init -y >/dev/null
# Relative paths, which npm on Windows understands too. In a dry run the
# other platform packages are not on the registry; npm skips optional
# dependencies it cannot fetch.
npm install --no-audit --no-fund ../packages/pestle-rs-node-[0-9]*.tgz "../packages/pestle-rs-node-$TRIPLE-"*.tgz
ls node_modules/@pestle-rs
cat > esm.mjs <<'JS'
import { readFileSync } from "node:fs";
import { Extractor, parseRobots, readSitemap, scrape } from "@pestle-rs/node";
const html = readFileSync("node_modules/@pestle-rs/node/examples/agenda.html");
const { data } = new Extractor({ events: { select: "article.event", all: true, fields: { title: "h2", price: { select: ".price", number: true } } } })
  .extract(html, { url: "https://venue.example/agenda" });
if (data.events.length !== 2 || data.events[0].price !== 25) throw new Error(`unexpected ${JSON.stringify(data)}`);
const { structured } = await scrape(html, { url: "https://venue.example/agenda" });
if (structured.events.length !== 2) throw new Error(`expected 2 events, got ${structured.events.length}`);
if (parseRobots("User-agent: *\nDisallow: /x", "bot").allows("/x/1")) throw new Error("robots.txt not read");
if (readSitemap("<urlset><url><loc>https://s.example/p</loc></url></urlset>").entries.length !== 1) throw new Error("sitemap not read");
console.log(`esm: ${process.platform}-${process.arch}, node ${process.version}: ${data.events.length} events`);
JS
cat > cjs.cjs <<'JS'
const { extractPage, ariaSnapshot } = require("@pestle-rs/node");
const page = extractPage('<title>T</title><a href="/a">A</a>', { url: "https://s.example/" });
if (page.links[0].href !== "https://s.example/a") throw new Error(`unexpected ${JSON.stringify(page)}`);
if (!ariaSnapshot("<button>Buy</button>").snapshot.includes('button "Buy"')) throw new Error("no snapshot");
console.log(`cjs: ${page.title}`);
JS
node esm.mjs
node cjs.cjs
node -e 'require.resolve("@pestle-rs/node/package.json"); console.log("package.json: exported")'
