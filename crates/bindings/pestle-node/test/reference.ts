/**
 * The reference pestle is held to: htmlparser2 12 + css-select 7, and
 * festimap's own readers on top of them (`textOf`, `skeletonPaths`,
 * `cleanForLearn`, from packages/sources/src/sites/dom.ts), copied as they
 * are so the differential tests compare against what festimap runs.
 */
import { is as matches, selectAll } from "css-select";
import { DomUtils, ElementType, parseDocument } from "htmlparser2";

export type Doc = ReturnType<typeof parseDocument>;
type Node = Doc["children"][number];
export type El = Parameters<typeof DomUtils.getAttributeValue>[0];
type Text = Extract<Node, { type: typeof ElementType.Text }>;

export const parse = (html: string): Doc => parseDocument(html, { decodeEntities: true, lowerCaseTags: true });
const isElement = (n: Node): n is El => ElementType.isTag(n);
const isText = (n: Node): n is Text => n.type === ElementType.Text;

/** `inPage`: matched in the page's context, as `querySelectorAll` (css-select's `relativeSelector: false`). */
export function select(scope: Node | Node[], selector: string, inPage = false): El[] {
  return selectAll<Node, El>(selector, scope, inPage ? { relativeSelector: false } : {});
}

export function is(el: El, selector: string): boolean {
  return matches<Node, El>(el, selector);
}

// ── festimap dom.ts ──────────────────────────────────────────────────────

const INLINE = new Set([
  "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "em", "i", "kbd", "mark", "q", "s", "small", "span",
  "strong", "sub", "sup", "time", "u", "var", "wbr",
]);
const NEVER_TEXT = new Set(["script", "style", "noscript", "template", "svg", "iframe", "object", "canvas"]);
export const collapse = (text: string) => text.replace(/[\s ​]+/g, " ").trim();

export function textOf(node: Node | readonly Node[]): string {
  const parts: string[] = [];
  const walk = (n: Node) => {
    if (isText(n)) {
      parts.push(n.data);
      return;
    }
    if (!isElement(n)) {
      if ("children" in n) for (const c of n.children) walk(c);
      return;
    }
    if (NEVER_TEXT.has(n.name)) return;
    const block = !INLINE.has(n.name);
    if (block) parts.push(" ");
    for (const c of n.children) walk(c);
    if (block) parts.push(" ");
  };
  for (const n of Array.isArray(node) ? node : [node as Node]) walk(n);
  return collapse(parts.join(""));
}

const DROPPED = new Set([
  ...NEVER_TEXT, "link", "base", "input", "select", "option", "textarea", "button", "form", "video", "audio", "source",
  "picture", "map", "head",
]);
const VOID = new Set(["area", "br", "col", "embed", "hr", "img", "meta", "wbr"]);
const KEPT_ATTRIBUTES = new Set([
  "id", "class", "itemprop", "itemtype", "itemscope", "href", "src", "datetime", "content", "property", "name", "rel",
  "title", "alt", "aria-label", "lang", "role",
]);

export function metaOf(doc: Doc): Map<string, string> {
  const out = new Map<string, string>();
  for (const el of select(doc, "meta[content]")) {
    const key = (el.attribs.property ?? el.attribs.name ?? "").toLowerCase();
    const content = collapse(el.attribs.content ?? "");
    if (key && content && !out.has(key)) out.set(key, content);
  }
  return out;
}

const escapeText = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const escapeAttr = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;");
const cut = (s: string, max: number) => (s.length > max ? `${s.slice(0, max).trimEnd()}…` : s);
const likeness = (el: El) =>
  `${el.name}.${(el.attribs.class ?? "").split(/\s+/).filter(Boolean).sort().join(".")}`;
const isJsonLd = (el: El) =>
  el.name === "script" && (el.attribs.type ?? "").trim().toLowerCase().startsWith("application/ld+json");

