/**
 * HTML read as a stream: CSS selectors matched as elements open, values
 * read when they close, no DOM. The tree is the one htmlparser2 builds and
 * selectors match what css-select matches on it.
 */

import { Grammar, rethrow } from "./grammar.js";
import * as native from "./native.js";
import type { NativeExtractor, NativeExtractorOptions, NativeOutput, NativeSession } from "./native.js";

/** What a field reads with a pest grammar. */
export interface ParseSpec {
  /** A grammar named in {@link ExtractorOptions.grammars}. */
  grammar: string;
  rule?: string;
  /** Search the value for the first match instead of parsing from its start. */
  find?: boolean;
  /** Every match (an array). */
  all?: boolean;
  limit?: number;
  /** Shape options (see `Grammar.shape`). */
  lists?: string[];
  numbers?: string[];
  byRule?: boolean;
  trim?: boolean;
  /** The parse tree (`{ rule, tag?, text, children? }`) instead of the shaped value. */
  tree?: boolean;
}

/** One field: where its value is and how it is read. */
export interface FieldSpec {
  /** A CSS selector, relative to the field's scope; absent: the scope element itself. */
  select?: string;
  /** Every match (an array), not only the first with a value. */
  all?: boolean;
  /** With `all`: at most this many. */
  limit?: number;
  /** Read this attribute instead of the text. */
  attr?: string;
  /** `visible` (default: as a reader sees it), `raw` (all text as is, scripts' too), `own` (the element's own text only). */
  text?: "visible" | "raw" | "own";
  /** Read the element's outer HTML. */
  html?: boolean;
  /** `true` when something matches, else `false`. */
  exists?: boolean;
  /** Resolve against the page (`true`), or as an http(s) page without `#…` (`"http"`; other schemes are no value). */
  url?: boolean | "http";
  /** A regex (case-insensitive unless it starts with `(?`): its first group, else its match. No match, no value. */
  pattern?: string;
  /** Cut the value to this many characters. */
  max?: number;
  /** Collapse whitespace (default: for visible and own text and attributes). */
  collapse?: boolean;
  /** Read as a number (`€ 25,-` is 25). */
  number?: boolean;
  /** Parse as JSON (CDATA, comments and raw control characters tolerated). */
  json?: boolean;
  /** Parse with a pest grammar: `"name"`, `"name.rule"`, or a {@link ParseSpec}. Needs a build with grammars ({@link hasGrammars}). */
  parse?: string | ParseSpec;
  /** An object per match, its fields relative to the match. */
  fields?: Record<string, Field>;
  /** The value when nothing matched. */
  default?: unknown;
  /** Top-level, with `all`: hand items over as they complete (see {@link ExtractSession.take}) instead of keeping them. */
  emit?: boolean;
}

/** A field: a selector (`"h1"`, `"meta[property=og:image]@content"` for an attribute), `[selector]` for all matches, or a {@link FieldSpec}. */
export type Field = string | [string] | [FieldSpec] | FieldSpec;

/** What to extract: an object of fields. */
export type Spec = Record<string, Field>;

/** Options of the template fingerprint. */
export interface SkeletonOptions {
  /** How deep below `<body>` paths go (default 14). */
  depth?: number;
  /** Elements left out with their content (default: scripts, styles, forms, media, `<head>` …). */
  ignore?: string[];
  /** Also return the paths. */
  paths?: boolean;
}

/**
 * Options of the stripped page (defaults: festimap's learn prompt). Every length is in UTF-16
 * units, as `string.length` counts them: an emoji or `𝕊` is two.
 */
export interface StripOptions {
  /** The budget (default 36,000: 12k tokens). */
  maxChars?: number;
  /** A text run longer than this is cut with `…` (80). */
  maxText?: number;
  /** An attribute value longer than this is cut, a `data-*` one dropped (120). */
  maxAttr?: number;
  /** Alike siblings (same tag and classes) past this many collapse to a comment (4). */
  maxAlike?: number;
  /** A JSON-LD block longer than this is cut (4,000). */
  maxJsonLd?: number;
  /** Elements dropped with their content. */
  drop?: string[];
  /** Attributes kept, besides `data-*`. */
  keepAttrs?: string[];
  keepData?: boolean;
  /** Title, matching `<meta>`s and JSON-LD up front (true). */
  head?: boolean;
  /** A regex of the `<meta>` keys kept up front. */
  meta?: string;
  /** Elements dropped when empty and without attributes but `class`. */
  dropEmpty?: string[];
}

