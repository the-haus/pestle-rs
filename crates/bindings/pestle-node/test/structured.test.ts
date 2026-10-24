import { describe, expect, it } from "vitest";

import { extractStructured, extractStructuredAsync } from "../ts/index.js";

const PAGE = `<html><head><base href="https://v.example/nl/">
<meta property="og:title" content="Nora live">
<script type="application/ld+json">{"@context":"https://schema.org","@type":"MusicEvent","name":"Nora","url":"/e/nora",
 "startDate":"2026-11-14T20:00:00+01:00","location":{"@type":"Place","name":"Paradiso"},
 "offers":{"@type":"Offer","price":"25,00","priceCurrency":"EUR"},}</script></head><body>
<div itemscope itemtype="https://schema.org/MusicEvent"><a itemprop="url" href="/e/nora"><span itemprop="name">Nora</span></a>
 <time itemprop="startDate" datetime="2026-11-14">za 14 nov</time> <span itemprop="doorTime">19:00</span></div>
<div class="h-event"><span class="p-name">Mira</span> <time class="dt-start" datetime="2026-11-15 21:30">zo</time></div>
</body></html>`;

describe("structured data", () => {
  it("gives events from every format, one per event", () => {
    const s = extractStructured(PAGE, { url: "https://v.example/agenda" });
    expect(s.events.map((e) => e.name)).toEqual(["Nora", "Mira"]);
    const [nora, mira] = s.events;
    expect(nora).toMatchObject({
      startDate: "2026-11-14T20:00:00+01:00",
      doorTime: "19:00",
      url: "https://v.example/e/nora",
      format: "json-ld",
      formats: ["json-ld", "microdata"],
      offers: [{ price: "25,00", currency: "EUR" }],
    });
    expect(mira).toMatchObject({ startDate: "2026-11-15 21:30", format: "microformats" });
    expect(s.warnings.some((w) => w.includes("trailing"))).toBe(true);
    expect(s.items.some((i) => i["og:title"] === "Nora live")).toBe(true);
  });

  it("keeps them apart when asked, on the pool too", async () => {
    const s = await extractStructuredAsync(PAGE, { mergeEvents: false, microformats: false });
    expect(s.events.map((e) => e.name)).toEqual(["Nora", "Nora"]);
  });

  it("reads JSON-LD to the letter of JSON-LD 1.1 when not lenient", () => {
    const strict = extractStructured(PAGE, { url: "https://v.example/agenda", lenient: false, microdata: false, microformats: false });
    // The block's trailing comma makes it no JSON: left out, and said why.
    expect(strict.items.filter((i) => i["@format"] === "json-ld")).toEqual([]);
    expect(strict.warnings).toContain("json-ld: a block was left out: it is not JSON");
    const fixed = PAGE.replace(`"EUR"},}`, `"EUR"}}`);
    const ok = extractStructured(fixed, { url: "https://v.example/agenda", lenient: false, microdata: false, microformats: false });
    expect(ok.warnings).toEqual([]);
    expect(ok.items[0]).toMatchObject({ "@type": "MusicEvent", name: "Nora", url: "https://v.example/e/nora", location: { name: "Paradiso" } });
  });

  it("survives junk", () => {
    // Random fragments of every format's markup, glued at random: no throw, no crash.
    const bits = [
      "<script type=application/ld+json>", "</script>", "{", "}", "[", "]", '"@type":"Event"', '"@context":', '"name":"x"', ",", ":",
      "<div itemscope itemtype=https://schema.org/Event>", "<div itemprop=name>", "itemref=a", " id=a", "</div>",
      '<div vocab="https://schema.org/" typeof="Event">', '<span property="startDate" content="2026-01-01">', 'rel="x"', "about=[_:b]",
      '<div class="h-event h-card">', '<span class="p-name dt-start u-url e-content value">', '<img class="u-photo" src=x alt>', "<template>",
      "<base href=//x>", "<!--", "-->", "<![CDATA[", "\u0000", "&amp;", "&#x0;", "💥", "<svg>", "<math>", "<table><tr><td>", "<p>", ">",
    ];
    let seed = 7;
    const rnd = (n: number) => ((seed = (seed * 1103515245 + 12345) >>> 0) % n);
    for (let page = 0; page < 300; page++) {
      let html = "";
      for (let i = 0, n = 20 + rnd(400); i < n; i++) html += bits[rnd(bits.length)];
      const s = extractStructured(html, { url: "https://x.example/", raw: true });
      expect(Array.isArray(s.events)).toBe(true);
    }
  });
});

describe("raw JSON-LD", () => {
  it("keeps each block at its place on the page", () => {
    const page = [
      `{"@type":"Event","name":"A"}`,
      `{"@type":"Event","name":"B",}`,
      `totally broken`,
      `{"@type":"Event","name":"C"}`,
    ]
      .map((b) => `<script type="application/ld+json">${b}</script>`)
      .join("\n");
    const lenient = extractStructured(page, { raw: true }).raw!.jsonLd;
    expect(lenient).toEqual([{ "@type": "Event", name: "A" }, { "@type": "Event", name: "B" }, null, { "@type": "Event", name: "C" }]);
    const strict = extractStructured(page, { raw: true, lenient: false }).raw!.jsonLd;
    expect(strict).toEqual([{ "@type": "Event", name: "A" }, null, null, { "@type": "Event", name: "C" }]);
  });
});
