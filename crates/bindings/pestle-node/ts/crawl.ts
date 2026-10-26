// robots.txt (RFC 9309), sitemaps (sitemaps.org) and the Public Suffix List,
// read natively: what a crawler asks before and while it reads a site's pages.

import { chunks, type Source } from "./extract.js";
import * as native from "./native.js";
import type { NativeRobots, NativeSitemapReader } from "./native.js";

/** One rule of a robots.txt group. */
export interface RobotsRule {
  allow: boolean;
  /** Its path pattern, percent-encoding normalised. */
  pattern: string;
}

/**
 * A robots.txt read for one product token: the groups that name it (else `*`), merged; the
 * longest matching rule wins, `Allow` on a tie; `*` and `$` as RFC 9309 has them; misspelled
 * fields (`Dissallow`, `User agent`) read as Google reads them.
 */
export class Robots {
  /** @internal */
  readonly native: NativeRobots;
  readonly rules: readonly RobotsRule[];
  /** Seconds, when a chosen group states one (the largest). */
  readonly crawlDelay?: number;
  /** Every `Sitemap:` URL of the file (absolute http(s), in order, once each). */
  readonly sitemaps: readonly string[];

  /** @internal */
  constructor(n: NativeRobots) {
    this.native = n;
    const j = n.json as { rules: RobotsRule[]; crawlDelay?: number; sitemaps?: string[] };
    this.rules = j.rules;
    if (j.crawlDelay !== undefined) this.crawlDelay = j.crawlDelay;
    this.sitemaps = j.sitemaps ?? [];
  }

  /** Whether `url` (a URL, or a path with its query) may be fetched. */
  allows(url: string): boolean {
    return this.native.allows(url);
  }

  /** {@link allows} for many URLs in one call. */
  allowsAll(urls: readonly string[]): boolean[] {
    return this.native.allowsAll([...urls]);
  }

  /** Allows everything: a robots.txt that is absent (4xx). */
  static allowAll(): Robots {
    return new Robots(native.NativeRobots.allowAll());
  }

  /** Allows nothing: a robots.txt that could not be read (5xx, no answer). */
  static disallowAll(): Robots {
    return new Robots(native.NativeRobots.disallowAll());
  }
}

/**
 * A host's registrable domain, by the bundled Public Suffix List (ICANN and private sections, of
 * {@link PUBLIC_SUFFIX_LIST_VERSION}): `example.com.mx` of `tickets.example.com.mx`,
 * `foo.github.io` of itself. An IDN comes back as its punycode; a host that is a public suffix
 * (`co.uk`), or an IP address, is its own; `null` for what is no host (`.com`, `a..b`).
 */
export function registrableDomain(host: string): string | null {
  return native.registrableDomain(host);
}

/** A host's public suffix (`co.uk` of `www.example.co.uk`, `github.io` of `github.io`); `null` for an IP address or no host. */
export function publicSuffix(host: string): string | null {
  return native.publicSuffix(host);
}

/** The date of the bundled Public Suffix List (`2026-10-22_08-36-41_UTC`). */
export const PUBLIC_SUFFIX_LIST_VERSION: string = native.publicSuffixListVersion();

/** A robots.txt's rules for `token` (your crawler's product token, `festimapbot`). */
export function parseRobots(text: string, token: string): Robots {
  return new Robots(new native.NativeRobots(text, token));
}

/** One entry of a sitemap: a page (`kind: "url"`, a feed's item too), or a sitemap an index names. */
export interface SitemapEntry {
  kind: "url" | "sitemap";
  /** Absolute http(s), without `#…`. */
  url: string;
  /** As written, when it is a date (W3C, or RFC 822 in feeds). */
  lastmod?: string;
  changefreq?: "always" | "hourly" | "daily" | "weekly" | "monthly" | "yearly" | "never";
  /** 0–1. */
  priority?: number;
  /** `<image:loc>`s. */
  images?: string[];
  /** `<xhtml:link rel="alternate" hreflang>`s. */
  alternates?: { hreflang: string; href: string }[];
  /** A feed item's or a news entry's title. */
  title?: string;
}

