/**
 * The recipes of the README's cookbook, run against a local site: if one of
 * them stops working, this fails before the README misleads anyone.
 */
import assert from "node:assert/strict";
import { createReadStream, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { it } from "vitest";

import {
  ariaSnapshot,
  charsetOf,
  extractPage,
  extractStructured,
  Extractor,
  parseRobots,
  readSitemap,
  scrape,
  SpecError,
  strip,
} from "../ts/index.js";

const P = join(__dirname, "..") + "/";

it("the README cookbook", async () => {
  const tmp = mkdtempSync(join(tmpdir(), "pestle-cookbook-"));
  const agenda = readFileSync(P + "examples/agenda.html");
  const latin1 = Buffer.from('<meta charset="windows-1252"><title>Caf\xe9 Ren\xe9</title>', "latin1");
  const pages: Record<string, [string, string | Buffer]> = {
    "/agenda": ["text/html; charset=utf-8", agenda],
    "/agenda?page=2": ["text/html", '<title>Agenda 2</title><link rel=prev href="/agenda"><article class=event><h2>Ada</h2></article>'],
    "/latin1": ["text/html; charset=windows-1252", latin1],
    "/next": ["text/html", '<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"events":[{"id":1,"name":"Nora"}]}}}</script>'],
    "/product": ["text/html", `<script type="application/ld+json">{"@context":"https://schema.org","@type":"Product","name":"Tee","sku":"T-1","offers":{"@type":"Offer","price":"19.95","priceCurrency":"EUR","availability":"https://schema.org/InStock"}}</script>`],
    "/wall": ["text/html", '<title>Just a moment...</title><div id="cf-wrapper"><div class="cf-browser-verification">Checking your browser</div></div><script src="/cdn-cgi/challenge-platform/h/b/orchestrate/jsch/v1"></script>'],
  };
  const server = createServer((req, res) => {
    const p = pages[req.url ?? ""];
    if (!p) return res.writeHead(404).end();
    res.writeHead(200, { "content-type": p[0] }).end(p[1]);
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const site = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  
  const spec = { events: { select: "article.event", all: true, fields: { title: "h2" } } };
  const x = new Extractor(spec);
  
  // --- HTTP clients ---
  {
    // fetch: URL and charset come from the response
    const { data } = await x.stream(await fetch(`${site}/agenda`));
    assert.equal(data.events[0].title, "Nora & Co");
    const { page } = await scrape(await fetch(`${site}/latin1`));
    assert.equal(page.title, "Café René");
  }
  {
    // axios-style: a Buffer of bytes
    const url = `${site}/latin1`;
    const res = await fetch(url);
    const data = Buffer.from(await res.arrayBuffer()); // what axios gives with responseType: "arraybuffer"
    const { title } = extractPage(data, { url, charset: charsetOf(res.headers.get("content-type")) });
    assert.equal(title, "Café René");
  }
  {
    // a file on disk, streamed
    writeFileSync(join(tmp, "agenda.html"), agenda);
    const { data } = await x.stream(createReadStream(join(tmp, "agenda.html")), { url: "https://venue.example/agenda" });
    assert.equal(data.events.length, 2);
  }
  
  // --- Many pages at once ---
  {
    const urls = [`${site}/agenda`, `${site}/agenda?page=2`, `${site}/agenda`];
    const results = await Promise.all(
      urls.map(async (url) => {
        const res = await fetch(url);
        return x.extractAsync(new Uint8Array(await res.arrayBuffer()), { url, charset: charsetOf(res.headers.get("content-type")) });
      }),
    );
    assert.deepEqual(results.map((r) => r.data.events.length), [2, 1, 2]);
  }
  
  // --- Pagination ---
  {
    const all: unknown[] = [];
    let url: string | undefined = `${site}/agenda`;
    const withPage = new Extractor(spec, { page: { links: false, images: false } });
    // agenda.html has no next link; page 2 has prev. Use page 2 to check next/prev reading.
    const r2 = await withPage.stream(await fetch(`${site}/agenda?page=2`));
    assert.equal(r2.page.prev, `${site}/agenda`);
    for (let n = 0; url && n < 50; n++) {
      const { data, page } = await withPage.stream(await fetch(url));
      all.push(...data.events);
      url = page.next;
    }
    assert.equal(all.length, 2);
  }
  
  // --- Next.js data ---
  {
    const { embedded } = await extractPage(await (await fetch(`${site}/next`)).text(), { url: `${site}/next` });
    const next = embedded?.find((e) => e.name === "__NEXT_DATA__")?.value;
    assert.equal(next.props.pageProps.events[0].name, "Nora");
  }
  
  // --- Product page ---
  {
    const { items } = extractStructured(await (await fetch(`${site}/product`)).text(), { url: `${site}/product`, types: ["Product"] });
    const product = items[0];
    assert.equal(product.name, "Tee");
    assert.equal(product.offers.price, "19.95");
    assert.equal(product.offers.availability, "InStock");
  }
  
  // --- Bot walls ---
  {
    const { page } = await scrape(await fetch(`${site}/wall`));
    assert.equal(page.blocked, "cloudflare");
  }
  
  // --- Specs from a model ---
  {
    const fromModel = { title: "h1", price: { select: ".price", number: true }, bad: "article:has(.x) h2", worse: "h1[" };
    const lenient = new Extractor(fromModel, { lenient: true });
    assert.equal(lenient.warnings.length, 2);
    const { data } = lenient.extract(agenda);
    assert.equal(data.title, "Agenda");
    assert.ok(!("bad" in data));
    try {
      new Extractor(fromModel);
      assert.fail();
    } catch (e) {
      assert.ok(e instanceof SpecError);
      assert.equal(e.errors.length, 2);
    }
  }
  
  // --- Why is my field null? ---
  {
    const { data, errors, stats } = new Extractor({ ld: { select: "script[type='application/ld+json']", text: "raw", json: true }, nothing: ".nope" }).extract(
      '<script type="application/ld+json">{ broken</script>',
    );
    assert.equal(data.nothing, null);
    assert.equal(data.ld, null);
    assert.equal(errors.length, 1);
    assert.equal(typeof stats.matches, "number");
  }
  
  // --- Agents: refs to Playwright selectors ---
  {
    const { refs, snapshot } = ariaSnapshot(agenda, { mode: "ai" });
    const ref = /button "Zoek" \[ref=(e\d+)\]/.exec(snapshot)[1];
    assert.equal(refs[ref], "body > main:nth-child(2) > form:nth-child(2) > button:nth-child(2)");
  }
  
  // --- Typed results ---
  // (checked by tsc in the smoke test)
  
  // --- Timeouts ---
  {
    const slow = createServer((req, res) => {
      res.writeHead(200, { "content-type": "text/html" });
      res.write("<article class=event><h2>first</h2></article>");
    });
    await new Promise<void>((r) => slow.listen(0, "127.0.0.1", r));
    const url = `http://127.0.0.1:${(slow.address() as AddressInfo).port}/`;
    const signal = AbortSignal.timeout(300);
    await assert.rejects(x.stream(await fetch(url, { signal }), { signal }), (e: Error) => e.name === "TimeoutError" || e.name === "AbortError");
    slow.closeAllConnections();
    slow.close();
  }
  
  // strip budget
  {
    const { tokens, truncated } = strip(agenda, { maxChars: 24_000 });
    assert.ok(tokens > 0 && !truncated);
  }
  // readSitemap / robots basics
  assert.equal(readSitemap("<urlset><url><loc>https://a.example/</loc></url></urlset>").entries.length, 1);
  assert.equal(parseRobots("User-agent: *\nDisallow: /", "bot").allows("/x"), false);
  
  
  server.close();
  rmSync(tmp, { recursive: true, force: true });
});
