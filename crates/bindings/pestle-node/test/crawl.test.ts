import { gzipSync } from "node:zlib";

import { describe, expect, it } from "vitest";

import { NotASitemap, parseRobots, parseSitemap, readSitemap, readSitemapAsync, Robots } from "../ts/index.js";
import * as festimap from "./reference-crawl.js";
import { random } from "./reference.js";

const pick = <T>(next: () => number, xs: readonly T[]): T => xs[Math.floor(next() * xs.length)]!;

describe("robots.txt", () => {
  it("decides as festimap does, on random files and paths", () => {
    const next = random(9309);
    // Not `festimap`: festimap's robots.ts takes it as naming `festimapbot` (a prefix); RFC 9309
    // matches whole product tokens (tested below).
    const agents = ["*", "festimapbot", "FestimapBot/2.1", "googlebot", "otherbot"];
    const pieces = ["/a", "/a/b", "/*.pdf$", "/*?", "/b*c", "/%7Euser", "/~user", "/caf%C3%A9", "/x$", "/", "/a/*/c", "*", "/A"];
    // Not `/robots.txt` (always allowed by RFC 9309 § 2.2.2) nor URLs with a literal `*` or `$`.
    const paths = ["/", "/a", "/a/b/c", "/doc.pdf", "/doc.pdf?x=1", "/bxc", "/bc", "/~user/p", "/%7euser", "/café", "/x", "/xy", "/a/q/c", "/A", "/?q"];
    let compared = 0;
    let disallowed = 0;
    for (let file = 0; file < 400; file++) {
      const lines: string[] = [];
      for (let g = 0, groups = 1 + Math.floor(next() * 4); g < groups; g++) {
        for (let a = 0, n = 1 + Math.floor(next() * 2); a < n; a++) lines.push(`User-agent: ${pick(next, agents)}`);
        for (let r = 0, n = Math.floor(next() * 5); r < n; r++) {
          const kind = pick(next, ["Allow", "Disallow", "Disallow", "Crawl-delay"]);
          lines.push(kind === "Crawl-delay" ? `Crawl-delay: ${Math.floor(next() * 10)}` : `${kind}: ${next() < 0.1 ? "" : pick(next, pieces)}`);
        }
        if (next() < 0.3) lines.push(`Sitemap: https://v.example/s${Math.floor(next() * 3)}.xml`);
        lines.push(pick(next, ["", "# comment", ""]));
      }
      const text = lines.join(pick(next, ["\n", "\r\n"]));
      const ours = parseRobots(text, "festimapbot");
      const theirs = festimap.parseRobots(text, "festimapbot");
      expect(ours.crawlDelay, text).toBe(theirs.crawlDelay);
      expect([...ours.sitemaps], text).toEqual(theirs.sitemaps ?? []);
      for (const p of paths) {
        const url = `https://v.example${p}`;
        const allowed = festimap.robotsAllow(theirs, url);
        expect(ours.allows(url), `${text}\n--- ${url}`).toBe(allowed);
        compared++;
        if (!allowed) disallowed++;
      }
    }
    expect(compared).toBe(400 * 15);
    // Not a comparison of two "allowed"s: a good share is disallowed.
    expect(disallowed).toBeGreaterThan(1000);
  });

  it("follows RFC 9309 where festimap's robots.ts does not", () => {
    const txt = "User-agent: festimap\nDisallow: /\n";
    expect(parseRobots(txt, "festimapbot").allows("/x")).toBe(true);
    expect(festimap.robotsAllow(festimap.parseRobots(txt, "festimapbot"), "https://v.example/x")).toBe(false);
    const all = "User-agent: *\nDisallow: /\n";
    expect(parseRobots(all, "festimapbot").allows("https://v.example/robots.txt")).toBe(true);
    expect(parseRobots("User-agent: *\nDisallow: /a%2A\n", "x").allows("https://v.example/a*")).toBe(false);
  });

  it("reads what festimap's misses", () => {
    const r = parseRobots("User agent: *\nDissallow: /private\n", "festimapbot");
    expect(r.allows("/private/x")).toBe(false);
    expect(festimap.robotsAllow(festimap.parseRobots("User agent: *\nDissallow: /private\n"), "https://v.example/private/x")).toBe(true);
    expect(Robots.disallowAll().allows("/x")).toBe(false);
    expect(Robots.allowAll().allowsAll(["/a", "/b"])).toEqual([true, true]);
  });
});

async function* bytesOf(body: Uint8Array, size: number): AsyncGenerator<Uint8Array> {
  for (let i = 0; i < body.length; i += size) yield body.subarray(i, i + size);
}

async function collect<T>(it: AsyncIterable<T>): Promise<T[]> {
  const out: T[] = [];
  for await (const x of it) out.push(x);
  return out;
}

