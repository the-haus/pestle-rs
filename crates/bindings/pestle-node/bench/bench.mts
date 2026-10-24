/**
 * Rust (pestle, through N-API) against TypeScript doing the same work.
 *
 *   HTML: festimap's per-page read today (htmlparser2's DOM, css-select
 *   queries, its regex readers for JSON-LD, links and feeds, and a second
 *   parse for the skeleton) against one pestle pass with the same fields.
 *   Grammars: the same grammar in pest (pestle) and in peggy (the PEG
 *   parser generator for JavaScript).
 *
 *   yarn build && node --expose-gc --import tsx bench/bench.mts [pages dir]
 *
 * Pages: every *.html in the directory (default ../../../bench/.work, which
 * is not committed; any saved pages do), else the repository's fixtures.
 */
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { availableParallelism } from "node:os";
import { join } from "node:path";

import { selectAll } from "css-select";
import { DomUtils, parseDocument } from "htmlparser2";
import peggy from "peggy";

import { Extractor, Grammar, hasGrammars, parseRobots, parseSitemap, readSitemap, strip } from "../dist/index.js";
import * as festimapCrawl from "../test/reference-crawl.js";
import { gzipSync, gunzipSync } from "node:zlib";
import { cleanForLearn, skeletonPaths, textOf } from "../test/reference.js";

// ── Timing ───────────────────────────────────────────────────────────────

function time(fn: () => void, minMs = 400): number {
  for (let i = 0; i < 3; i++) fn();
  const samples: number[] = [];
  const until = performance.now() + minMs;
  while (performance.now() < until || samples.length < 5) {
    const t = performance.now();
    fn();
    samples.push(performance.now() - t);
  }
  samples.sort((a, b) => a - b);
  return samples[Math.floor(samples.length / 2)]!;
}

const ms = (x: number) => (x < 1 ? `${(x * 1000).toFixed(0)} µs` : `${x.toFixed(2)} ms`);
const row = (cells: string[], widths: number[]) => cells.map((c, i) => c.padEnd(widths[i]!)).join("  ");

// ── Pages ────────────────────────────────────────────────────────────────

const here = import.meta.dirname;
const dir = process.argv[2] ?? join(here, "..", "..", "..", "..", "bench", ".work");
const fixtures = join(here, "..", "..", "..", "..", "fixtures", "sites");
const from = (d: string) =>
  readdirSync(d)
    .filter((f) => f.endsWith(".html") && statSync(join(d, f)).size > 10_000)
    .map((f) => ({ name: f.replace(/\.html$/, ""), html: readFileSync(join(d, f), "utf8") }));
let pages = [] as { name: string; html: string }[];
try {
  pages = from(dir);
} catch {
  /* no saved pages */
}
if (!pages.length) pages = readdirSync(fixtures).map((f) => ({ name: f, html: readFileSync(join(fixtures, f), "utf8") }));
const url = "https://venue.example/agenda/";

// ── The same read, both ways ─────────────────────────────────────────────

const ATTRS = { decodeEntities: true, lowerCaseTags: true };

/** festimap html.ts: an href made absolute, http(s) only, no fragment. */
function absolute(href: string, base: string): string | null {
  const trimmed = href.trim();
  if (!trimmed || trimmed.startsWith("#")) return null;
  let u: URL;
  try {
    u = new URL(trimmed.replace(/^webcal:/i, "https:"), base);
  } catch {
    return null;
  }
  if (u.protocol !== "https:" && u.protocol !== "http:") return null;
  u.hash = "";
  return u.toString();
}

