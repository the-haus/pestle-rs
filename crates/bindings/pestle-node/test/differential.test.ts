/**
 * pestle against what festimap runs today (htmlparser2 + css-select, and
 * its readers on top): the same elements matched, the same text read, the
 * same skeleton and the same stripped page, on festimap's fixtures and on
 * thousands of random, badly broken documents.
 */
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { Extractor, skeleton, strip } from "../ts/index.js";
import {
  cleanForLearn,
  collapse,
  is,
  messyHtml,
  parse,
  random,
  select,
  skeletonPaths,
  textOf,
  type El,
} from "./reference.js";

const FIXTURES = join(__dirname, "..", "..", "..", "..", "fixtures", "sites");
/** Real pages saved for the benchmark (bench/.work, not committed), when there are any. */
const SAVED = join(__dirname, "..", "..", "..", "..", "bench", ".work");
const htmlIn = (dir: string) => {
  try {
    return readdirSync(dir)
      .filter((f) => f.endsWith(".html"))
      .map((f) => ({ name: f, html: readFileSync(join(dir, f), "utf8") }));
  } catch {
    return [];
  }
};
const pages = [...htmlIn(FIXTURES), ...htmlIn(SAVED)];

/** Selectors festimap's recipes use, and the rest of what pestle matches. */
const SELECTORS = [
  "div", "p", "li", "a", "td", "span", "b", "option", "path", "title",
  "div > p", "ul li", "ul > li", "table td", "li + li", "p ~ span", "h1 + p", "td ~ td", "h1, h2", "div p span",
  ".a", ".a.b", ".Event", "[class]", "[data-n]", "[href^='/']", "a[href$='1']", "[href*=example]", "[class~=x-1]",
  "[class|=x]", "[class!=a]", "[CLASS=A i]", "#i3", "[hidden]", "[data-v]",
  "li:first-child", "li:nth-child(2n+1)", "li:nth-child(-n+2)", "p:first-of-type", "td:nth-of-type(2)", ":root",
  "div:not(.a)", ":is(td, th)", "div:not(p > div)", ":where(.b) > *",
  "li:last-child", "li:only-child", "p:last-of-type", "td:nth-last-child(1)", "li:nth-last-of-type(2)",
  "p:empty", "div:has(> p)", "div:has(li.a)", "ul:has(+ p)", "td:contains('Nora')", "p:icontains('ZAAL')",
];

/** The `data-n` of each element a selector matches, by css-select on htmlparser2's DOM. */
function reference(html: string, selector: string): string[] {
  return select(parse(html), selector)
    .map((el) => el.attribs["data-n"])
    .filter((n): n is string => n !== undefined);
}

function ours(html: string, selector: string): string[] {
  const x = new Extractor({ n: { select: selector, attr: "data-n", all: true } });
  return x.extract(html).data.n as string[];
}

function supported(selector: string): boolean {
  try {
    new Extractor({ n: selector });
    return true;
  } catch {
    return false;
  }
}

describe("matches what css-select matches on htmlparser2's tree", () => {
  it("on random broken documents", () => {
    const usable = SELECTORS.filter(supported);
    expect(usable.length).toBeGreaterThan(SELECTORS.length - 3);
    let compared = 0;
    for (let seed = 1; seed <= 400; seed++) {
      const html = messyHtml(random(seed));
      for (const selector of usable) {
        const want = reference(html, selector);
        const got = ours(html, selector);
        if (JSON.stringify(got) !== JSON.stringify(want)) {
          throw new Error(`seed ${seed}, ${selector}\nwant ${want}\ngot  ${got}\n${html}`);
        }
        compared += want.length;
      }
    }
    expect(compared).toBeGreaterThan(50_000);
  });

  it("scoped as querySelectorAll scopes an element (css-select without relative selectors)", () => {
    // Matched in the page's context; one that starts at the scope (`> p`, `:scope`) as css-select reads it.
    const fields = ["p", "> p", "li:first-child", "div p", "div > p", "section div p", ":scope > *", ":scope p", "a + a", "li ~ li a"];
    for (let seed = 500; seed < 600; seed++) {
      const html = messyHtml(random(seed));
      const doc = parse(html);
      for (const field of fields) {
        const fromScope = field.startsWith(">") || field.includes(":scope");
        const want = select(doc, "div").map((scope) =>
          select(scope, field, !fromScope)
            .map((el) => el.attribs["data-n"])
            .filter((n): n is string => n !== undefined),
        );
        const x = new Extractor({ s: { select: "div", all: true, fields: { n: { select: field, attr: "data-n", all: true } } } });
        const got = (x.extract(html).data.s as { n: string[] }[]).map((s) => s.n);
        if (JSON.stringify(got) !== JSON.stringify(want)) {
          throw new Error(`seed ${seed}, div → ${field}\nwant ${JSON.stringify(want)}\ngot  ${JSON.stringify(got)}\n${html}`);
        }
      }
    }
  });
});

