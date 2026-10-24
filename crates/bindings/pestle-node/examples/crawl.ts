// The crawler's half: read a site's robots.txt for your product token, follow
// its sitemaps (indexes, gzipped or not, feeds too) as they stream in, and
// print every page the site lets you fetch, one JSON line each.
//
//   npx tsx crawl.ts https://venue.example [mybot] [max-pages]

import { NotASitemap, parseRobots, parseSitemap, Robots } from "@pestle-rs/node";

const [site, token = "pestle-example", max = "1000"] = process.argv.slice(2);
if (!site) throw new Error("usage: crawl.ts <site> [token] [max-pages]");
const limit = Number(max);
const headers = { "user-agent": `${token}/1.0 (+https://github.com/the-haus/pestle-rs)` };

// RFC 9309: a 4xx means no rules, a 5xx (or no answer) means stay out.
let robots: Robots;
try {
  const res = await fetch(new URL("/robots.txt", site), { headers });
  robots = res.ok ? parseRobots(await res.text(), token) : res.status < 500 ? Robots.allowAll() : Robots.disallowAll();
} catch {
  robots = Robots.disallowAll();
}
if (robots.crawlDelay) console.error(`crawl-delay: ${robots.crawlDelay}s`);

const queue: string[] = robots.sitemaps.length ? [...robots.sitemaps] : [new URL("/sitemap.xml", site).href];
const seen = new Set(queue);
let pages = 0;

while (queue.length && pages < limit) {
  const url = queue.shift()!;
  const res = await fetch(url, { headers });
  if (!res.ok) {
    console.error(`${url}: HTTP ${res.status}`);
    continue;
  }
  try {
    for await (const entry of parseSitemap(res)) {
      if (entry.kind === "sitemap") {
        if (!seen.has(entry.url)) {
          seen.add(entry.url);
          queue.push(entry.url);
        }
      } else if (robots.allows(entry.url)) {
        console.log(JSON.stringify({ url: entry.url, lastmod: entry.lastmod ?? null }));
        if (++pages >= limit) break;
      }
    }
  } catch (e) {
    if (!(e instanceof NotASitemap)) throw e;
    console.error(`${url}: not a sitemap (${e.message})`);
  }
}
console.error(`${pages} pages`);