/** A sitemap read whole. */
export interface Sitemap {
  format: "urlset" | "index" | "text" | "rss" | "atom";
  entries: SitemapEntry[];
  gzip: boolean;
  /** What was cut or skipped. */
  warnings?: string[];
}

export interface SitemapOptions {
  /** The sitemap's own URL: relative `<loc>`s resolve against it. */
  url?: string | undefined;
  /** The most the body may be once gunzipped (256 MiB): a gzip bomb stops there. */
  maxBytes?: number;
  /** At most this many entries. */
  maxEntries?: number;
  /** An entry longer than this without its end is skipped (64 KiB). */
  maxEntryBytes?: number;
}

/** A body that is no sitemap (an HTML page, a soft 404, an empty body). */
export class NotASitemap extends Error {
  override name = "NotASitemap";
}

function reader(options: SitemapOptions): NativeSitemapReader {
  const o: { maxBytes?: number; maxEntries?: number; maxEntryBytes?: number } = {};
  if (options.maxBytes !== undefined) o.maxBytes = options.maxBytes;
  if (options.maxEntries !== undefined) o.maxEntries = options.maxEntries;
  if (options.maxEntryBytes !== undefined) o.maxEntryBytes = options.maxEntryBytes;
  return new native.NativeSitemapReader(options.url ?? null, o);
}

function sitemapError(e: unknown): never {
  const message = e instanceof Error ? e.message : String(e);
  if (message.startsWith("SITEMAP:")) throw new NotASitemap(JSON.parse(message.slice(8)) as string);
  throw e;
}

/**
 * A sitemap's entries as its bytes arrive: XML (`<urlset>`, `<sitemapindex>`, any prefix), text
 * (a URL a line), or a feed (RSS, Atom), gzipped or not (by its magic bytes), in its charset.
 * Only the entry in progress is held. Throws {@link NotASitemap} for a body that is none.
 *
 * ```ts
 * for await (const e of parseSitemap(await fetch(url), { url })) if (e.kind === "sitemap") queue(e.url);
 * ```
 */
export async function* parseSitemap(source: Source, options: SitemapOptions = {}): AsyncGenerator<SitemapEntry> {
  if (typeof Response !== "undefined" && source instanceof Response && options.url === undefined && source.url) {
    options = { ...options, url: source.url };
  }
  const r = reader(options);
  try {
    // Entries cross over as JSON: one string per chunk, which V8 reads faster than objects built one by one.
    for await (const chunk of chunks(source)) {
      const json = r.writeJson(chunk);
      if (json !== "[]") yield* JSON.parse(json) as SitemapEntry[];
    }
    yield* (JSON.parse(r.endJson()) as Sitemap).entries;
  } catch (e) {
    sitemapError(e);
  }
}

/** A whole sitemap at once (bytes, gzipped or not, or text). Throws {@link NotASitemap}. */
export function readSitemap(body: string | Uint8Array, options: SitemapOptions = {}): Sitemap {
  const r = reader(options);
  try {
    const first = JSON.parse(r.writeJson(body)) as SitemapEntry[];
    const done = JSON.parse(r.endJson()) as Sitemap;
    if (first.length) done.entries = first.concat(done.entries);
    return done;
  } catch (e) {
    sitemapError(e);
  }
}

/** {@link readSitemap} on the thread pool: the JS thread stays free while a large sitemap is read. */
export async function readSitemapAsync(body: string | Uint8Array, options: SitemapOptions = {}): Promise<Sitemap> {
  const o: { maxBytes?: number; maxEntries?: number; maxEntryBytes?: number } = {};
  if (options.maxBytes !== undefined) o.maxBytes = options.maxBytes;
  if (options.maxEntries !== undefined) o.maxEntries = options.maxEntries;
  if (options.maxEntryBytes !== undefined) o.maxEntryBytes = options.maxEntryBytes;
  try {
    return JSON.parse(await native.readSitemapAsync(body, options.url ?? null, o)) as Sitemap;
  } catch (e) {
    sitemapError(e);
  }
}
