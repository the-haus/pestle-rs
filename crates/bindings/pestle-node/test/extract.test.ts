import { Readable } from "node:stream";

import { describe, expect, it } from "vitest";

import { Extractor, Grammar, hasGrammars, skeleton, SpecError, strip, templateHash, type Item } from "../ts/index.js";

const PAGE = `<html><head><title>Agenda</title><base href="https://venue.example/nl/">
<script type="application/ld+json">{"@type":"MusicEvent","name":"Nora"}</script></head><body>
<article class="event"><h2><a href="e/nora#t">Nora</a></h2><p class="date">za 14 nov 2026 · 20:00</p><span class="price">€ 25,-</span></article>
<article class="event"><h2><a href="e/mira">Mira</a></h2><p class="date">zo 15 nov 2026 · 21:30</p></article>
</body></html>`;

const date = `
  WHITESPACE = _{ " " }
  when    = { weekday? ~ #day = day ~ #month = month ~ #year = year ~ "·" ~ #time = time }
  weekday = @{ ("za" | "zo") }
  day     = @{ ASCII_DIGIT{1,2} }
  month   = @{ ASCII_ALPHA+ }
  year    = @{ ASCII_DIGIT{4} }
  time    = @{ ASCII_DIGIT{2} ~ ":" ~ ASCII_DIGIT{2} }
`;

const spec = {
  title: "title",
  ld: { select: "script[type='application/ld+json']", text: "raw" as const, json: true },
  events: {
    select: "article.event",
    all: true,
    fields: {
      title: "h2",
      url: { select: "a", attr: "href", url: "http" as const },
      when: ".date",
      price: { select: ".price", number: true },
    },
  },
};

const want = {
  title: "Agenda",
  ld: { "@type": "MusicEvent", name: "Nora" },
  events: [
    { title: "Nora", url: "https://venue.example/nl/e/nora", when: "za 14 nov 2026 · 20:00", price: 25 },
    { title: "Mira", url: "https://venue.example/nl/e/mira", when: "zo 15 nov 2026 · 21:30", price: null },
  ],
};