describe("closest reads as festimap's closestFields does", () => {
  /** The nearest value: under the row, else under or on its ancestors (each its first match, else itself). */
  function closest(row: El, selector: string, attr: string, depth: number): string | null {
    const valued = (els: El[]) => els.find((el) => el.attribs[attr] !== undefined)?.attribs[attr] ?? null;
    const under = valued(select(row, selector, true));
    if (under !== null) return under;
    let at: El | null = row;
    for (let k = 1; k <= depth; k++) {
      const parent: unknown = at?.parent;
      if (!parent || !(parent as El).attribs) return null;
      at = parent as El;
      const found = valued(select(at, selector, true)) ?? (is(at, selector) ? valued([at]) : null);
      if (found !== null) return found;
    }
    return null;
  }

  it("on random broken documents, whole and streamed", async () => {
    let compared = 0;
    for (let seed = 2000; seed < 2150; seed++) {
      const html = messyHtml(random(seed));
      const doc = parse(html);
      for (const [rows, selector, depth] of [
        ["li", ".b, .c", 8],
        ["td, li", "[data-v]", 3],
        ["p", "h1, h2", 2],
        ["div", ".Event", 8],
      ] as const) {
        const want = select(doc, rows).map((row) => ({
          n: row.attribs["data-n"] ?? null,
          v: closest(row, selector, "id", depth),
        }));
        const x = new Extractor({
          r: { select: rows, all: true, fields: { n: { attr: "data-n" }, v: { select: selector, attr: "id", closest: { depth } } } },
        });
        const got = x.extract(html).data.r;
        if (JSON.stringify(got) !== JSON.stringify(want)) {
          throw new Error(`seed ${seed}, ${rows} → ${selector} (${depth})\nwant ${JSON.stringify(want)}\ngot  ${JSON.stringify(got)}\n${html}`);
        }
        const bytes = new TextEncoder().encode(html);
        const pieces = Array.from({ length: Math.ceil(bytes.length / 5) }, (_, i) => bytes.subarray(i * 5, i * 5 + 5));
        expect((await x.stream(pieces)).data.r, `seed ${seed} streamed`).toEqual(want);
        compared += want.filter((w) => w.v !== null).length;
      }
    }
    expect(compared).toBeGreaterThan(1000);
  });
});

describe("reads what festimap reads", () => {
  it("text, as textOf", () => {
    for (let seed = 700; seed < 900; seed++) {
      const html = messyHtml(random(seed));
      for (const selector of ["div", "li", "p", "a", "td", "section", "h1"]) {
        const want = select(parse(html), selector)
          .map((el) => textOf(el))
          .filter(Boolean);
        const got = new Extractor({ t: [selector] }).extract(html).data.t;
        expect(got, `seed ${seed}, ${selector}`).toEqual(want);
      }
    }
  });

  it("attributes, collapsed as readElement does", () => {
    for (let seed = 900; seed < 1000; seed++) {
      const html = messyHtml(random(seed));
      const want = select(parse(html), "[class]").map((el) => collapse(el.attribs.class!)).filter(Boolean);
      expect(new Extractor({ c: ["[class]@class"] }).extract(html).data.c, `seed ${seed}`).toEqual(want);
    }
  });

  it("the skeleton, as skeletonPaths", () => {
    for (const { name, html } of pages) {
      expect(skeleton(html, { paths: true }).paths, name).toEqual([...skeletonPaths(html)].sort());
    }
    for (let seed = 1000; seed < 1300; seed++) {
      const html = messyHtml(random(seed));
      expect(skeleton(html, { paths: true }).paths, `seed ${seed}`).toEqual([...skeletonPaths(html)].sort());
    }
  });

  it("the stripped page, as cleanForLearn", () => {
    for (const { name, html } of pages) {
      expect(strip(html).html, name).toBe(cleanForLearn(html).html);
    }
    for (let seed = 1300; seed < 1600; seed++) {
      const html = messyHtml(random(seed));
      expect(strip(html).html, `seed ${seed}`).toBe(cleanForLearn(html).html);
    }
  });

  it("the stripped page cut where cleanForLearn cuts it, in UTF-16 units", () => {
    // Characters past U+FFFF (two units each) before every space, and budgets that cut
    // the head, the body, a text run, an attribute. Where festimap cuts one of them in
    // half it keeps a lone surrogate; pestle writes U+FFFD, the same bytes in UTF-8.
    const astral = (s: string) => s.replace(/(\w{3}) /g, "$1𝕊ᑭ🎸 ");
    const utf8 = (s: string) => Buffer.from(s).toString("hex");
    for (const { name, html } of pages) {
      for (const page of [html, astral(html)]) {
        for (const tokens of [40, 150, 600, 2_500, 8_000]) {
          const got = strip(page, { maxChars: tokens * 3 });
          const want = cleanForLearn(page, tokens);
          expect(got.html.length, `${name} at ${tokens}`).toBe(want.html.length);
          expect(utf8(got.html), `${name} at ${tokens}`).toBe(utf8(want.html));
          expect(got.truncated, `${name} at ${tokens}`).toBe(want.truncated);
          expect(got.tokens).toBe(Math.ceil(got.html.length / 3));
        }
      }
    }
    for (let seed = 1600; seed < 1800; seed++) {
      const html = astral(messyHtml(random(seed), 600));
      for (const tokens of [20, 60, 200]) {
        expect(utf8(strip(html, { maxChars: tokens * 3 }).html), `seed ${seed} at ${tokens}`).toBe(
          utf8(cleanForLearn(html, tokens).html),
        );
      }
    }
  });
});