/** festimap's page read today, the parts that touch the HTML. */
function viaTypeScript(html: string) {
  const doc = parseDocument(html, ATTRS);
  const one = (sel: string, attr?: string) => {
    const el = selectAll(sel, doc)[0];
    return el ? (attr ? (el.attribs[attr] ?? null) : textOf(el) || null) : null;
  };
  const data = {
    title: one("h1"),
    ogTitle: one("meta[property='og:title']", "content"),
    description: one("meta[name=description]", "content"),
    links: selectAll("a[href]", doc).map((a) => ({ url: absolute(a.attribs.href!, url), text: textOf(a).slice(0, 120) || null })),
    images: selectAll("img[src]", doc).flatMap((i) => absolute(i.attribs.src!, url) ?? []),
    times: selectAll("time[datetime]", doc).map((t) => t.attribs.datetime),
    headings: selectAll("h2, h3", doc).map((h) => textOf(h)).filter(Boolean),
    jsonLd: selectAll("script", doc)
      .filter((s) => (s.attribs.type ?? "").toLowerCase().startsWith("application/ld+json"))
      .flatMap((s) => {
        try {
          return [JSON.parse(DomUtils.textContent(s))];
        } catch {
          return [];
        }
      }),
    feeds: selectAll("link[type='text/calendar' i], a[href$='.ics' i]", doc).map((a) => a.attribs.href),
  };
  // skeletonHash(html) parses the page again.
  const hash = createHash("sha256").update([...skeletonPaths(html)].sort().join("\n")).digest("hex");
  return { data, hash };
}

const extractor = new Extractor(
  {
    title: "h1",
    ogTitle: "meta[property='og:title']@content",
    description: "meta[name=description]@content",
    links: { select: "a[href]", all: true, fields: { url: { attr: "href", url: "http" }, text: { max: 120 } } },
    images: { select: "img[src]", attr: "src", url: "http", all: true },
    times: ["time[datetime]@datetime"],
    headings: ["h2, h3"],
    jsonLd: { select: "script[type^='application/ld+json' i]", text: "raw", json: true, all: true },
    feeds: ["link[type='text/calendar' i], a[href$='.ics' i]@href"],
  },
  { skeleton: true },
);
const parseOnly = new Extractor({});

// ── HTML ─────────────────────────────────────────────────────────────────

console.log(`\nHTML: festimap's page read, ${pages.length} pages (median of repeated runs, one thread)\n`);
const w = [24, 9, 13, 13, 9, 13, 13, 9];
console.log(row(["page", "size", "TS read", "pestle read", "speed-up", "TS parse", "pestle parse", "speed-up"], w));
let sumTs = 0;
let sumRs = 0;
let bytes = 0;
for (const { name, html } of pages) {
  const ts = time(() => viaTypeScript(html));
  const rs = time(() => extractor.extract(html, { url }));
  const tsParse = time(() => parseDocument(html, ATTRS));
  const rsParse = time(() => parseOnly.extract(html));
  sumTs += ts;
  sumRs += rs;
  bytes += html.length;
  console.log(
    row(
      [name.slice(0, 24), `${(html.length / 1024).toFixed(0)} kB`, ms(ts), ms(rs), `${(ts / rs).toFixed(1)}×`, ms(tsParse), ms(rsParse), `${(tsParse / rsParse).toFixed(1)}×`],
      w,
    ),
  );
}
console.log(
  `\ntotal: TS ${ms(sumTs)}, pestle ${ms(sumRs)}: ${(sumTs / sumRs).toFixed(1)}× ` +
    `(${((bytes / 1048576) / (sumTs / 1000)).toFixed(0)} vs ${((bytes / 1048576) / (sumRs / 1000)).toFixed(0)} MB/s)`,
);

// Same answers? (links and headings compared; JSON-LD parsed the same.)
{
  const { html } = pages[0]!;
  const a = viaTypeScript(html);
  const b = extractor.extract(html, { url });
  const same = (k: string) =>
    JSON.stringify((a.data as Record<string, unknown>)[k]) === JSON.stringify((b.data as Record<string, unknown>)[k]);
  console.log(
    `same answers on ${pages[0]!.name}: ` +
      ["title", "links", "images", "times", "headings", "jsonLd", "feeds"].map((k) => `${k} ${same(k) ? "✓" : "✗"}`).join(", ") +
      `, skeleton ${a.hash === b.skeleton!.hash ? "✓" : "✗"}`,
  );
}