describe("Extractor", () => {
  const x = new Extractor(spec);

  it("extracts strings and bytes, sync and on the pool", async () => {
    expect(x.extract(PAGE).data).toEqual(want);
    expect(x.extract(new TextEncoder().encode(PAGE)).data).toEqual(want);
    expect((await x.extractAsync(PAGE)).data).toEqual(want);
  });

  it("streams from every kind of source", async () => {
    const bytes = new TextEncoder().encode(PAGE);
    const pieces = (n: number) => Array.from({ length: Math.ceil(bytes.length / n) }, (_, i) => bytes.subarray(i * n, i * n + n));
    expect((await x.stream(pieces(7))).data).toEqual(want);
    expect((await x.stream(Readable.from(pieces(3)))).data).toEqual(want);
    expect((await x.stream(new Blob([bytes]).stream())).data).toEqual(want);
    expect((await x.stream(new Response(PAGE))).data).toEqual(want);
    expect((await x.stream(PAGE)).data).toEqual(want);
  });

  it("hands emitted items over as they complete", async () => {
    const e = new Extractor({ events: { select: "article", all: true, emit: true, fields: { t: "h2" } } });
    const seen: Item[] = [];
    const out = await e.stream(PAGE.split(/(?<=<\/article>)/), { onItem: (i) => seen.push(i) });
    expect(seen).toEqual([
      { field: "events", value: { t: "Nora" } },
      { field: "events", value: { t: "Mira" } },
    ]);
    expect(out.data).toEqual({});
    const s = e.session();
    s.write("<article><h2>A</h2>");
    expect(s.take()).toEqual([]);
    s.end();
    expect(s.take()).toEqual([{ field: "events", value: { t: "A" } }]);
  });

  it("matches a field's selector in the page, under its scope (P-6)", () => {
    const html = `<section class="card"><h3>Friday</h3></section>`;
    expect(new Extractor({ items: [{ select: ".card", fields: { t: ".card h3" } }] }).extract(html).data).toEqual({ items: [{ t: "Friday" }] });
    const nested = `<section><ul><li><b>A</b></li></ul></section><ul><li><b>B</b></li></ul>`;
    const x = new Extractor({ rows: [{ select: "li", fields: { s: "section b", own: ":scope b", up: ":scope section b" } }] });
    expect(x.extract(nested).data).toEqual({ rows: [{ s: "A", own: "A", up: null }, { s: null, own: "B", up: null }] });
  });

  it("reads the nearest value above a row (P-7), whole and streamed", async () => {
    const html = `<section data-day="fri"><ul><li class="set"><b>A</b></li></ul></section>`;
    const x = new Extractor({ rows: [{ select: "li.set", fields: { act: "b", day: { select: "section@data-day", closest: true } } }] });
    const want = { rows: [{ act: "A", day: "fri" }] };
    expect(x.extract(html).data).toEqual(want);
    expect((await x.stream(html.split(""))).data).toEqual(want);
    const s = x.session();
    for (const c of html.match(/.{1,3}/gs)!) s.write(c);
    expect(s.end().data).toEqual(want);
    const timetable = `<div><h3>Fri</h3><ul><li>A</li><li>B</li></ul></div><div><ul><li>C</li></ul><h3>Sat</h3></div>`;
    const t = new Extractor({ sets: { select: "li", all: true, emit: true, fields: { act: { text: "own" }, day: { select: "h3", closest: { depth: 2 } } } } });
    const seen: Item[] = [];
    await t.stream([timetable], { onItem: (i) => seen.push(i) });
    expect(seen.map((i) => i.value)).toEqual([
      { act: "A", day: "Fri" },
      { act: "B", day: "Fri" },
      { act: "C", day: "Sat" },
    ]);
    expect(() => new Extractor({ d: { select: "h3", closest: true } })).toThrow(/under a scope/);
  });

  it("reads text as lines (P-8), in a field and as the page's", async () => {
    const html = "<body><h1>Title</h1><p>One</p><ul><li>a</li><li>b</li></ul></body>";
    expect(new Extractor({ t: "body" }).extract(html).data).toEqual({ t: "Title One a b" });
    const lines = new Extractor({ t: { select: "body", text: "lines" } }, { page: { text: true } });
    const out = lines.extract(html);
    expect(out.data).toEqual({ t: "Title\nOne\na\nb" });
    expect(out.page!.text).toBe("Title\nOne\na\nb");
    const streamed = await lines.stream(html.split(""));
    expect(streamed.data).toEqual(out.data);
    expect(streamed.page!.text).toBe(out.page!.text);
    expect(new Extractor({}, { page: true }).extract(html).page!.text).toBeUndefined();
  });

  it("reports what does not compile, or leaves it out", () => {
    expect(() => new Extractor({ a: "li:hover" })).toThrow(SpecError);
    if (hasGrammars()) {
      expect(() => new Extractor({ a: { select: "li", parse: "nope" } })).toThrow(/no grammar named "nope"/);
      expect(() => new Extractor({}, { grammars: { bad: "a = {" } })).toThrow(/grammar "bad"/);
    } else {
      expect(() => new Extractor({ a: { select: "li", parse: "nope" } })).toThrow(/grammar` feature/);
      expect(() => new Extractor({}, { grammars: { bad: "a = {" } })).toThrow(/reads no grammars/);
    }
    const lenient = new Extractor({ a: "li:hover", b: "title" }, { lenient: true });
    expect(lenient.warnings).toHaveLength(1);
    expect(lenient.extract(PAGE).data).toEqual({ b: "Agenda" });
  });

  it.skipIf(!hasGrammars())("shapes values with pest grammars", () => {
    const g = new Extractor(
      { events: { select: "article.event", all: true, fields: { when: { select: ".date", parse: { grammar: "date", numbers: ["day", "year"] } } } } },
      { grammars: { date: new Grammar(date) } },
    );
    expect(g.extract(PAGE).data).toEqual({
      events: [{ when: { day: 14, month: "nov", year: 2026, time: "20:00" } }, { when: { day: 15, month: "nov", year: 2026, time: "21:30" } }],
    });
  });

  it("strips, fingerprints and hashes templates", () => {
    const s = strip(PAGE);
    expect(s.html).toContain('<article class="event"><h2><a href="e/nora#t">Nora</a></h2>');
    expect(s.tokens).toBeGreaterThan(10);
    const k = skeleton(PAGE, { paths: true });
    expect(k.paths).toContain("article.event>h2>a");
    expect(k.hash).toMatch(/^[0-9a-f]{64}$/);
    expect(templateHash([k.paths!, k.paths!, ["x"]])).toBe(skeleton(PAGE).hash);
    expect(templateHash([])).toBeUndefined();
  });
});