export function cleanForLearn(html: string, maxTokens = 12_000): { html: string; truncated: boolean } {
  const maxChars = maxTokens * 3;
  const maxText = 80;
  const maxAttr = 120;
  const maxAlike = 4;
  const maxJsonLd = 4_000;
  const doc = parse(html);
  const out: string[] = [];
  let length = 0;
  let truncated = false;
  const emit = (s: string) => {
    out.push(s);
    length += s.length;
  };
  const full = (reserve: number) => length + reserve > maxChars;
  const title = DomUtils.findOne((n) => n.name === "title", doc.children);
  if (title) emit(`<title>${escapeText(cut(textOf(title), maxText))}</title>\n`);
  for (const [key, value] of metaOf(doc)) {
    if (/^(?:og:|twitter:title|description$|event:|music:)/.test(key))
      emit(`<meta property="${escapeAttr(key)}" content="${escapeAttr(cut(value, maxAttr))}">\n`);
  }
  for (const el of select(doc, "script")) {
    if (!isJsonLd(el)) continue;
    const json = collapse(textOf(el.children) || DomUtils.textContent(el));
    emit(`<script type="application/ld+json">${cut(json, maxJsonLd)}</script>\n`);
  }
  const attributesOf = (el: El) => {
    let s = "";
    for (const [name, value] of Object.entries(el.attribs)) {
      if (!KEPT_ATTRIBUTES.has(name) && !name.startsWith("data-")) continue;
      if (name.startsWith("data-") && value.length > maxAttr) continue;
      s += value === "" ? ` ${name}` : ` ${name}="${escapeAttr(cut(value, maxAttr))}"`;
    }
    return s;
  };
  const walk = (nodes: readonly Node[]) => {
    const alike = new Map<string, number>();
    let collapsed = 0;
    const flushCollapsed = () => {
      if (collapsed) emit(`<!-- ${collapsed} more alike -->`);
      collapsed = 0;
    };
    for (const n of nodes) {
      if (truncated) return;
      if (isText(n)) {
        const t = collapse(n.data);
        if (!t) continue;
        const s = escapeText(cut(t, maxText));
        if (full(s.length)) {
          truncated = true;
          return;
        }
        emit(s);
        continue;
      }
      if (!isElement(n)) continue;
      if (n.type === ElementType.Script || n.type === ElementType.Style) continue;
      if (DROPPED.has(n.name) || n.name === "title" || n.name === "meta") continue;
      if (n.attribs.hidden !== undefined) continue;
      const key = likeness(n);
      const seen = (alike.get(key) ?? 0) + 1;
      alike.set(key, seen);
      if (seen > maxAlike) {
        collapsed++;
        continue;
      }
      flushCollapsed();
      if (n.name === "html" || n.name === "body") {
        walk(n.children);
        continue;
      }
      const open = `<${n.name}${attributesOf(n)}>`;
      const close = VOID.has(n.name) ? "" : `</${n.name}>`;
      if (full(open.length + close.length)) {
        truncated = true;
        return;
      }
      emit(open);
      if (!VOID.has(n.name)) {
        walk(n.children);
        emit(close);
      }
    }
    if (!truncated) flushCollapsed();
  };
  walk(doc.children);
  const cleaned = out.join("").replace(/<(div|span|section|p|li|ul)(?: class="[^"]*")?><\/\1>/g, "");
  return { html: cleaned, truncated };
}

export function skeletonPaths(html: string): Set<string> {
  const doc = parse(html);
  const body = DomUtils.findOne((n) => n.name === "body", doc.children);
  const out = new Set<string>();
  const fold = (s: string) => s.replace(/\d+/g, "#");
  const sig = (el: El) => {
    const classes = [...new Set((el.attribs.class ?? "").split(/\s+/).filter(Boolean).map(fold))].sort();
    const id = el.attribs.id ? `#${fold(el.attribs.id)}` : "";
    return `${el.name}${id}${classes.length ? `.${classes.join(".")}` : ""}`;
  };
  const walk = (nodes: readonly Node[], prefix: string, depth: number) => {
    if (depth > 14) return;
    for (const n of nodes) {
      if (!isElement(n) || DROPPED.has(n.name)) continue;
      const path = prefix ? `${prefix}>${sig(n)}` : sig(n);
      out.add(path);
      walk(n.children, path, depth + 1);
    }
  };
  walk(body ? body.children : doc.children, "", 0);
  return out;
}

// ── Random HTML ──────────────────────────────────────────────────────────

/** A seeded PRNG (mulberry32). */
export function random(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const TAGS = [
  "div", "div", "p", "p", "span", "a", "li", "li", "ul", "ol", "table", "tr", "td", "th", "tbody", "thead", "h1", "h2",
  "dl", "dt", "dd", "b", "i", "section", "article", "form", "input", "select", "option", "br", "img", "svg", "path",
  "title", "script", "style", "noscript", "template", "button", "textarea", "main", "header", "em", "image", "math",
];
const WORDS = ["Nora", "Mira", "de", "zaal", "&amp;", "&lt;b&gt;", "&copy", "caf&eacute;", "&#233;t&#xE9;", "x", "  ", "\n", "20:00", "€25,-", "&nbsp;"];
const CLASSES = ["a", "b", "c", "x-1", "x-2", "event", "Event"];

/** A messy document: misnested and unclosed tags, stray end tags, raw text, entities. Every element carries `data-n`. */
export function messyHtml(next: () => number, size = 300): string {
  const pick = <T>(a: readonly T[]) => a[Math.floor(next() * a.length)]!;
  let n = 0;
  let out = "";
  for (let i = 0; i < size; i++) {
    const r = next();
    if (r < 0.42) {
      const tag = pick(TAGS);
      let attrs = ` data-n="${n++}"`;
      if (next() < 0.5) attrs += ` class="${pick(CLASSES)}${next() < 0.4 ? ` ${pick(CLASSES)}` : ""}"`;
      if (next() < 0.15) attrs += ` id="i${n}"`;
      if (next() < 0.2) attrs += ` href="${pick(["/e/1", "https://x.example/a", "#top", "mailto:a@b"])}"`;
      if (next() < 0.1) attrs += " hidden";
      if (next() < 0.1) attrs += " data-v";
      if (next() < 0.05) attrs += ' role="presentation"';
      const self = next() < 0.08 ? "/" : "";
      out += `<${next() < 0.1 ? tag.toUpperCase() : tag}${attrs}${self}>`;
      if (tag === "script" || tag === "style") out += `var s = "</div><p>${pick(WORDS)}</p>";</${tag}>`;
      if (tag === "title" || tag === "textarea") out += `${pick(WORDS)} &amp; <i>x</i></${tag}>`;
    } else if (r < 0.7) {
      out += `</${pick(TAGS)}>`;
    } else if (r < 0.95) {
      out += pick(WORDS) + (next() < 0.5 ? " " : "");
    } else if (r < 0.98) {
      out += "<!-- c -->";
    } else {
      out += pick(["</p>", "</br>", "<", "</ x>", "<!doctype html>", "<?x?>"]);
    }
  }
  return out;
}