// Throughput: pestle's extractAsync runs pages on the libuv pool; the TS read holds the one JS thread.
{
  const batch = Array.from({ length: 32 }, (_, i) => pages[i % pages.length]!.html);
  const threads = Number(process.env.UV_THREADPOOL_SIZE ?? 4);
  const t0 = performance.now();
  for (const html of batch) viaTypeScript(html);
  const ts = performance.now() - t0;
  const t1 = performance.now();
  await Promise.all(batch.map((html) => extractor.extractAsync(html, { url })));
  const rs = performance.now() - t1;
  console.log(
    `32 pages: TS ${ms(ts)} on the main thread; pestle extractAsync ${ms(rs)} on ${threads} pool threads ` +
      `(${(ts / rs).toFixed(1)}×, ${availableParallelism()} cores; the JS thread stays free)`,
  );
}

// Strip for the learn prompt.
{
  const { name, html } = pages.reduce((a, b) => (a.html.length > b.html.length ? a : b));
  const ts = time(() => cleanForLearn(html));
  const rs = time(() => strip(html));
  console.log(`strip (cleanForLearn) on ${name}: TS ${ms(ts)}, pestle ${ms(rs)}: ${(ts / rs).toFixed(1)}×, same output ${cleanForLearn(html).html === strip(html).html ? "✓" : "✗"}`);
}

// Memory: the DOM a page becomes, against pestle's answer.
if (global.gc) {
  const { name, html } = pages.reduce((a, b) => (a.html.length > b.html.length ? a : b));
  const heap = () => {
    global.gc!();
    return process.memoryUsage().heapUsed;
  };
  const h0 = heap();
  const keep = [parseDocument(html, ATTRS)];
  const dom = heap() - h0;
  keep.length = 0;
  const h1 = heap();
  const out = [extractor.extract(html, { url })];
  const answer = heap() - h1;
  console.log(
    `memory on ${name} (${(html.length / 1048576).toFixed(1)} MB): htmlparser2's DOM ${(dom / 1048576).toFixed(1)} MB of JS heap; ` +
      `pestle keeps no DOM (its answer: ${(answer / 1048576).toFixed(2)} MB)`,
  );
  void out;
}

// ── Grammars ─────────────────────────────────────────────────────────────