/** What the structured-data pass reads and how it writes it. */
export interface StructuredOptions {
  /** Which formats to read (all by default). */
  jsonLd?: boolean;
  microdata?: boolean;
  rdfa?: boolean;
  microformats?: boolean;
  /** Also each format in its own standard form (`structured.raw`). */
  raw?: boolean;
  /** The flat events view (`structured.events`, default true). */
  events?: boolean;
  /** Keep only items of these types, wherever they are (schema.org subtypes count: `"Event"` keeps `Festival`). */
  types?: string[];
  /** Every property value as an array, even a single one. */
  arrays?: boolean;
  /** Literals with language/datatype as `{ "@value", "@language" | "@type" }`, links as `{ "@id" }`. */
  detailed?: boolean;
  /** Embed the item a link (`@id`, `itemid`, an RDFa subject) names (default true). */
  embed?: boolean;
  /** Collapse whitespace in strings (default true). */
  collapse?: boolean;
  /**
   * Give every JSON-LD, microdata and RDFa item its JSON-LD `@context` (default true): `"https://schema.org"`,
   * or that and the prefixes it uses (`["https://schema.org", { "og": "http://ogp.me/ns#" }]`). Each item is then
   * the same self-contained JSON-LD whichever format the page used.
   */
  context?: boolean;
  /**
   * Also list every item that sits inside another (an event's place, a product's offer, an
   * article's author) as an item of its own, besides embedded where it is (default false).
   */
  flatten?: boolean;
  /** The vocabulary of JSON-LD that states no `@context` (default schema.org; `""`: none). */
  defaultVocab?: string;
  /** Microdata as Google reads it: `content` on any element, `itemtype` without `itemscope` (default true). */
  lenientMicrodata?: boolean;
  /**
   * Read what sites get wrong as they mean it (default true): microdata's `content` anywhere and
   * `itemtype` without `itemscope`, RDFa terms without a `vocab` as schema.org, JSON-LD repaired
   * (trailing commas, raw newlines) and read without a context as schema.org. `false`: every format
   * exactly as its specification has it (WHATWG microdata, RDFa 1.1 Core and HTML+RDFa, JSON-LD 1.1
   * Processing Algorithms, with schema.org's context built in and nothing fetched; a block that is
   * not valid JSON-LD is left out with a warning naming the spec's error code).
   */
  lenient?: boolean;
  /** One event for the same event stated twice, in two formats or two blocks (default true; see `StructuredEvent.formats`). */
  mergeEvents?: boolean;
}

/** An item, of any type (Product, Organization, Article, Event, Recipe, h-card …): compact JSON-LD (`{ "@context", "@type", "@id", "@format", …properties }`), schema.org names short. */
export type StructuredItem = Record<string, unknown> & { "@context"?: string | [string, Record<string, string>]; "@type"?: string | string[]; "@id"?: string; "@format"?: "json-ld" | "microdata" | "rdfa" | "microformats" };

/** An event, flat, from whichever format stated it. */
export interface StructuredEvent {
  name?: string;
  /** As the page states it (usually ISO 8601). */
  startDate?: string;
  endDate?: string;
  doorTime?: string;
  duration?: string;
  status?: string;
  attendanceMode?: string;
  url?: string;
  description?: string;
  image?: string[];
  location?: { name?: string; address?: string; street?: string; locality?: string; region?: string; postalCode?: string; country?: string; lat?: string; lon?: string; url?: string }[];
  performers?: { name?: string; url?: string; sameAs?: string[]; type?: string }[];
  organizer?: { name?: string; url?: string; sameAs?: string[]; type?: string }[];
  /** As the page states them. */
  offers?: { name?: string; price?: string; lowPrice?: string; highPrice?: string; currency?: string; url?: string; availability?: string; validFrom?: string }[];
  keywords?: string[];
  superEvent?: string;
  types?: string[];
  format: "json-ld" | "microdata" | "rdfa" | "microformats";
  /** When the event was stated in more than one place and merged: each format, the first is `format`. */
  formats?: ("json-ld" | "microdata" | "rdfa" | "microformats")[];
  id?: string;
}

