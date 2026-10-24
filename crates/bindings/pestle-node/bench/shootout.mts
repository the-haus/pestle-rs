/**
 * pestle against the JavaScript parsers, on the same work.
 *
 *   HTML: links (href and text), headings (h2, h3), images (src) and the
 *   title (h1) from every saved page, with pestle, htmlparser2 + css-select
 *   (festimap's stack), cheerio (parse5), node-html-parser, linkedom, and
 *   a hand-written htmlparser2 SAX handler (no selectors: the least work a
 *   JavaScript parser can do).
 *   Grammars: JSON with pest (pestle), peggy and Chevrotain; short dates one
 *   call at a time and in one batch.
 *
 *   yarn build && node --import tsx bench/shootout.mts [pages dir]
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import * as cheerio from "cheerio";
import { createToken, CstParser, Lexer } from "chevrotain";
import { selectAll } from "css-select";
import { DomUtils, parseDocument, Parser } from "htmlparser2";
import { parseHTML } from "linkedom";
import { parse as nhpParse } from "node-html-parser";
import peggy from "peggy";

import { Extractor, Grammar, hasGrammars } from "../dist/index.js";

const here = import.meta.dirname;
const dir = process.argv[2] ?? join(here, "..", "..", "..", "..", "bench", ".work");
const pages = readdirSync(dir)
  .filter((f) => f.endsWith(".html") && statSync(join(dir, f)).size > 10_000)
  .map((f) => readFileSync(join(dir, f), "utf8"));
const bytes = pages.reduce((n, p) => n + Buffer.byteLength(p), 0);

function time(fn: () => void, minMs = 1500): number {
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

type Found = { links: number; headings: number; images: number; title: boolean };

// ── HTML ─────────────────────────────────────────────────────────────────

const extractor = new Extractor({
  links: { select: "a[href]", all: true, fields: { href: { attr: "href", collapse: false }, text: { text: "raw" } } },
  headings: { select: "h2, h3", all: true, text: "raw", default: [] },
  images: { select: "img[src]", attr: "src", all: true, collapse: false },
  title: { select: "h1", text: "raw", default: "" },
});

const libraries: Record<string, (html: string) => Found> = {
  pestle(html) {
    const d = extractor.extract(html).data as { links: unknown[]; headings: unknown[]; images: unknown[]; title: string };
    return { links: d.links.length, headings: d.headings.length, images: d.images.length, title: d.title !== "" };
  },
  "htmlparser2 + css-select"(html) {
    const doc = parseDocument(html, { decodeEntities: true, lowerCaseTags: true });
    const links = selectAll("a[href]", doc).map((a) => [a.attribs.href, DomUtils.textContent(a)]);
    const headings = selectAll("h2, h3", doc).map((h) => DomUtils.textContent(h));
    const images = selectAll("img[src]", doc).map((i) => i.attribs.src);
    const title = selectAll("h1", doc)[0];
    return { links: links.length, headings: headings.length, images: images.length, title: !!title };
  },
  cheerio(html) {
    const $ = cheerio.load(html);
    const links = $("a[href]").map((_, a) => [[$(a).attr("href"), $(a).text()]]).get();
    const headings = $("h2, h3").map((_, h) => $(h).text()).get();
    const images = $("img[src]").map((_, i) => $(i).attr("src")).get();
    return { links: links.length, headings: headings.length, images: images.length, title: $("h1").length > 0 };
  },
  "node-html-parser"(html) {
    const root = nhpParse(html);
    const links = root.querySelectorAll("a[href]").map((a) => [a.getAttribute("href"), a.textContent]);
    const headings = root.querySelectorAll("h2, h3").map((h) => h.textContent);
    const images = root.querySelectorAll("img[src]").map((i) => i.getAttribute("src"));
    return { links: links.length, headings: headings.length, images: images.length, title: !!root.querySelector("h1") };
  },
  linkedom(html) {
    const { document } = parseHTML(html);
    const links = [...document.querySelectorAll("a[href]")].map((a) => [a.getAttribute("href"), a.textContent]);
    const headings = [...document.querySelectorAll("h2, h3")].map((h) => h.textContent);
    const images = [...document.querySelectorAll("img[src]")].map((i) => i.getAttribute("src"));
    return { links: links.length, headings: headings.length, images: images.length, title: !!document.querySelector("h1") };
  },
  "htmlparser2 SAX, by hand"(html) {
    const links: [string, string][] = [];
    const headings: string[] = [];
    const images: string[] = [];
    let title: string | null = null;
    const open: (string | null)[] = [];
    const p = new Parser(
      {
        onopentag(name, attrs) {
          if (name === "a" && attrs.href !== undefined) {
            links.push([attrs.href, ""]);
            open.push("a");
          } else if (name === "h2" || name === "h3") {
            headings.push("");
            open.push("h");
          } else if (name === "h1" && title === null) {
            title = "";
            open.push("t");
          } else open.push(null);
          if (name === "img" && attrs.src !== undefined) images.push(attrs.src);
        },
        ontext(t) {
          for (const k of open) {
            if (k === "a") links[links.length - 1]![1] += t;
            else if (k === "h") headings[headings.length - 1] += t;
            else if (k === "t") title += t;
          }
        },
        onclosetag() {
          open.pop();
        },
      },
      { decodeEntities: true, lowerCaseTags: true },
    );
    p.end(html);
    return { links: links.length, headings: headings.length, images: images.length, title: title !== null };
  },
};

console.log(`\nHTML: links, headings, images and title from ${pages.length} pages (${(bytes / 1048576).toFixed(1)} MB), median of repeated runs\n`);
const results: [string, number][] = [];
for (const [name, fn] of Object.entries(libraries)) {
  const ms = time(() => {
    for (const p of pages) fn(p);
  });
  results.push([name, ms]);
}
const fastestJs = Math.min(...results.filter(([n]) => n !== "pestle").map(([, ms]) => ms));
const pestleMs = results.find(([n]) => n === "pestle")![1];
for (const [name, ms] of results) {
  const mbs = bytes / 1048576 / (ms / 1000);
  const vs = name === "pestle" ? `${(fastestJs / ms).toFixed(1)}× the fastest JS` : `pestle is ${(ms / pestleMs).toFixed(1)}× faster`;
  console.log(`${name.padEnd(28)} ${ms.toFixed(1).padStart(7)} ms  ${mbs.toFixed(0).padStart(5)} MB/s   ${vs}`);
}
{
  const batch = Array.from({ length: 32 }, (_, i) => pages[i % pages.length]!);
  const t = performance.now();
  await Promise.all(batch.map((html) => extractor.extractAsync(html)));
  const ms = performance.now() - t;
  console.log(`pestle extractAsync, 32 pages over the thread pool: ${(ms).toFixed(1)} ms (${((batch.reduce((n, p) => n + p.length, 0) / 1048576) / (ms / 1000)).toFixed(0)} MB/s), the JS thread free`);
}

// ── Grammars ─────────────────────────────────────────────────────────────

if (!hasGrammars()) {
  console.log("\n(pest grammars: not in this build; `yarn build:grammar` to compare them with peggy and Chevrotain)");
  process.exit(0);
}

const jsonPest = new Grammar(readFileSync(join(here, "..", "..", "..", "pestle-grammar", "tests", "pest", "json.pest"), "utf8"));
const jsonPeggy = peggy.generate(`
  value = _ v:(object / array / string / number / "true" / "false" / "null") _ { return v; }
  object = "{" _ head:pair? tail:("," @pair)* _ "}" { return head ? [head, ...tail] : []; }
  pair = _ k:string _ ":" v:value { return [k, v]; }
  array = "[" _ head:value? tail:("," @value)* _ "]" { return head ? [head, ...tail] : []; }
  string = $('"' ([^"\\\\] / "\\\\" .)* '"')
  number = $("-"? [0-9]+ ("." [0-9]+)? ([eE] [+-]? [0-9]+)?)
  _ = [ \\t\\n\\r]*`);

// Chevrotain's own JSON example (CST mode, its fastest).
const T = {
  True: createToken({ name: "True", pattern: /true/ }),
  False: createToken({ name: "False", pattern: /false/ }),
  Null: createToken({ name: "Null", pattern: /null/ }),
  LCurly: createToken({ name: "LCurly", pattern: /{/ }),
  RCurly: createToken({ name: "RCurly", pattern: /}/ }),
  LSquare: createToken({ name: "LSquare", pattern: /\[/ }),
  RSquare: createToken({ name: "RSquare", pattern: /]/ }),
  Comma: createToken({ name: "Comma", pattern: /,/ }),
  Colon: createToken({ name: "Colon", pattern: /:/ }),
  StringLiteral: createToken({ name: "StringLiteral", pattern: /"(?:[^\\"]|\\(?:[bfnrtv"\\/]|u[0-9a-fA-F]{4}))*"/ }),
  NumberLiteral: createToken({ name: "NumberLiteral", pattern: /-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?/ }),
  WhiteSpace: createToken({ name: "WhiteSpace", pattern: /[ \t\n\r]+/, group: Lexer.SKIPPED }),
};
const allTokens = [T.WhiteSpace, T.NumberLiteral, T.StringLiteral, T.LCurly, T.RCurly, T.LSquare, T.RSquare, T.Comma, T.Colon, T.True, T.False, T.Null];
const jsonLexer = new Lexer(allTokens, { positionTracking: "onlyOffset" });
class JsonParser extends CstParser {
  constructor() {
    super(allTokens, { recoveryEnabled: false });
    this.performSelfAnalysis();
  }
  json = this.RULE("json", () => this.OR([{ ALT: () => this.SUBRULE(this.object) }, { ALT: () => this.SUBRULE(this.array) }]));
  object = this.RULE("object", () => {
    this.CONSUME(T.LCurly);
    this.MANY_SEP({ SEP: T.Comma, DEF: () => this.SUBRULE(this.objectItem) });
    this.CONSUME(T.RCurly);
  });
  objectItem = this.RULE("objectItem", () => {
    this.CONSUME(T.StringLiteral);
    this.CONSUME(T.Colon);
    this.SUBRULE(this.value);
  });
  array = this.RULE("array", () => {
    this.CONSUME(T.LSquare);
    this.MANY_SEP({ SEP: T.Comma, DEF: () => this.SUBRULE(this.value) });
    this.CONSUME(T.RSquare);
  });
  value = this.RULE("value", () =>
    this.OR([
      { ALT: () => this.CONSUME(T.StringLiteral) },
      { ALT: () => this.CONSUME(T.NumberLiteral) },
      { ALT: () => this.SUBRULE(this.object) },
      { ALT: () => this.SUBRULE(this.array) },
      { ALT: () => this.CONSUME(T.True) },
      { ALT: () => this.CONSUME(T.False) },
      { ALT: () => this.CONSUME(T.Null) },
    ]),
  );
}
const chevrotain = new JsonParser();
const chevrotainParse = (text: string) => {
  chevrotain.input = jsonLexer.tokenize(text).tokens;
  return chevrotain.json();
};

const big = JSON.stringify(
  Array.from({ length: 6_000 }, (_, i) => ({ id: i, name: `Event ${i}`, acts: ["A", "B", "C"], price: 25.5, soldOut: i % 7 === 0 })),
);
console.log(`\nGrammars: ${(big.length / 1048576).toFixed(1)} MB of JSON, a full parse tree in JavaScript\n`);
const g: [string, number][] = [
  ["pestle (pest grammar)", time(() => jsonPest.parse(big, "json"))],
  ["Chevrotain (CST)", time(() => chevrotainParse(big))],
  ["peggy", time(() => jsonPeggy.parse(big))],
];
for (const [name, ms] of g) {
  console.log(`${name.padEnd(28)} ${ms.toFixed(2).padStart(7)} ms  ${(big.length / 1048576 / (ms / 1000)).toFixed(0).padStart(5)} MB/s`);
}
console.log(`JSON.parse (a hand-written native JSON parser, not a grammar engine), for scale: ${time(() => JSON.parse(big)).toFixed(2)} ms`);

const date = new Grammar(`
  WHITESPACE = _{ " " }
  when    = { weekday? ~ #day = day ~ #month = month ~ (#year = year)? ~ ("·" ~ #time = time)? }
  weekday = @{ ("ma" | "di" | "wo" | "do" | "vr" | "za" | "zo") ~ "."? }
  day     = @{ ASCII_DIGIT{1,2} }
  month   = @{ ASCII_ALPHA+ }
  year    = @{ ASCII_DIGIT{4} }
  time    = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }`);
const datePeggy = peggy.generate(`
  when = (weekday _)? day:day _ month:month year:(_ @year)? time:(_ "·" _ @time)? {
    const o = { day, month }; if (year) o.year = year; if (time) o.time = time; return o; }
  weekday = ("ma" / "di" / "wo" / "do" / "vr" / "za" / "zo") "."?
  day = $([0-9] [0-9]?)
  month = $[a-zA-Z]+
  year = $([0-9] [0-9] [0-9] [0-9])
  time = $([0-9] [0-9] ":" [0-9] [0-9])
  _ = " "*`);
const dates = Array.from({ length: 10_000 }, (_, i) => `za ${(i % 28) + 1} november 2026 · ${String(18 + (i % 6)).padStart(2, "0")}:30`);
console.log(`\nGrammars: 10,000 short dates into objects\n`);
const d: [string, number][] = [
  ["pestle shapeAll (one call)", time(() => date.shapeAll(dates))],
  ["pestle shape (a call each)", time(() => dates.forEach((x) => date.shape(x)))],
  ["peggy (a call each)", time(() => dates.forEach((x) => datePeggy.parse(x)))],
];
for (const [name, ms] of d) console.log(`${name.padEnd(28)} ${ms.toFixed(2).padStart(7)} ms  ${((ms * 1e6) / dates.length).toFixed(0).padStart(5)} ns a date`);