console.log("\nCrawling: robots.txt and sitemaps, against festimap's robots.ts and sitemap.ts\n");
{
  const robotsTxt = [
    "User-agent: *", "Disallow: /admin", "Disallow: /*?sort=", "Disallow: /*.pdf$", "Allow: /admin/public",
    ...Array.from({ length: 60 }, (_, i) => `Disallow: /section-${i}/*/private`),
    "User-agent: festimapbot", "Disallow: /tickets", "Allow: /tickets/info", "Crawl-delay: 2", "Sitemap: https://v.example/sitemap.xml",
  ].join("\n");
  const urls = Array.from({ length: 100_000 }, (_, i) => `https://v.example/section-${i % 80}/x/${i % 3 ? "public" : "private"}/page-${i}?sort=${i % 5}`);
  const ours = parseRobots(robotsTxt, "festimapbot");
  const theirs = festimapCrawl.parseRobots(robotsTxt, "festimapbot");
  if (urls.some((u, i) => i < 2000 && ours.allows(u) !== festimapCrawl.robotsAllow(theirs, u))) throw new Error("robots disagree");
  const tOurs = time(() => urls.forEach((u) => ours.allows(u)));
  const tBatch = time(() => ours.allowsAll(urls));
  const tTheirs = time(() => urls.forEach((u) => festimapCrawl.robotsAllow(theirs, u)), 3000);
  console.log(`100,000 robots checks (66 rules): pestle ${ms(tOurs)} (${ms(tBatch)} in one allowsAll), festimap ${ms(tTheirs)}: ${(tTheirs / tOurs).toFixed(0)}×`);

  const sitemap =
    `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">` +
    Array.from({ length: 50_000 }, (_, i) => `<url><loc>https://v.example/agenda/event-${i}?lang=nl&amp;ref=sitemap</loc><lastmod>2026-11-${String((i % 28) + 1).padStart(2, "0")}T10:00:00+01:00</lastmod><changefreq>daily</changefreq><priority>0.${i % 10}</priority><image:image><image:loc>https://cdn.example/${i}.jpg</image:loc></image:image></url>`).join("\n") +
    "</urlset>";
  const bytes = new TextEncoder().encode(sitemap);
  const gz = gzipSync(bytes);
  const chunked = (b: Uint8Array) => (async function* () { for (let i = 0; i < b.length; i += 65536) yield b.subarray(i, i + 65536); })();
  const count = async (it: AsyncIterable<unknown>) => { let n = 0; for await (const _ of it) n++; return n; };
  const asyncTime = async (f: () => Promise<unknown>, rounds = 5) => { await f(); const t = performance.now(); for (let i = 0; i < rounds; i++) await f(); return (performance.now() - t) / rounds; };
  const nOurs = await count(parseSitemap(chunked(bytes)));
  const nTheirs = await count(festimapCrawl.parseSitemap(chunked(bytes)));
  if (nOurs !== nTheirs || nOurs !== 50_000) throw new Error(`sitemaps disagree: ${nOurs} ${nTheirs}`);
  const sOurs = await asyncTime(() => count(parseSitemap(chunked(bytes))));
  const sTheirs = await asyncTime(() => count(festimapCrawl.parseSitemap(chunked(bytes))));
  const sWhole = time(() => readSitemap(bytes));
  const sGzOurs = time(() => readSitemap(gz));
  const sGzTheirs = await asyncTime(() => count(festimapCrawl.parseSitemap(chunked(gunzipSync(gz)))));
  console.log(
    `a 50,000-URL sitemap (${(bytes.length / 1048576).toFixed(1)} MB): pestle streamed ${ms(sOurs)} (whole ${ms(sWhole)}), festimap ${ms(sTheirs)}: ${(sTheirs / sOurs).toFixed(1)}×; ` +
      `gzipped (${(gz.length / 1048576).toFixed(1)} MB) pestle ${ms(sGzOurs)}, festimap with zlib ${ms(sGzTheirs)}`,
  );
}