export interface Structured {
  items: StructuredItem[];
  events: StructuredEvent[];
  raw?: {
    /**
     * One entry per JSON-LD `<script>`, in page order, so `jsonLd[i]` is the page's i-th block
     * even when one before it is broken: `null` for a block that cannot be read, an array for
     * several values glued together in one.
     */
    jsonLd: unknown[];
    microdata: unknown;
    rdfa: unknown[];
    microformats: unknown;
  };
  /** What was repaired or cut (`json-ld: trailing commas`). */
  warnings: string[];
}

export interface ExtractorOptions {
  /** Grammars a field's `parse` can name. */
  grammars?: Record<string, Grammar | string>;
  /** Also fingerprint the page's template. */
  skeleton?: boolean | SkeletonOptions;
  /** Also strip the page down for a model's prompt. */
  strip?: boolean | StripOptions;
  /** Leave out fields that do not compile (see {@link Extractor.warnings}) instead of throwing. */
  lenient?: boolean;
  /** How deep elements nest at most (1024); deeper start tags are ignored. */
  maxDepth?: number;
  /** The most one value may hold, in bytes (4 MiB). */
  maxCapture?: number;
  /** Also read the page's structured data: JSON-LD, microdata, RDFa (OpenGraph), microformats. */
  structured?: boolean | StructuredOptions;
  /** Also read what a scraper wants of any page: title, meta, robots, links, images, feeds, embedded JSON … ({@link Page}). */
  page?: boolean | PageOptions;
  /** Also take the page's accessibility snapshot, as Playwright's `ariaSnapshot()` gives it ({@link AriaSnapshot}). */
  aria?: boolean | AriaOptions;
}

/** Options of the accessibility snapshot. */
export interface AriaOptions {
  /**
   * `"default"`: what Playwright's `page.ariaSnapshot()` gives. `"ai"`: its snapshot for agents
   * (`ariaSnapshot({ mode: "ai" })`): a ref on each element that can be acted on (`[ref=e3]`,
   * with a CSS selector for it in {@link AriaSnapshot.refs}), generic containers kept, names
   * that repeat their content cut.
   */
  mode?: "default" | "ai";
  /** How deep nodes are written at most. */
  depth?: number;
  /**
   * Apply the page's own `<style>` blocks: their `display`, `visibility`, `content-visibility`,
   * `cursor` and `pointer-events` rules, media queries read for a 1280×720 screen (true). Style
   * sheets the page links to are never fetched.
   */
  styleBlocks?: boolean;
  /**
   * Also leave out text the page conceals from sight but leaves in its markup: no size
   * (`font-size: 0`), no opacity, far off screen, clipped away, or hidden by a framework's class
   * (`hidden`, `d-none`, `is-hidden`). Text meant for screen readers (`sr-only`) stays. For pages
   * read by a model: hidden text is where instructions get slipped in (false).
   */
  concealed?: boolean;
}

/** A page's accessibility tree: roles, names, states, links' URLs and form values, as YAML. */
export interface AriaSnapshot {
  /**
   * The snapshot, as Playwright writes it:
   *
   * ```yaml
   * - navigation "Main":
   *   - link "Agenda":
   *     - /url: /agenda
   * - main:
   *   - heading "Nora & Co" [level=1]
   *   - button "Buy tickets" [disabled]
   * ```
   */
  snapshot: string;
  /** In `"ai"` mode, a CSS selector for each ref, for a browser to act on it (`{ e3: "#buy" }`). */
  refs: Record<string, string>;
}

export interface Skeleton {
  /** sha256 (hex) of the page's element paths. */
  hash: string;
  paths?: string[];
}

export interface Stripped {
  html: string;
  /** Estimated: `html.length / 3`, rounded up. */
  tokens: number;
  truncated: boolean;
}

export interface ExtractStats {
  bytes: number;
  elements: number;
  maxDepth: number;
  /** Start tags ignored past `maxDepth`. */
  tooDeep: number;
  /** Elements a field matched. */
  matches: number;
}

export interface ExtractResult<T> {
  data: T;
  /** Values that could not be read (bad JSON, a grammar that did not match), each with its field. */
  errors: string[];
  skeleton?: Skeleton;
  stripped?: Stripped;
  structured?: Structured;
  /** With the `page` option: what a scraper wants of any page. */
  page?: Page;
  /** With the `aria` option: the accessibility snapshot. */
  aria?: AriaSnapshot;
  /** The encoding the page was read in (`UTF-8`, `windows-1252`, `Shift_JIS` …). */
  charset: string;
  stats: ExtractStats;
}

