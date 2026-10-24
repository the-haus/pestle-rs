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
  messyHtml,
  parse,
  random,
  select,
  skeletonPaths,
  textOf,
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

  it("scoped as css-select scopes an element", () => {
    const fields = ["p", "> p", "li:first-child", "div p", ":scope > *", "a + a"];
    for (let seed = 500; seed < 600; seed++) {
      const html = messyHtml(random(seed));
      const doc = parse(html);
      for (const field of fields) {
        const want = select(doc, "div").map((scope) =>
          select(scope, field)
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
});