if (!hasGrammars()) {
  console.log("\n(pest grammars: not in this build; `yarn build:grammar` to compare them with peggy)");
} else {
console.log("\nGrammars: the same grammar in pest (pestle, Rust) and peggy (JavaScript)\n");

const dateSrc = {
  pest: `
    WHITESPACE = _{ " " }
    when    = { weekday? ~ #day = day ~ #month = month ~ (#year = year)? ~ ("·" ~ #time = time)? }
    weekday = @{ ("ma" | "di" | "wo" | "do" | "vr" | "za" | "zo") ~ "."? }
    day     = @{ ASCII_DIGIT{1,2} }
    month   = @{ ASCII_ALPHA+ }
    year    = @{ ASCII_DIGIT{4} }
    time    = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }`,
  peggy: `
    when = (weekday _)? day:day _ month:month year:(_ @year)? time:(_ "·" _ @time)? {
      const o = { day, month }; if (year) o.year = year; if (time) o.time = time; return o; }
    weekday = ("ma" / "di" / "wo" / "do" / "vr" / "za" / "zo") "."?
    day = $([0-9] [0-9]?)
    month = $[a-zA-Z]+
    year = $([0-9] [0-9] [0-9] [0-9])
    time = $([0-9] [0-9] ":" [0-9] [0-9])
    _ = " "*`,
};
const pestDate = new Grammar(dateSrc.pest);
const peggyDate = peggy.generate(dateSrc.peggy);
const dates = Array.from({ length: 2_000 }, (_, i) => `za ${(i % 28) + 1} november 2026 · ${String(18 + (i % 6)).padStart(2, "0")}:30`);
if (JSON.stringify(pestDate.shape(dates[0]!)) !== JSON.stringify(peggyDate.parse(dates[0]!))) {
  throw new Error(`the date grammars disagree: ${JSON.stringify(pestDate.shape(dates[0]!))} ${JSON.stringify(peggyDate.parse(dates[0]!))}`);
}
const reDate = /^(?:(?:ma|di|wo|do|vr|za|zo)\.? *)?(\d{1,2}) *([a-zA-Z]+)(?: *(\d{4}))?(?: *· *(\d{2}:\d{2}))?/;
{
  const pest = time(() => dates.forEach((d) => pestDate.shape(d)));
  const pg = time(() => dates.forEach((d) => peggyDate.parse(d)));
  const re = time(() => dates.forEach((d) => reDate.exec(d)));
  console.log(
    `2,000 short dates, one call each: pest ${ms(pest)} (${ms((pest / 2000))} a call), peggy ${ms(pg)}, a regex ${ms(re)}`,
  );
  // Inside an extraction the grammar runs in Rust per value, with no crossing.
  const html = `<ul>${dates.map((d) => `<li class=date>${d}</li>`).join("")}</ul>`;
  const x = new Extractor({ d: { select: "li.date", all: true, parse: "date" } }, { grammars: { date: pestDate } });
  const inRust = time(() => x.extract(html));
  const doc = () => {
    const d = parseDocument(html, ATTRS);
    return selectAll("li.date", d).map((li) => peggyDate.parse(textOf(li)));
  };
  const inTs = time(doc);
  console.log(`the same 2,000 dates read from HTML: pestle (selector + grammar in Rust) ${ms(inRust)}, htmlparser2 + css-select + peggy ${ms(inTs)}: ${(inTs / inRust).toFixed(1)}×`);
}

// A large input: JSON, with pest's own JSON grammar and peggy's.
{
  const jsonPest = readFileSync(join(here, "..", "..", "..", "pestle-grammar", "tests", "pest", "json.pest"), "utf8");
  const jsonPeggy = `
    value = _ v:(object / array / string / number / "true" / "false" / "null") _ { return v; }
    object = "{" _ head:pair? tail:("," @pair)* _ "}" { return head ? [head, ...tail] : []; }
    pair = _ k:string _ ":" v:value { return [k, v]; }
    array = "[" _ head:value? tail:("," @value)* _ "]" { return head ? [head, ...tail] : []; }
    string = $('"' ([^"\\\\] / "\\\\" .)* '"')
    number = $("-"? [0-9]+ ("." [0-9]+)? ([eE] [+-]? [0-9]+)?)
    _ = [ \\t\\n\\r]*`;
  const pest = new Grammar(jsonPest);
  const pg = peggy.generate(jsonPeggy);
  const big = JSON.stringify(
    Array.from({ length: 6_000 }, (_, i) => ({ id: i, name: `Event ${i}`, acts: ["A", "B", "C"], price: 25.5, soldOut: i % 7 === 0 })),
  );
  const tPest = time(() => pest.parse(big, "json"), 1500);
  const tPeggy = time(() => pg.parse(big), 1500);
  const tNative = time(() => JSON.parse(big));
  const pairs = pest.parse(big, "json").size;
  console.log(
    `${(big.length / 1048576).toFixed(1)} MB of JSON: pest ${ms(tPest)} (${pairs.toLocaleString()} pairs to JS), peggy ${ms(tPeggy)}: ` +
      `${(tPeggy / tPest).toFixed(1)}×; JSON.parse, for scale, ${ms(tNative)}`,
  );
}
}
