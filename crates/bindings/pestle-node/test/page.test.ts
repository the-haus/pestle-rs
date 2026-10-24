import { describe, expect, it } from "vitest";

import { charsetOf, Extractor, extractPage, extractPageAsync, scrape } from "../ts/index.js";

const latin1 = (s: string) => Uint8Array.from([...s].map((c) => c.charCodeAt(0)));
const PAGE = `<html lang="nl"><head><title>Café — Paradiso</title>
<meta name="description" content="Concerten"><meta property="og:image" content="/og.jpg">
<link rel="canonical" href="/e/nora"><link rel="alternate" type="application/rss+xml" href="/feed">
<script id="__NEXT_DATA__" type="application/json">{"props":{"event":"Nora"}}</script>
<script type="application/ld+json">{"@context":"https://schema.org","@type":"Event","name":"Nora","startDate":"2026-11-14T20:00"}</script>
</head><body><h1>Nora</h1><a href="/e/nora">Nora live</a><img data-src="/n.jpg" src="data:x" alt="Nora">
<a href="mailto:info@paradiso.example">mail</a></body></html>`;

describe("page", () => {
  it("reads what a scraper wants", () => {
    const p = extractPage(PAGE, { url: "https://paradiso.example/agenda" });
    expect(p).toMatchObject({
      charset: "UTF-8",
      title: "Café — Paradiso",
      description: "Concerten",
      lang: "nl",
      canonical: "https://paradiso.example/e/nora",
      robots: { index: true, follow: true },
      feeds: [{ href: "https://paradiso.example/feed", type: "application/rss+xml" }],
      links: [{ href: "https://paradiso.example/e/nora", text: "Nora live" }],
      images: [{ src: "https://paradiso.example/n.jpg", alt: "Nora" }],
      headings: [{ level: 1, text: "Nora" }],
      emails: ["info@paradiso.example"],
      embedded: [{ name: "__NEXT_DATA__", value: { props: { event: "Nora" } } }],
    });
    expect(p.meta["og:image"]).toBe("/og.jpg");
  });

  it("reads bytes in their charset; strings as they are", async () => {
    // windows-1252 bytes: by the server's word, by the page's, by the bytes themselves.
    const bytes = latin1(PAGE.replace("—", "-"));
    for (const p of [
      extractPage(bytes, { charset: "iso-8859-1" }),
      extractPage(latin1(PAGE.replace("—", "-").replace("<head>", '<head><meta charset="windows-1252">'))),
      extractPage(bytes),
      await extractPageAsync(bytes),
    ]) {
      expect(p.title).toBe("Café - Paradiso");
      expect(p.charset).toBe("windows-1252");
    }
    // A string is text already: a meta that says otherwise changes nothing.
    const s = extractPage(PAGE.replace("<head>", '<head><meta charset="windows-1252">'));
    expect(s.title).toBe("Café — Paradiso");
    // A session fed bytes, or strings.
    const x = new Extractor({ t: "title" });
    const a = x.session({ charset: "windows-1252" });
    for (let i = 0; i < bytes.length; i += 7) a.write(bytes.subarray(i, i + 7));
    expect(a.end()).toMatchObject({ data: { t: "Café - Paradiso" }, charset: "windows-1252" });
    const b = x.session();
    b.write(PAGE.slice(0, 50)).write(PAGE.slice(50));
    expect(b.end().data).toEqual({ t: "Café — Paradiso" });
  });

  it("scrapes a fetch response in one pass", async () => {
    const res = new Response(latin1(PAGE.replace("—", "-")), { headers: { "content-type": "text/html; charset=ISO-8859-1" } });
    const { page, structured, charset } = await scrape(res, { url: "https://paradiso.example/" });
    expect(charset).toBe("windows-1252");
    expect(page.title).toBe("Café - Paradiso");
    expect(structured.events[0]).toMatchObject({ name: "Nora", startDate: "2026-11-14T20:00" });
    expect(structured.items[0]).toMatchObject({ "@context": "https://schema.org", "@type": "Event" });
    const fromString = await scrape(PAGE);
    expect(fromString.page.title).toBe("Café — Paradiso");
  });

  it("knows a bot wall", () => {
    expect(extractPage("<title>Just a moment...</title><div id=challenge-running></div>").blocked).toBe("cloudflare");
    expect(extractPage(PAGE).blocked).toBeUndefined();
  });

  it("reads charsets from headers", () => {
    expect(charsetOf("text/html; charset=ISO-8859-1")).toBe("ISO-8859-1");
    expect(charsetOf('text/html;charset="utf-8"')).toBe("utf-8");
    expect(charsetOf("text/html")).toBeUndefined();
    expect(charsetOf(null)).toBeUndefined();
  });
});
