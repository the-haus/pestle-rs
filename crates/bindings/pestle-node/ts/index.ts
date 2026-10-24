/**
 * @pestle-rs/node — HTML read as a stream, fast: CSS selectors over the
 * token stream, and the page's structured data (JSON-LD, microdata, RDFa,
 * microformats) in the same pass, as the page states it.
 *
 * ```ts
 * import { Extractor } from "@pestle-rs/node";
 *
 * const agenda = new Extractor({
 *   events: { select: "article.event", all: true, fields: {
 *     title: "h2",
 *     url: { select: "a", attr: "href", url: "http" },
 *     when: ".date",
 *     price: { select: ".price", number: true },
 *     acts: [".lineup li"],
 *   } },
 * }, { structured: true });
 *
 * const { data, structured } = await agenda.stream(await fetch(url), { url });
 * ```
 *
 * The document is tokenized and matched in Rust in one pass, as it
 * arrives: memory is the stack of open elements plus what the spec keeps,
 * never a DOM. Malformed pages give what can be read, never a crash.
 * pest grammars (`Grammar`, a field's `parse`) are in builds with the
 * `grammar` feature ({@link hasGrammars}).
 */

export {
  Grammar,
  GrammarError,
  Pair,
  ParseError,
  ParseTree,
  hasGrammars,
  setCallLimit,
  SpecError,
  type Match,
  type PairJSON,
  type ParseProblem,
  type Recovered,
  type Recovery,
  type RuleInfo,
  type ShapeOptions,
} from "./grammar.js";
export {
  ExtractSession,
  Extractor,
  ariaSnapshot,
  ariaSnapshotAsync,
  charsetOf,
  extractPage,
  extractPageAsync,
  extractStructured,
  extractStructuredAsync,
  scrape,
  skeleton,
  strip,
  templateHash,
  type AriaOptions,
  type AriaSnapshot,
  type ExtractOptions,
  type ExtractResult,
  type ExtractStats,
  type ExtractorOptions,
  type Field,
  type FieldSpec,
  type Item,
  type Page,
  type PageOptions,
  type ParseSpec,
  type Scraped,
  type Skeleton,
  type SkeletonOptions,
  type Source,
  type Spec,
  type Stripped,
  type StripOptions,
  type Structured,
  type StructuredEvent,
  type StructuredItem,
  type StructuredOptions,
} from "./extract.js";
export {
  NotASitemap,
  parseRobots,
  parseSitemap,
  readSitemap,
  readSitemapAsync,
  Robots,
  type RobotsRule,
  type Sitemap,
  type SitemapEntry,
  type SitemapOptions,
} from "./crawl.js";
