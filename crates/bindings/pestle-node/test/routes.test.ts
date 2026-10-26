import { describe, expect, it } from "vitest";

import { inferRoutes, readSitemap, RouteBuilder, type Route } from "../ts/index.js";

const site = "https://www.venue.example";

/** A venue's sitemap: events, news by date, pagination, pages. */
function sitemap(): string {
  const urls = ["/", "/agenda", "/contact", "/over-ons", "/nieuws"];
  for (const s of ["nora-live-in-paradiso", "jazz-night", "open-air-2026", "de-dijk", "kensington"]) urls.push(`/agenda/${s}`);
  for (let n = 2; n <= 4; n++) urls.push(`/agenda/page/${n}`);
  for (const [y, m, s] of [["2025", "11", "a-new-hall"], ["2026", "01", "season-two-opens"], ["2026", "02", "hello"]]) {
    urls.push(`/nieuws/${y}/${m}/${s}`);
  }
  const body = urls
    .map((u, k) => `<url><loc>${site}${u}</loc><lastmod>2026-10-${String(k + 1).padStart(2, "0")}</lastmod>${u.startsWith("/agenda/") && !u.includes("page") ? "<image:image><image:loc>https://cdn.example/i.jpg</image:loc></image:image>" : ""}</url>`)
    .join("");
  return `<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">${body}</urlset>`;
}

const flat = (r: Route): string[] => [r.pattern, ...(r.children ?? []).flatMap(flat)];

describe("routes", () => {
  it("infers a site's routes from its sitemap, with counts", () => {
    const routes = inferRoutes(readSitemap(sitemap()).entries);
    expect(routes.sites).toHaveLength(1);
    const [s] = routes.sites;
    expect(s!.origin).toBe(site);
    expect(s!.urls).toBe(16);
    expect(flat(s!.root).sort()).toEqual([
      "/",
      "/agenda",
      "/agenda/[slug]",
      "/agenda/page",
      "/agenda/page/[page]",
      "/contact",
      "/nieuws",
      "/nieuws/[year]",
      "/nieuws/[year]/[month]",
      "/nieuws/[year]/[month]/[slug]",
      "/over-ons",
    ]);
    const agenda = s!.root.children!.find((c) => c.segment === "agenda")!;
    expect(agenda).toMatchObject({ pages: 1, total: 9, kind: "listing", topic: "event" });
    const events = routes.find(`${site}/agenda/someone-new`)!;
    expect(events).toMatchObject({
      pattern: "/agenda/[slug]",
      param: { name: "slug", type: "slug" },
      pages: 5,
      total: 5,
      kind: "item",
      topic: "event",
      images: 5,
      lastmod: "2026-10-10",
    });
    expect(events.examples).toEqual([`${site}/agenda/nora-live-in-paradiso`, `${site}/agenda/jazz-night`, `${site}/agenda/open-air-2026`]);
    expect(routes.items().map((r) => r.pattern).sort()).toEqual(["/agenda/[slug]", "/nieuws/[year]/[month]/[slug]"]);
    expect(routes.find(`${site}/agenda/page/12`)?.kind).toBe("pagination");
    expect(routes.find("https://venue.example/contact")?.kind).toBe("page");
    expect(routes.findAll([`${site}/agenda/x-y-z`, "https://elsewhere.example/", "not a url"]).map((r) => r?.pattern)).toEqual([
      "/agenda/[slug]",
      undefined,
      undefined,
    ]);
  });

  it("prints the tree", () => {
    const outline = inferRoutes(readSitemap(sitemap()).entries).outline();
    expect(outline.split("\n").slice(0, 4)).toEqual([
      "https://www.venue.example (16 URLs)",
      expect.stringMatching(/^\s+pages\s+in all$/),
      expect.stringMatching(/^\/\s+1\s+16\s+root$/),
      expect.stringMatching(/^├── agenda\s+1\s+9\s+listing · event$/),
    ]);
    expect(outline).toMatch(/│ {3}├── \[slug\]\s+5\s+5\s+item · event · \d\.\d\d {2}e\.g\. \/agenda\/nora-live-in-paradiso/);
    expect(outline).toContain("page/[page]");
  });

  it("takes URLs one by one, in batches, in the order given", () => {
    const b = new RouteBuilder({ maxExamples: 2 });
    for (let k = 0; k < 10_000; k++) b.add(k % 2 ? `${site}/products/model-${k}-shoe` : { kind: "url", url: `${site}/blog/post-number-${k}` });
    b.add({ kind: "sitemap", url: `${site}/sitemap-2.xml` });
    b.add("mailto:someone@venue.example");
    const routes = b.finish();
    expect(routes.sites[0]!.urls).toBe(10_000);
    expect(routes.warnings).toEqual(["routes: 1 URLs that are not absolute http(s) were left out"]);
    expect(routes.find(`${site}/products/x-y-z`)).toMatchObject({ pattern: "/products/[slug]", pages: 5000, topic: "product" });
    expect(routes.find(`${site}/blog/x-y-z`)?.examples).toEqual([`${site}/blog/post-number-0`, `${site}/blog/post-number-2`]);
  });

  it("never throws on what it is given", () => {
    const routes = inferRoutes(["", "https://", "ftp://x.example/a", "https://x.example/" + "a/".repeat(5000), "https://x.example/%ZZ/é/🎸"]);
    expect(routes.sites).toHaveLength(1);
    expect(inferRoutes([]).sites).toEqual([]);
    expect(inferRoutes([]).outline()).toBe("");
  });
});
