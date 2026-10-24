/**
 * Microdata held to the WHATWG HTML standard (§ 5.2): the algorithms "crawl the properties", "the
 * property value of a name-value pair" and "get the object" (§ 5.2.6, JSON), written here from
 * the spec on htmlparser2's DOM, an implementation apart from pestle's streaming reader, and
 * compared with pestle's strict (`lenient: false`) microdata JSON on random documents.
 */
import { describe, expect, it } from "vitest";
import { DomUtils, ElementType, parseDocument } from "htmlparser2";

import { extractStructured } from "../ts/index.js";
import { random } from "./reference.js";

type Doc = ReturnType<typeof parseDocument>;
type Node = Doc["children"][number];
type El = Parameters<typeof DomUtils.getAttributeValue>[0];
const isEl = (n: Node): n is El => ElementType.isTag(n);
const attr = (e: El, a: string) => (a in e.attribs ? e.attribs[a]! : undefined);
const tokens = (v: string | undefined) => [...new Set((v ?? "").split(/[\t\n\f\r ]+/).filter(Boolean))];

function treeOrder(doc: Doc): Map<El, number> {
  const order = new Map<El, number>();
  let i = 0;
  const walk = (ns: Node[]) => {
    for (const n of ns) if (isEl(n)) {
      order.set(n, i++);
      walk(n.children as Node[]);
    }
  };
  walk(doc.children as Node[]);
  return order;
}

/** "Crawl the properties" (§ 5.2.5). */
function properties(root: El, doc: Doc, order: Map<El, number>): El[] {
  const results: El[] = [];
  const memory = new Set<El>([root]);
  const pending: El[] = (root.children as Node[]).filter(isEl);
  for (const id of tokens(attr(root, "itemref"))) {
    const found = DomUtils.findOne((e) => e.attribs.id === id, doc.children as Node[], true);
    if (found) pending.push(found);
  }
  while (pending.length) {
    const current = pending.shift()!;
    if (memory.has(current)) continue;
    memory.add(current);
    if (attr(current, "itemscope") === undefined) pending.push(...(current.children as Node[]).filter(isEl));
    if (attr(current, "itemprop") !== undefined && tokens(attr(current, "itemprop")).length) results.push(current);
  }
  return results.sort((a, b) => order.get(a)! - order.get(b)!);
}

const url = (v: string | undefined, base: string) => {
  if (v === undefined) return "";
  try {
    return new URL(v.trim(), base).href;
  } catch {
    return "";
  }
};

/** "The property value of a name-value pair" (§ 5.2.4). */
function value(e: El, base: string): string | { item: El } {
  if (attr(e, "itemscope") !== undefined) return { item: e };
  switch (e.name) {
    case "meta": return attr(e, "content") ?? "";
    case "audio": case "embed": case "iframe": case "img": case "source": case "track": case "video": return url(attr(e, "src"), base);
    case "a": case "area": case "link": return url(attr(e, "href"), base);
    case "object": return url(attr(e, "data"), base);
    case "data": case "meter": return attr(e, "value") ?? "";
    case "time": if (attr(e, "datetime") !== undefined) return attr(e, "datetime")!;
  }
  return DomUtils.textContent(e);
}

