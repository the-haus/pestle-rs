// A site's routes, inferred from its URLs: the tree of patterns its pages follow, as Next.js
// writes them (`/agenda/[slug]`), how many URLs each holds, and which hold one item a page.

import type { SitemapEntry } from "./crawl.js";
import * as native from "./native.js";
import type { NativeRouteBuilder, NativeRoutes } from "./native.js";

/** What a parameter's values are. */
export type RouteParamType = "slug" | "number" | "year" | "month" | "day" | "date" | "uuid" | "hash" | "locale";

/**
 * What a route's pages most likely are: `item` (one event, product or article a page), `listing`
 * (a section above items), `taxonomy` (a page a tag or category), `archive` (a page a year or
 * month), `pagination`, `page` (one page), `section`, or the site's `root`.
 */
export type RouteKind = "root" | "page" | "section" | "listing" | "item" | "taxonomy" | "archive" | "pagination";

/** One route of a site. */
export interface Route {
  /** As written in the path (`agenda`), or the parameter in brackets (`[slug]`). */
  segment: string;
  /** For a dynamic segment: its name (unique along the route) and what its values are. */
  param?: { name: string; type: RouteParamType };
  /** From the site's root: `/agenda/[slug]`. */
  pattern: string;
  /** URLs that are this route. */
  pages: number;
  /** URLs at this route or below it. */
  total: number;
  kind: RouteKind;
  /** What its pages are about, from the words of its path. */
  topic?: "event" | "product" | "article" | "person" | "venue" | "job" | "recipe";
  /** For an item route, how sure it is that each page is one item, 0–1; 0 for other routes. */
  score: number;
  /** The first URLs given that are this route. */
  examples?: string[];
  /** The latest `<lastmod>` of its pages (sitemap entries only). */
  lastmod?: string;
  /** How many of its pages have images (sitemap entries only). */
  images?: number;
  /** Most URLs first. */
  children?: Route[];
}

/** The routes of one site (a host, `www.` or not). */
export interface RouteSite {
  /** `https://venue.example`, as its first URL has it. */
  origin: string;
  /** The URLs read for it. */
  urls: number;
  root: Route;
}

export interface RouteOptions {
  /** Siblings that are short slugs (`jazz-night`) become a parameter from this many (5); single words from four times as many. */
  minSiblings?: number;
  /** Example URLs kept for each route (3). */
  maxExamples?: number;
  /** At most this many URLs are read (1,000,000). */
  maxUrls?: number;
}

/** Every site's routes. */
export class Routes {
  /** Most URLs first. */
  readonly sites: readonly RouteSite[];
  /** What was left out (URLs that are not absolute http(s), past `maxUrls`). */
  readonly warnings: readonly string[];
  readonly #native: NativeRoutes;
  readonly #byPattern = new Map<string, Route>();

  /** @internal */
  constructor(n: NativeRoutes) {
    this.#native = n;
    const j = JSON.parse(n.json()) as { sites: RouteSite[]; warnings?: string[] };
    this.sites = j.sites;
    this.warnings = j.warnings ?? [];
    const index = (origin: string, r: Route) => {
      this.#byPattern.set(`${origin} ${r.pattern}`, r);
      for (const c of r.children ?? []) index(origin, c);
    };
    for (const s of this.sites) index(s.origin, s.root);
  }

  /** The route `url` is: static segments first, then a parameter its value fits. */
  find(url: string): Route | undefined {
    return this.findAll([url])[0];
  }

  /** {@link find} for many URLs in one call: which of a page's links are items, say. */
  findAll(urls: readonly string[]): (Route | undefined)[] {
    return this.#native.findAll([...urls]).map((m) => (m ? this.#byPattern.get(`${m[0]} ${m[1]}`) : undefined));
  }

  /** Every route of items, the likeliest first: where a site's events, products or articles are. */
  items(): Route[] {
    const out: Route[] = [];
    const walk = (r: Route) => {
      if (r.kind === "item") out.push(r);
      for (const c of r.children ?? []) walk(c);
    };
    for (const s of this.sites) walk(s.root);
    return out.sort((a, b) => b.score - a.score || b.pages - a.pages);
  }

  /** The tree as text, a line a route: its pages, the URLs at or below it, kind, topic and score. */
  outline(): string {
    return this.#native.outline();
  }
}

const BATCH = 4096;

/**
 * URLs, or sitemap entries, added one by one (from a stream, say); routes inferred at
 * {@link finish}. Entries that name another sitemap are left out.
 *
 * ```ts
 * const b = new RouteBuilder();
 * for await (const e of parseSitemap(await fetch(url))) b.add(e);
 * console.log(b.finish().outline());
 * ```
 */
export class RouteBuilder {
  readonly #native: NativeRouteBuilder;
  #urls: string[] = [];
  #entries: { url: string; lastmod?: string; images?: boolean }[] = [];

  constructor(options: RouteOptions = {}) {
    const o: { minSiblings?: number; maxExamples?: number; maxUrls?: number } = {};
    if (options.minSiblings !== undefined) o.minSiblings = options.minSiblings;
    if (options.maxExamples !== undefined) o.maxExamples = options.maxExamples;
    if (options.maxUrls !== undefined) o.maxUrls = options.maxUrls;
    this.#native = new native.NativeRouteBuilder(o);
  }

  add(entry: string | SitemapEntry): this {
    // One kind is pending at a time, so URLs go over in the order given (examples are the first).
    if (typeof entry === "string") {
      if (this.#entries.length) this.#flush();
      this.#urls.push(entry);
      if (this.#urls.length >= BATCH) this.#flush();
    } else if (entry.kind !== "sitemap") {
      if (this.#urls.length) this.#flush();
      const e: { url: string; lastmod?: string; images?: boolean } = { url: entry.url };
      if (entry.lastmod !== undefined) e.lastmod = entry.lastmod;
      if (entry.images?.length) e.images = true;
      this.#entries.push(e);
      if (this.#entries.length >= BATCH) this.#flush();
    }
    return this;
  }

  /** The routes of every site the URLs were on. */
  finish(): Routes {
    this.#flush();
    return new Routes(this.#native.finish());
  }

  #flush(): void {
    if (this.#urls.length) {
      this.#native.add(this.#urls);
      this.#urls = [];
    }
    if (this.#entries.length) {
      this.#native.addEntries(this.#entries);
      this.#entries = [];
    }
  }
}

/**
 * A site's routes from its URLs (a sitemap's entries, the links of its pages): a tree of patterns
 * as Next.js writes them, with how many URLs each holds and which hold one item a page.
 *
 * ```ts
 * const routes = inferRoutes(readSitemap(body, { url }).entries);
 * routes.outline();             // the tree, a line a route
 * routes.items()[0]?.pattern;   // "/agenda/[slug]"
 * routes.find(link)?.kind;      // "item"
 * ```
 */
export function inferRoutes(urls: Iterable<string | SitemapEntry>, options: RouteOptions = {}): Routes {
  const b = new RouteBuilder(options);
  for (const u of urls) b.add(u);
  return b.finish();
}