/** What the page pass reads, and how much of it (all of it by default). */
export interface PageOptions {
  links?: boolean;
  images?: boolean;
  headings?: boolean;
  /** JSON in scripts: `__NEXT_DATA__`, `<script type="application/json">`, `window.__INITIAL_STATE__ = {…}`. */
  embedded?: boolean;
  /** At most this many links (5000), images (2000), headings (500), embedded values (32). */
  maxLinks?: number;
  maxImages?: number;
  maxHeadings?: number;
  maxEmbedded?: number;
  /** The most characters a link's or heading's text keeps (240). */
  maxText?: number;
}

/** What a scraper wants of any page. Every URL is absolute (against `<base href>`); links and images are http(s), without `#…`. */
export interface Page {
  /** The encoding the page was read in. */
  charset: string;
  title?: string;
  /** `<meta name="description">`, else OpenGraph's or Twitter's. */
  description?: string;
  /** `<html lang>`, else `content-language`, else `og:locale`. */
  lang?: string;
  canonical?: string;
  base?: string;
  /** `<meta name="robots">` (and `googlebot`): `noindex`/`nofollow`/`none` read, every directive listed. */
  robots: { index: boolean; follow: boolean; directives?: string[] };
  /** `<meta http-equiv="refresh" content="5; url=…">`. */
  refresh?: { seconds: number; url?: string };
  /** Every `<meta name|property|itemprop|http-equiv content>`, by key, lowercase (the first of each): OpenGraph, Twitter cards … */
  meta: Record<string, string>;
  /** `<link rel="alternate">` by language or media. */
  alternates?: { href: string; hreflang?: string; type?: string; media?: string; title?: string }[];
  amp?: string;
  /** RSS, Atom and JSON feeds. */
  feeds?: { href: string; type?: string; title?: string }[];
  /** iCalendar: `<link type="text/calendar">`, `webcal:` (as `https:`) and `.ics` links. */
  calendars?: string[];
  icons?: { href: string; rel: string; sizes?: string; type?: string }[];
  manifest?: string;
  /** Pagination: `rel="next"`/`"prev"` on `<link>` or `<a>`. */
  next?: string;
  prev?: string;
  /** `<a href>` once each, with its text (or its `title`, `aria-label`, an image's `alt`). */
  links?: { href: string; text: string; rel?: string[]; nofollow?: boolean }[];
  /** `<img>`, lazy-loaded ones by their real address (`data-src`, `srcset`'s largest). */
  images?: { src: string; alt?: string; width?: number; height?: number }[];
  headings?: { level: number; text: string }[];
  /** `<iframe src>`: maps, videos, ticket shops. */
  frames?: string[];
  /** `mailto:` and `tel:` links. */
  emails?: string[];
  phones?: string[];
  /** JSON in scripts, by where it was (`__NEXT_DATA__`, `window.__INITIAL_STATE__`, `application/json`). */
  embedded?: { name: string; value: unknown }[];
  /** A bot wall instead of the page: `cloudflare`, `datadome`, `perimeterx`, `incapsula`, `akamai`, `captcha`. */
  blocked?: string;
  /** What was cut at a limit. */
  warnings?: string[];
}

/** An item of an `emit` field. */
export interface Item {
  field: string;
  value: unknown;
}

/** Chunks of a document: a string, bytes, a (web or Node) stream, a `fetch` response, or any (async) iterable of chunks. */
export type Source =
  | string
  | Uint8Array
  | Response
  | ReadableStream<Uint8Array>
  | AsyncIterable<string | Uint8Array>
  | Iterable<string | Uint8Array>;

export interface ExtractOptions {
  /** The page's URL: relative URLs resolve against it (and its `<base href>`). */
  url?: string | undefined;
  /**
   * The charset the server stated (`Content-Type: text/html; charset=…`), for bytes. Without it
   * the page's own `<meta charset>`, a byte order mark, or its bytes decide (UTF-8, else
   * windows-1252). Strings are text already decoded: read as they are.
   */
  charset?: string | undefined;
}

