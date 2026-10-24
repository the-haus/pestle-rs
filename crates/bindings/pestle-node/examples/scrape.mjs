// Everything a scraper reads of a page, in one pass: what the page says of
// itself (title, language, canonical, robots, links, images, feeds, embedded
// JSON, a bot wall) and its structured data (JSON-LD, microdata, RDFa,
// microformats) as JSON-LD items. Prints a summary as JSON.
//
//   node scrape.mjs https://venue.example/agenda
//   node scrape.mjs page.html [https://venue.example/agenda]   (a saved page, and the URL it came from)

import { readFileSync } from "node:fs";

import { scrape } from "@pestle-rs/node";

const [input, base] = process.argv.slice(2);
if (!input) throw new Error("usage: node scrape.mjs <url | file [url]>");

// A fetch response is read as it arrives, in the charset of its Content-Type, against its own URL.
const { page, structured, charset } = /^https?:\/\//.test(input)
  ? await scrape(await fetch(input))
  : await scrape(readFileSync(input), { url: base });

if (page.blocked) {
  console.error(`blocked by ${page.blocked}: try again later, or slower`);
  process.exit(2);
}

// Every item of every format is JSON-LD of one shape; an untyped one is usually the page's OpenGraph (RDFa).
const types = {};
for (const item of structured.items) for (const t of [item["@type"] ?? "(untyped)"].flat()) types[t] = (types[t] ?? 0) + 1;

console.log(
  JSON.stringify(
    {
      title: page.title ?? null,
      lang: page.lang ?? null,
      canonical: page.canonical ?? null,
      charset,
      indexable: page.robots.index,
      links: page.links?.length ?? 0,
      images: page.images?.length ?? 0,
      feeds: page.feeds?.map((f) => f.href) ?? [],
      embedded: page.embedded?.map((e) => e.name) ?? [],
      types,
      events: structured.events.map((e) => ({ name: e.name, startDate: e.startDate, url: e.url, formats: e.formats ?? [e.format] })),
      warnings: [...(page.warnings ?? []), ...structured.warnings],
    },
    null,
    2,
  ),
);
