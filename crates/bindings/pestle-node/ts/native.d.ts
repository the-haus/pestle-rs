// Types of the N-API module (see src/lib.rs). The public, documented API
// is in index.ts; these are internal.

export interface NativeRule {
  name: string;
  kind: "normal" | "silent" | "atomic" | "compound" | "nonAtomic";
}

export interface NativeError {
  message: string;
  line: number;
  col: number;
  offset: number;
  end?: number;
  expected: string[];
  unexpected: string[];
}

/** `[rule, tag, start, end, size]` per pair; tag `0xFFFFFFFF`: none. */
export interface NativeTree {
  nodes: Uint32Array;
  end: number;
}

export interface NativeFound {
  start: number;
  end: number;
  tree: NativeTree;
}

export interface NativeShapeOptions {
  rule?: string;
  lists?: string[];
  numbers?: string[];
  byRule?: boolean;
  trim?: boolean;
}

export class NativeShaper {
  json(input: string): string;
  jsonAll(inputs: string[]): string;
}

export interface NativeRecovered {
  rule: string;
  start: number;
  at: number;
  end: number;
  error: NativeError;
}

export class NativeGrammar {
  constructor(source: string);
  readonly rules: NativeRule[];
  readonly names: string[];
  readonly tags: string[];
  readonly defaultRule: string | null;
  parse(input: string, rule?: string | null): { tree?: NativeTree; error?: NativeError };
  parseRecovering(
    input: string,
    rule: string | null,
    recovery: { rules: string[]; sync: string[]; consume?: boolean },
  ): { tree?: NativeTree; recovered: NativeRecovered[]; error?: NativeError };
  shape(input: string, options?: NativeShapeOptions | null): { value: unknown; error?: NativeError };
  shaper(options?: NativeShapeOptions | null): NativeShaper;
  find(input: string, rule?: string | null): NativeFound | null;
  findAll(input: string, rule?: string | null, limit?: number | null): NativeFound[];
}

export interface NativeSkeletonOptions {
  depth?: number;
  ignore?: string[];
  paths?: boolean;
}

export interface NativeStripOptions {
  maxChars?: number;
  maxText?: number;
  maxAttr?: number;
  maxAlike?: number;
  maxJsonLd?: number;
  drop?: string[];
  keepAttrs?: string[];
  keepData?: boolean;
  head?: boolean;
  meta?: string;
  dropEmpty?: string[];
}

export interface NativeStructuredOptions {
  jsonLd?: boolean;
  microdata?: boolean;
  rdfa?: boolean;
  microformats?: boolean;
  raw?: boolean;
  events?: boolean;
  types?: string[];
  arrays?: boolean;
  detailed?: boolean;
  embed?: boolean;
  collapse?: boolean;
  context?: boolean;
  flatten?: boolean;
  defaultVocab?: string;
  lenientMicrodata?: boolean;
  lenient?: boolean;
  mergeEvents?: boolean;
}

export interface NativeExtractorOptions {
  grammars?: Record<string, string>;
  skeleton?: NativeSkeletonOptions;
  strip?: NativeStripOptions;
  lenient?: boolean;
  maxDepth?: number;
  maxCapture?: number;
  structured?: NativeStructuredOptions;
  page?: NativePageOptions;
  aria?: NativeAriaOptions;
}

export interface NativeAriaOptions {
  mode?: "default" | "ai";
  depth?: number;
  styleBlocks?: boolean;
  concealed?: boolean;
}

export interface NativePageOptions {
  links?: boolean;
  images?: boolean;
  headings?: boolean;
  embedded?: boolean;
  maxLinks?: number;
  maxImages?: number;
  maxHeadings?: number;
  maxText?: number;
  maxEmbedded?: number;
  text?: boolean;
  maxTextBytes?: number;
}

export interface NativeOutput {
  data: unknown;
  errors: string[];
  skeleton?: { hash: string; paths?: string[] };
  stripped?: { html: string; tokens: number; truncated: boolean };
  structured?: unknown;
  page?: unknown;
  aria?: { snapshot: string; refs: { id: string; selector: string }[] };
  charset: string;
  stats: { bytes: number; elements: number; maxDepth: number; tooDeep: number; matches: number };
  items: NativeItem[];
}

export interface NativeItem {
  field: string;
  value: unknown;
}

export class NativeSession {
  write(chunk: string | Uint8Array): void;
  take(): NativeItem[];
  end(): NativeOutput;
}

export class NativeExtractor {
  constructor(spec: unknown, options?: NativeExtractorOptions | null);
  readonly warnings: string[];
  extract(html: string | Uint8Array, url?: string | null, charset?: string | null): NativeOutput;
  extractAsync(html: string | Uint8Array, url?: string | null, charset?: string | null): Promise<NativeOutput>;
  session(url?: string | null, charset?: string | null): NativeSession;
}

export function setCallLimit(limit?: number | null): void;
/** Whether this build reads pest grammars (the `grammar` feature). */
export function hasGrammars(): boolean;
export function templateHash(pages: string[][]): string | null;
export function registrableDomain(host: string): string | null;
export function publicSuffix(host: string): string | null;
export function publicSuffixListVersion(): string;

export class NativeRobots {
  constructor(text: string, token: string);
  static allowAll(): NativeRobots;
  static disallowAll(): NativeRobots;
  allows(url: string): boolean;
  allowsAll(urls: string[]): boolean[];
  get json(): unknown;
}

export interface NativeSitemapOptions {
  maxEntryBytes?: number;
  maxBytes?: number;
  maxEntries?: number;
}

export class NativeSitemapReader {
  constructor(url?: string | null, options?: NativeSitemapOptions | null);
  write(chunk: string | Uint8Array): unknown[];
  writeJson(chunk: string | Uint8Array): string;
  end(): unknown;
  endJson(): string;
}
export function readSitemapAsync(body: string | Uint8Array, url?: string | null, options?: NativeSitemapOptions | null): Promise<string>;

export interface NativeRouteOptions {
  minSiblings?: number;
  maxExamples?: number;
  maxUrls?: number;
}

export class NativeRouteBuilder {
  constructor(options?: NativeRouteOptions | null);
  add(urls: string[]): void;
  addEntries(entries: { url: string; lastmod?: string; images?: boolean }[]): void;
  finish(): NativeRoutes;
}

export class NativeRoutes {
  json(): string;
  findAll(urls: string[]): (string[] | null)[];
  outline(): string;
}