function result<T>(o: NativeOutput): ExtractResult<T> {
  const out: ExtractResult<T> = { data: o.data as T, errors: o.errors, charset: o.charset, stats: o.stats };
  if (o.page) out.page = o.page as Page;
  if (o.skeleton) out.skeleton = o.skeleton.paths ? o.skeleton : { hash: o.skeleton.hash };
  if (o.stripped) out.stripped = o.stripped;
  if (o.structured) out.structured = o.structured as Structured;
  if (o.aria) out.aria = { snapshot: o.aria.snapshot, refs: Object.fromEntries(o.aria.refs.map((r) => [r.id, r.selector])) };
  return out;
}

function nativeOptions(options: ExtractorOptions): NativeExtractorOptions {
  const out: NativeExtractorOptions = {};
  if (options.grammars) {
    out.grammars = Object.fromEntries(
      Object.entries(options.grammars).map(([name, g]) => [name, typeof g === "string" ? g : g.source]),
    );
  }
  if (options.skeleton) out.skeleton = options.skeleton === true ? {} : options.skeleton;
  if (options.strip) out.strip = options.strip === true ? {} : options.strip;
  if (options.lenient !== undefined) out.lenient = options.lenient;
  if (options.page) out.page = options.page === true ? {} : options.page;
  if (options.maxDepth !== undefined) out.maxDepth = options.maxDepth;
  if (options.maxCapture !== undefined) out.maxCapture = options.maxCapture;
  if (options.structured) out.structured = options.structured === true ? {} : options.structured;
  if (options.aria) out.aria = options.aria === true ? {} : options.aria;
  return out;
}

export async function* chunks(source: Source): AsyncGenerator<string | Uint8Array> {
  if (typeof source === "string" || source instanceof Uint8Array) {
    yield source;
    return;
  }
  let s: unknown = source;
  if (typeof Response !== "undefined" && s instanceof Response) {
    if (!s.body) return;
    s = s.body;
  }
  if (typeof s === "object" && s !== null && Symbol.asyncIterator in s) {
    yield* s as AsyncIterable<string | Uint8Array>;
    return;
  }
  if (typeof (s as ReadableStream<Uint8Array>).getReader === "function") {
    const reader = (s as ReadableStream<Uint8Array>).getReader();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return;
        yield value;
      }
    } finally {
      reader.releaseLock();
    }
  }
  yield* s as Iterable<string | Uint8Array>;
}

/**
 * A compiled spec. Compile once, extract from many pages; it is cheap to
 * keep and safe to use from any number of pages at once.
 *
 * ```ts
 * const x = new Extractor({
 *   title: "h1",
 *   events: { select: "article.event", all: true, fields: {
 *     title: "h2",
 *     url: { select: "h2 a", attr: "href", url: "http" },
 *     when: { select: ".date", parse: { grammar: "date", find: true } },
 *     acts: ["ul.acts > li"],
 *   } },
 * }, { grammars: { date: dateGrammar } });
 * const { data } = await x.stream(await fetch(url), { url });
 * ```
 */
export class Extractor<T = Record<string, unknown>> {
  /** @internal */
  readonly native: NativeExtractor;
  /** Fields left out of a `lenient` spec, with why. */
  readonly warnings: readonly string[];

  /** @throws {SpecError} {GrammarError} */
  constructor(spec: Spec, options: ExtractorOptions = {}) {
    try {
      this.native = new native.NativeExtractor(spec, nativeOptions(options));
    } catch (e) {
      rethrow(e);
    }
    this.warnings = this.native.warnings;
  }

  /** A whole document (on this thread). */
  extract(html: string | Uint8Array, options: ExtractOptions = {}): ExtractResult<T> {
    return result(this.native.extract(html, options.url ?? null, options.charset ?? null));
  }

  /** A whole document, read on the thread pool. */
  async extractAsync(html: string | Uint8Array, options: ExtractOptions = {}): Promise<ExtractResult<T>> {
    return result(await this.native.extractAsync(html, options.url ?? null, options.charset ?? null));
  }

  /** A document to feed chunk by chunk. */
  session(options: ExtractOptions = {}): ExtractSession<T> {
    return new ExtractSession(this.native.session(options.url ?? null, options.charset ?? null));
  }