/** "Get the object" (§ 5.2.6). */
function object(item: El, doc: Doc, order: Map<El, number>, base: string, memory: El[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const types = tokens(attr(item, "itemtype"));
  if (types.length) out.type = types;
  if (types.length && attr(item, "itemid") !== undefined) {
    const id = url(attr(item, "itemid"), base);
    if (id) out.id = id;
  }
  const props: Record<string, unknown[]> = {};
  for (const e of properties(item, doc, order)) {
    let v: unknown = value(e, base);
    if (typeof v === "object") {
      const it = (v as { item: El }).item;
      v = memory.includes(it) ? "ERROR" : object(it, doc, order, base, [...memory, item]);
    }
    for (const name of tokens(attr(e, "itemprop"))) (props[name] ??= []).push(v);
  }
  out.properties = props;
  return out;
}

function whatwg(html: string, base: string) {
  const doc = parseDocument(html, { decodeEntities: true, lowerCaseTags: true, lowerCaseAttributeNames: true });
  const order = treeOrder(doc);
  const tops = DomUtils.findAll((e) => attr(e, "itemscope") !== undefined && attr(e, "itemprop") === undefined, doc.children as Node[]);
  return { items: tops.sort((a, b) => order.get(a)! - order.get(b)!).map((t) => object(t, doc, order, base, [t])) };
}

const pick = <T>(next: () => number, xs: readonly T[]): T => xs[Math.floor(next() * xs.length)]!;

function randomMicrodata(next: () => number): string {
  const props = ["name", "url", "image", "startDate", "location", "offers", "price", "author", "a b", " name ", "x"];
  const ids = ["p1", "p2", "p3", "loop"];
  const types = ["https://schema.org/Event", "https://schema.org/Place", "https://schema.org/Person https://schema.org/Thing", "", "https://example.test/T"];
  const out: string[] = [];
  const el = (depth: number): string => {
    const kind = pick(next, ["item", "prop", "plain", "meta", "a", "img", "time", "data", "object", "text"]);
    const id = next() < 0.25 ? ` id="${pick(next, ids)}"` : "";
    const ip = next() < 0.6 ? ` itemprop="${pick(next, props)}"` : "";
    const kids = depth < 4 ? Array.from({ length: Math.floor(next() * 3) }, () => el(depth + 1)).join("") : "";
    switch (kind) {
      case "item": {
        const t = pick(next, types);
        const ref = next() < 0.3 ? ` itemref="${pick(next, ids)} ${pick(next, ids)}"` : "";
        const iid = next() < 0.2 ? ` itemid="${pick(next, ["/e/1", "urn:x:1", "https://v.example/e#1"])}"` : "";
        return `<div itemscope${t ? ` itemtype="${t}"` : ""}${iid}${ref}${ip}${id}>${kids}</div>`;
      }
      case "meta": return `<meta${ip}${id} content="${pick(next, ["25", "", "a &amp; b"])}">`;
      case "a": return `<a${ip}${id}${pick(next, [' href="/x"', ' href=""', "", ' href="https://o.example/p?q=1#f"'])}>link ${kids}</a>`;
      case "img": return `<img${ip}${id}${pick(next, [' src="i.jpg"', ' src=""', ""])}>`;
      case "time": return `<time${ip}${id}${pick(next, [' datetime="2026-11-14"', ""])}>za 14 nov</time>`;
      case "data": return `<data${ip}${id} value="${pick(next, ["7", ""])}">seven</data>`;
      case "object": return `<object${ip}${id}${pick(next, [' data="o.swf"', ""])}></object>`;
      case "text": return `text ${pick(next, ["&amp;", "<b>bold</b>", "  spaced  "])}`;
      default: return `<span${ip}${id}>${pick(next, ["Nora", "", " x "])}${kids}</span>`;
    }
  };
  for (let i = 0, n = 1 + Math.floor(next() * 4); i < n; i++) out.push(el(0));
  return `<html><body>${out.join("\n")}</body></html>`;
}

describe("microdata, held to the WHATWG standard", () => {
  it("gives the standard's JSON on random documents", () => {
    const next = random(52);
    const base = "https://v.example/agenda/";
    let items = 0;
    let props = 0;
    for (let i = 0; i < 2000; i++) {
      const html = randomMicrodata(next);
      const want = whatwg(html, base);
      const got = extractStructured(html, { url: base, raw: true, lenient: false, jsonLd: false, rdfa: false, microformats: false }).raw!.microdata;
      expect(got, html).toEqual(want);
      items += want.items.length;
      const count = (o: unknown): number =>
        o && typeof o === "object"
          ? Object.entries(o).reduce((n, [k, v]) => n + (k === "properties" ? Object.values(v as object).flat().length : 0) + count(v), 0)
          : 0;
      props += count(want);
    }
    // Not a comparison of empty lists.
    expect(items).toBeGreaterThan(250);
    expect(props).toBeGreaterThan(300);
  });
});
