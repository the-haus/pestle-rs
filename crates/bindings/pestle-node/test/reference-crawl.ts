/**
 * festimap's robots.txt and sitemap readers (packages/sources/src/sites/
 * robots.ts and sitemap.ts), copied as they are, so the differential tests
 * compare pestle with what festimap runs.
 */
/* eslint-disable */
const ENTITIES: Readonly<Record<string, string>> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
};

/** HTML character references decoded (named for the common five, numeric all). */
export function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, ref: string) => {
    if (ref[0] === "#") {
      const code =
        ref[1] === "x" || ref[1] === "X"
          ? parseInt(ref.slice(2), 16)
          : parseInt(ref.slice(1), 10);
      return Number.isFinite(code) && code > 0 && code < 0x110000
        ? String.fromCodePoint(code)
        : whole;
    }
    return ENTITIES[ref.toLowerCase()] ?? whole;
  });
}


export type Rule = { readonly allow: boolean; readonly pattern: string };

export type Robots = {
  readonly rules: readonly Rule[];
  /** Seconds, when the group states one. */
  readonly crawlDelay?: number;
  /** The file's `Sitemap:` URLs (absolute, in file order, once each). */
  readonly sitemaps?: readonly string[];
};

/** Allows everything: a robots.txt that is absent (4xx). */
export const ALLOW_ALL: Robots = { rules: [] };
/** Allows nothing: a robots.txt that could not be read (5xx, no answer). */
export const DISALLOW_ALL: Robots = {
  rules: [{ allow: false, pattern: "/" }],
};