  /**
   * A document read as it arrives (a `fetch` response, a file stream …):
   * never held whole, `onItem` called with each item of an `emit` field as
   * soon as it is complete. A `fetch` response gives its own URL and the
   * charset of its `Content-Type` unless `options` say otherwise.
   */
  async stream(
    source: Source,
    options: ExtractOptions & { onItem?: (item: Item) => void; signal?: AbortSignal } = {},
  ): Promise<ExtractResult<T>> {
    if (typeof Response !== "undefined" && source instanceof Response) {
      options = { url: source.url || undefined, charset: charsetOf(source.headers.get("content-type")), ...stripUndefined(options) };
    }
    const session = this.session(options);
    for await (const chunk of chunks(source)) {
      options.signal?.throwIfAborted();
      session.write(chunk);
      if (options.onItem) for (const item of session.take()) options.onItem(item);
    }
    const out = session.end();
    if (options.onItem) for (const item of session.take()) options.onItem(item);
    return out;
  }
}

/** One document being read. */
export class ExtractSession<T = Record<string, unknown>> {
  #native: NativeSession;
  #ended = false;
  /** Items the end of the document completed, until taken. */
  private taken: Item[] = [];

  /** @internal */
  constructor(session: NativeSession) {
    this.#native = session;
  }

  /** The next chunk (bytes in the page's encoding, or a string; any size, cut anywhere). */
  write(chunk: string | Uint8Array): this {
    this.#native.write(chunk);
    return this;
  }

  /** Items of `emit` fields completed since the last call, in document order. */
  take(): Item[] {
    return this.#ended ? this.taken.splice(0) : this.#native.take();
  }

  /** The end of the document: everything found. Items the end completed are left for one more {@link take}. */
  end(): ExtractResult<T> {
    const out = this.#native.end();
    this.taken = out.items;
    this.#ended = true;
    return result(out);
  }
}

/** Strips a page down for a reader with a budget (a model's prompt); see {@link StripOptions}. */
export function strip(html: string | Uint8Array, options: StripOptions = {}): Stripped {
  return new Extractor({}, { strip: options }).extract(html).stripped!;
}

/** A page's template fingerprint; see {@link SkeletonOptions}. */
export function skeleton(html: string | Uint8Array, options: SkeletonOptions = {}): Skeleton {
  return new Extractor({}, { skeleton: options }).extract(html).skeleton!;
}

/** The fingerprint of a template several pages share: the paths at least half of them have, hashed. */
export function templateHash(pages: readonly (readonly string[])[]): string | undefined {
  return native.templateHash(pages.map((p) => [...p])) ?? undefined;
}

const structuredExtractors = new Map<string, Extractor>();

/**
 * A page's structured data in one call: JSON-LD, microdata, RDFa (OpenGraph,
 * Dublin Core) and microformats, as `items` (every item) and `events` (flat).
 * Never throws on bad markup: what can be read is read, the rest is in `warnings`.
 *
 * ```ts
 * const { events } = extractStructured(html, { url });
 * const venues = extractStructured(html, { url, types: ["Place"] }).items;
 * ```
 */
export function extractStructured(
  html: string | Uint8Array,
  options: StructuredOptions & { url?: string } = {},
): Structured {
  const { url, ...rest } = options;
  const key = JSON.stringify(rest);
  let x = structuredExtractors.get(key);
  if (!x) {
    x = new Extractor({}, { structured: rest });
    if (structuredExtractors.size > 64) structuredExtractors.clear();
    structuredExtractors.set(key, x);
  }
  return x.extract(html, url === undefined ? {} : { url }).structured!;
}

/** {@link extractStructured} on the thread pool (the JS thread stays free). */
export async function extractStructuredAsync(
  html: string | Uint8Array,
  options: StructuredOptions & { url?: string } = {},
): Promise<Structured> {
  const { url, ...rest } = options;
  const key = JSON.stringify(rest);
  let x = structuredExtractors.get(key);
  if (!x) {
    x = new Extractor({}, { structured: rest });
    if (structuredExtractors.size > 64) structuredExtractors.clear();
    structuredExtractors.set(key, x);
  }
  return (await x.extractAsync(html, url === undefined ? {} : { url })).structured!;
}