const plain = (e: { kind: string; url: string; lastmod?: string; changefreq?: string; priority?: number }) => ({
  kind: e.kind,
  url: e.url,
  ...(e.lastmod ? { lastmod: e.lastmod } : {}),
  ...(e.changefreq ? { changefreq: e.changefreq } : {}),
  ...(e.priority !== undefined ? { priority: e.priority } : {}),
});

describe("sitemaps", () => {
  it("reads as festimap does, on random sitemaps", async () => {
    const next = random(50000);
    const enc = new TextEncoder();
    let entries = 0;
    for (let file = 0; file < 150; file++) {
      const prefix = pick(next, ["", "", "sm:", "ns2:"]);
      const index = next() < 0.25;
      const tag = index ? "sitemap" : "url";
      const parts = [`<?xml version="1.0" encoding="UTF-8"?>\n<${prefix}${index ? "sitemapindex" : "urlset"} xmlns${prefix ? ":" + prefix.slice(0, -1) : ""}="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">`];
      for (let i = 0, n = Math.floor(next() * 40); i < n; i++) {
        const loc = pick(next, [`https://v.example/e/${i}`, `https://v.example/e/${i}?a=1&amp;b=2`, `<![CDATA[https://v.example/c/${i}?x=1&y=2]]>`, `http://v.example/h/${i}#frag`, `/relative/${i}`, `mailto:x@y.z`, ` https://v.example/sp/${i} `]);
        const fields = [`<${prefix}loc>${loc}</${prefix}loc>`];
        if (next() < 0.5) fields.push(`<${prefix}lastmod>${pick(next, ["2026-11-01", "2026-11-01T10:00:00+01:00", "not a date", "2026"])}</${prefix}lastmod>`);
        if (!index && next() < 0.4) fields.push(`<${prefix}changefreq>${pick(next, ["daily", "Weekly", "sometimes"])}</${prefix}changefreq>`);
        if (!index && next() < 0.4) fields.push(`<${prefix}priority>${pick(next, ["0.5", "1.0", "7", "x"])}</${prefix}priority>`);
        if (!index && next() < 0.3) fields.unshift(`<image:image><image:loc>https://cdn.example/${i}.jpg</image:loc></image:image>`);
        parts.push(`<${prefix}${tag}>${fields.join(pick(next, ["", "\n  "]))}</${prefix}${tag}>`);
      }
      parts.push(`</${prefix}${index ? "sitemapindex" : "urlset"}>`);
      const body = enc.encode(parts.join("\n"));
      const theirs = await collect(festimap.parseSitemap(bytesOf(body, 997)));
      entries += theirs.length;
      for (const size of [1, 97, 1 << 20]) {
        const ours = await collect(parseSitemap(bytesOf(body, size)));
        expect(ours.map(plain), `size ${size}\n${parts.join("\n")}`).toEqual(theirs.map(plain));
      }
      // Gzipped, the same; on the pool, the same.
      expect(readSitemap(gzipSync(body)).entries.map(plain)).toEqual(theirs.map(plain));
      expect((await readSitemapAsync(body)).entries.map(plain)).toEqual(theirs.map(plain));
    }
    expect(entries).toBeGreaterThan(1500);
  });

  it("knows what is no sitemap", async () => {
    for (const body of ["<!DOCTYPE html><html><body>Not found</body></html>", "", "<note/>"]) {
      await expect(collect(parseSitemap(body))).rejects.toThrow(NotASitemap);
      expect(() => readSitemap(body)).toThrow(NotASitemap);
      await expect(readSitemapAsync(body)).rejects.toThrow(NotASitemap);
    }
  });

  it("reads feeds, text sitemaps and relative locs", async () => {
    const rss = readSitemap(`<rss version="2.0"><channel><item><title>Nora</title><link>https://v.example/e/nora</link><pubDate>Sat, 14 Nov 2026 20:00:00 +0100</pubDate></item></channel></rss>`);
    expect(rss).toMatchObject({ format: "rss", entries: [{ kind: "url", url: "https://v.example/e/nora", title: "Nora" }] });
    expect(readSitemap("https://v.example/a\nhttps://v.example/b\n").entries.map((e) => e.url)).toEqual(["https://v.example/a", "https://v.example/b"]);
    const rel = readSitemap("<urlset><url><loc>/e/1</loc></url></urlset>", { url: "https://v.example/sitemap.xml" });
    expect(rel.entries[0]!.url).toBe("https://v.example/e/1");
    const res = new Response(gzipSync(new TextEncoder().encode("<sitemapindex><sitemap><loc>https://v.example/s1.xml</loc></sitemap></sitemapindex>")));
    expect(await collect(parseSitemap(res))).toEqual([{ kind: "sitemap", url: "https://v.example/s1.xml" }]);
  });
});