/** The rules of the group for `token` (else `*`) in a robots.txt. */
export function parseRobots(text: string, token = "festimapbot"): Robots {
  type Group = { agents: string[]; rules: Rule[]; delay?: number };
  const groups: Group[] = [];
  const sitemaps = new Set<string>();
  let current: Group | null = null;
  let inRules = false;
  for (const raw of text.split(/\r?\n|\r/)) {
    const line = raw.replace(/#.*/, "").trim();
    const at = line.indexOf(":");
    if (at < 0) continue;
    const field = line.slice(0, at).trim().toLowerCase();
    const value = line.slice(at + 1).trim();
    if (field === "sitemap") {
      // Not part of any group, and it does not end one.
      // `http:// /sitemap.xml` (a blank host) is no URL: dropped, so the
      // site is indexed from /sitemap.xml instead of failing discovery.
      if (/^https?:\/\//i.test(value) && URL.canParse(value))
        sitemaps.add(value);
      continue;
    }
    if (field === "user-agent") {
      // Consecutive user-agent lines open one group.
      if (!current || inRules) {
        current = { agents: [], rules: [] };
        groups.push(current);
        inRules = false;
      }
      current.agents.push(value.toLowerCase());
      continue;
    }
    if (!current) continue;
    if (field === "allow" || field === "disallow") {
      inRules = true;
      // An empty Disallow allows everything: no rule.
      if (value)
        current.rules.push({ allow: field === "allow", pattern: value });
    } else if (field === "crawl-delay") {
      inRules = true;
      const delay = Number(value);
      if (Number.isFinite(delay) && delay >= 0) current.delay = delay;
    }
  }
  const named = groups.filter((g) =>
    g.agents.some((a) => a !== "*" && token.startsWith(a.replace(/\/.*$/, ""))),
  );
  const chosen = named.length
    ? named
    : groups.filter((g) => g.agents.includes("*"));
  const rules = chosen.flatMap((g) => g.rules);
  const delays = chosen.flatMap((g) =>
    g.delay === undefined ? [] : [g.delay],
  );
  return {
    rules,
    ...(delays.length ? { crawlDelay: Math.max(...delays) } : {}),
    ...(sitemaps.size ? { sitemaps: [...sitemaps] } : {}),
  };
}

/** Whether a pattern (with `*` and a closing `$`) matches a path. */
function matches(pattern: string, path: string): boolean {
  const anchored = pattern.endsWith("$");
  const body = anchored ? pattern.slice(0, -1) : pattern;
  const regex = new RegExp(
    `^${body
      .split("*")
      .map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&"))
      .join(".*")}${anchored ? "$" : ""}`,
  );
  return regex.test(path);
}

/** Percent-encoding normalised the RFC's way, so `%7E` and `~` compare equal. */
function normalise(path: string): string {
  try {
    return encodeURI(decodeURI(path));
  } catch {
    return path;
  }
}

/** Whether robots allows `url` (its path and query). */
export function robotsAllow(robots: Robots, url: string): boolean {
  const u = new URL(url);
  const path = normalise(`${u.pathname}${u.search}`);
  let best: Rule | null = null;
  for (const rule of robots.rules) {
    const pattern = normalise(rule.pattern);
    if (!matches(pattern, path)) continue;
    if (
      !best ||
      pattern.length > best.pattern.length ||
      (pattern.length === best.pattern.length && rule.allow)
    )
      best = { ...rule, pattern };
  }
  return best ? best.allow : true;
}


/** One entry of a sitemap: a page (`<url>`), or a sitemap an index names (`<sitemap>`). */
export type SitemapItem =
  | {
      readonly kind: "url";
      readonly url: string;
      readonly lastmod?: string;
      readonly changefreq?: string;
      readonly priority?: number;
    }
  | {
      readonly kind: "sitemap";
      readonly url: string;
      readonly lastmod?: string;
    };

/** A body that is no sitemap (a soft 404's HTML page, say). */
export class NotASitemap extends Error {}

/** A `<url>` longer than this without its close is skipped. */
const MAX_BLOCK_CHARS = 64 * 1024;
/** How far into the body its root (`<urlset>`, `<sitemapindex>`) must be. */
const MAX_PROLOG_CHARS = 16 * 1024;

const CHANGEFREQS = new Set([
  "always",
  "hourly",
  "daily",
  "weekly",
  "monthly",
  "yearly",
  "never",
]);

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const httpUrl = (value: string | undefined): string | null => {
  if (!value || !/^https?:\/\//i.test(value)) return null;
  try {
    const u = new URL(value);
    u.hash = "";
    return u.toString();
  } catch {
    return null;
  }
};

const cleanText = (raw: string) =>
  decodeEntities(raw.replace(/<!\[CDATA\[([\s\S]*?)\]\]>/g, "$1")).trim();

/**
 * A sitemap's entries as its bytes arrive: XML (`<urlset>` or
 * `<sitemapindex>`, any namespace prefix) or text (one URL a line). Only
 * the entry in progress is held; an `<image:loc>` or `<xhtml:link>` inside
 * a `<url>` is never taken for its `<loc>`. Throws {@link NotASitemap} for
 * a body that is none (an HTML page).
 */
export async function* parseSitemap(
  chunks: AsyncIterable<Uint8Array>,
): AsyncGenerator<SitemapItem> {
  const decoder = new TextDecoder("utf-8");
  let buf = "";
  let mode: "xml" | "text" | null = null;
  let item: RegExp | null = null;
  let close = "";
  let tag: "url" | "sitemap" = "url";
  let fields: Record<
    "loc" | "lastmod" | "changefreq" | "priority",
    RegExp
  > | null = null;

  const findRoot = (end: boolean) => {
    const root = /<([\w.-]+:)?(urlset|sitemapindex)(?=[\s>/])/i.exec(buf);
    if (!root) {
      if (/<html[\s>]|<!doctype\s+html/i.test(buf))
        throw new NotASitemap("an HTML page, not a sitemap");
      if (end || buf.length > MAX_PROLOG_CHARS)
        throw new NotASitemap("no <urlset> or <sitemapindex>");
      return false;
    }
    const prefix = escapeRe(root[1] ?? "");
    tag = root[2]!.toLowerCase() === "urlset" ? "url" : "sitemap";
    item = new RegExp(`<${prefix}${tag}(?=[\\s>])[^>]*>`, "gi");
    close = `</${root[1] ?? ""}${tag}`;
    const field = (name: string) =>
      new RegExp(
        `<${prefix}${name}(?=[\\s>])[^>]*>([\\s\\S]*?)</${prefix}${name}\\s*>`,
        "i",
      );
    fields = {
      loc: field("loc"),
      lastmod: field("lastmod"),
      changefreq: field("changefreq"),
      priority: field("priority"),
    };
    buf = buf.slice(root.index);
    return true;
  };

  const parseBlock = (block: string): SitemapItem | null => {
    const read = (name: keyof NonNullable<typeof fields>) => {
      const m = fields![name].exec(block);
      return m ? cleanText(m[1]!) : undefined;
    };
    const url = httpUrl(read("loc"));
    if (!url) return null;
    const lastmodRaw = read("lastmod");
    const lastmod =
      lastmodRaw && Number.isFinite(Date.parse(lastmodRaw))
        ? lastmodRaw
        : undefined;
    if (tag === "sitemap")
      return { kind: "sitemap", url, ...(lastmod ? { lastmod } : {}) };
    const changefreq = read("changefreq")?.toLowerCase();
    const priorityRaw = read("priority");
    const priority = priorityRaw === undefined ? NaN : Number(priorityRaw);
    return {
      kind: "url",
      url,
      ...(lastmod ? { lastmod } : {}),
      ...(changefreq && CHANGEFREQS.has(changefreq) ? { changefreq } : {}),
      ...(Number.isFinite(priority) && priority >= 0 && priority <= 1
        ? { priority }
        : {}),
    };
  };

  function* drain(end: boolean): Generator<SitemapItem> {
    if (mode === null) {
      buf = buf.replace(/^[\uFEFF\s]+/, "");
      if (!buf) return;
      mode = buf[0] === "<" ? "xml" : "text";
    }
    if (mode === "text") {
      const lines = buf.split(/\r?\n|\r/);
      buf = end ? "" : lines.pop()!;
      for (const line of lines) {
        const url = httpUrl(line.trim());
        if (url) yield { kind: "url", url };
      }
      return;
    }
    if (!item && !findRoot(end)) return;
    const open = item!;
    let at = 0;
    for (;;) {
      open.lastIndex = at;
      const start = open.exec(buf);
      if (!start) {
        // Keep a tag that may be cut at the chunk's end.
        at = Math.max(at, buf.length - 256);
        break;
      }
      const bodyAt = start.index + start[0].length;
      const closeAt = buf.indexOf(close, bodyAt);
      const closeEnd = closeAt < 0 ? -1 : buf.indexOf(">", closeAt);
      if (closeEnd < 0) {
        if (buf.length - start.index > MAX_BLOCK_CHARS) {
          // A broken entry: skip it rather than hold the file.
          at = start.index + 1;
          continue;
        }
        at = start.index;
        break;
      }
      const parsed = parseBlock(buf.slice(bodyAt, closeAt));
      at = closeEnd + 1;
      if (parsed) yield parsed;
    }
    buf = end ? "" : buf.slice(at);
  }

  for await (const chunk of chunks) {
    buf += decoder.decode(chunk, { stream: true });
    yield* drain(false);
  }
  buf += decoder.decode();
  if (mode === null && !buf.trim()) throw new NotASitemap("an empty body");
  yield* drain(true);
}