/** The charset of a `Content-Type` header (`text/html; charset=ISO-8859-1` → `ISO-8859-1`). */
export function charsetOf(contentType: string | null | undefined): string | undefined {
  const m = /charset\s*=\s*["']?([^"';\s]+)/i.exec(contentType ?? "");
  return m ? m[1] : undefined;
}

function stripUndefined<O extends object>(o: O): O {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as O;
}

const pageExtractors = new Map<string, Extractor>();

function pageExtractor(key: object, options: ExtractorOptions): Extractor {
  const k = JSON.stringify(key);
  let x = pageExtractors.get(k);
  if (!x) {
    x = new Extractor({}, options);
    if (pageExtractors.size > 64) pageExtractors.clear();
    pageExtractors.set(k, x);
  }
  return x;
}

/**
 * What a scraper wants of any page in one call ({@link Page}): title, description, language,
 * canonical, robots, meta refresh, every meta tag, alternates, feeds, calendars, icons,
 * pagination, links with text, images, headings, frames, e-mail and phone, embedded JSON, and
 * whether it is a bot wall. Bytes are read in their charset (`options.charset`, else the page's).
 */
export function extractPage(html: string | Uint8Array, options: PageOptions & ExtractOptions = {}): Page {
  const { url, charset, ...page } = options;
  return pageExtractor({ page }, { page }).extract(html, { url, charset }).page!;
}

/** {@link extractPage} on the thread pool. */
export async function extractPageAsync(html: string | Uint8Array, options: PageOptions & ExtractOptions = {}): Promise<Page> {
  const { url, charset, ...page } = options;
  return (await pageExtractor({ page }, { page }).extractAsync(html, { url, charset })).page!;
}

/**
 * A page's accessibility snapshot, as Playwright's `ariaSnapshot()` gives it, from its markup
 * alone and many times faster: no browser, no layout. What only a browser knows is read from the
 * markup ({@link AriaOptions}); scripts are not run, so give it the HTML a page renders to when
 * scripts build it. Bytes are read in their charset (`options.charset`, else the page's).
 *
 * ```ts
 * const { snapshot, refs } = ariaSnapshot(html, { url, mode: "ai" });
 * // - link "Buy tickets" [ref=e12] [cursor=pointer]   →   refs.e12: "body > main:nth-child(2) > a:nth-child(3)"
 * ```
 */
export function ariaSnapshot(html: string | Uint8Array, options: AriaOptions & ExtractOptions = {}): AriaSnapshot {
  const { url, charset, ...aria } = options;
  return pageExtractor({ aria }, { aria }).extract(html, { url, charset }).aria!;
}

/** {@link ariaSnapshot} on the thread pool. */
export async function ariaSnapshotAsync(
  html: string | Uint8Array,
  options: AriaOptions & ExtractOptions = {},
): Promise<AriaSnapshot> {
  const { url, charset, ...aria } = options;
  return (await pageExtractor({ aria }, { aria }).extractAsync(html, { url, charset })).aria!;
}

/** Everything a scraper reads of a page, in one pass. */
export interface Scraped {
  /** What any page says of itself ({@link Page}). */
  page: Page;
  /** Its structured data as JSON-LD items, and its events ({@link Structured}). */
  structured: Structured;
  /** With the `aria` option: its accessibility snapshot ({@link AriaSnapshot}). */
  aria?: AriaSnapshot;
  /** The encoding it was read in. */
  charset: string;
}

/**
 * A page read whole for a scraper, in one pass: {@link Page} and {@link Structured}. Takes a
 * string, bytes, or anything {@link Extractor.stream} takes (a `fetch` response: its URL and
 * charset are used).
 *
 * ```ts
 * const { page, structured } = await scrape(await fetch(url));
 * if (page.blocked) retryLater();
 * for (const item of structured.items) save(item);   // every item, of every type and format, as JSON-LD
 * ```
 */
export async function scrape(
  source: Source,
  options: ExtractOptions & {
    page?: PageOptions;
    structured?: StructuredOptions;
    /** Also take the accessibility snapshot, in the same pass. */
    aria?: boolean | AriaOptions;
    signal?: AbortSignal;
  } = {},
): Promise<Scraped> {
  const { page = {}, structured = {}, aria, ...rest } = options;
  const ariaOptions = aria ? (aria === true ? {} : aria) : undefined;
  const x = pageExtractor(
    { scrape: true, page, structured, aria: ariaOptions },
    ariaOptions ? { page, structured, aria: ariaOptions } : { page, structured },
  );
  const out =
    typeof source === "string" || source instanceof Uint8Array
      ? await x.extractAsync(source, rest)
      : await x.stream(source, rest);
  const scraped: Scraped = { page: out.page!, structured: out.structured!, charset: out.charset };
  if (out.aria) scraped.aria = out.aria;
  return scraped;
}
