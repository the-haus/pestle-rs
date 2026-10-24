/**
 * pest grammars at run time: any grammar `pest_derive` accepts (node tags,
 * `PUSH_LITERAL`, `+` and the other `grammar-extras` included) is compiled
 * once and parses with pest's own engine, so a grammar means here what it
 * means in Rust.
 */

import * as native from "./native.js";
import type { NativeError, NativeGrammar, NativeShaper, NativeShapeOptions, NativeTree } from "./native.js";

/** A rule and its kind (`a = { }`, `_{ }`, `@{ }`, `${ }`, `!{ }`). */
export interface RuleInfo {
  readonly name: string;
  readonly kind: "normal" | "silent" | "atomic" | "compound" | "nonAtomic";
}

/** Where and why a parse failed. Offsets count UTF-16 units, as strings do. */
export interface ParseProblem {
  readonly message: string;
  readonly line: number;
  readonly col: number;
  readonly offset: number;
  readonly end?: number;
  /** The rules that would have matched there, and those that must not have. */
  readonly expected: readonly string[];
  readonly unexpected: readonly string[];
}

/** Input a grammar does not match. `message` is pest's report, pointing at the line. */
export class ParseError extends Error implements ParseProblem {
  readonly line: number;
  readonly col: number;
  readonly offset: number;
  readonly end?: number;
  readonly expected: readonly string[];
  readonly unexpected: readonly string[];

  constructor(p: NativeError) {
    super(p.message);
    this.name = "ParseError";
    this.line = p.line;
    this.col = p.col;
    this.offset = p.offset;
    if (p.end !== undefined && p.end !== null) this.end = p.end;
    this.expected = p.expected;
    this.unexpected = p.unexpected;
  }
}

/** A grammar that does not compile: every problem pest found. */
export class GrammarError extends Error {
  readonly errors: readonly ParseProblem[];
  /** The grammar's name, when an extractor was given it by name. */
  readonly grammar?: string;

  constructor(errors: readonly ParseProblem[], grammar?: string) {
    super((grammar ? `grammar ${JSON.stringify(grammar)}:\n` : "") + errors.map((e) => e.message).join("\n\n"));
    this.name = "GrammarError";
    this.errors = errors;
    if (grammar !== undefined) this.grammar = grammar;
  }
}

/** A shaper's failure (`PARSE:{error}`) as a {@link ParseError}. */
function parseFailure(e: unknown): unknown {
  const message = e instanceof Error ? e.message : "";
  return message.startsWith("PARSE:") ? new ParseError(JSON.parse(message.slice(6)) as NativeError) : e;
}

/** Native errors are `<KIND>:<json>`; anything else is rethrown as it is. */
export function rethrow(e: unknown): never {
  const message = e instanceof Error ? e.message : String(e);
  const colon = message.indexOf(":");
  const kind = message.slice(0, colon);
  if (kind === "GRAMMAR") {
    const payload = JSON.parse(message.slice(colon + 1)) as ParseProblem[] | { name: string; errors: ParseProblem[] };
    throw Array.isArray(payload) ? new GrammarError(payload) : new GrammarError(payload.errors, payload.name);
  }
  if (kind === "SPEC") {
    throw new SpecError(JSON.parse(message.slice(colon + 1)) as string[]);
  }
  throw e;
}

/** A spec that does not compile: each problem with its field (`events[].title: selector: …`). */
export class SpecError extends Error {
  readonly errors: readonly string[];

  constructor(errors: readonly string[]) {
    super(errors.join("\n"));
    this.name = "SpecError";
    this.errors = errors;
  }
}

/** How {@link Grammar.shape} builds its value. */
export interface ShapeOptions {
  /** The rule to start from (default: the grammar's first). */
  rule?: string;
  /** Tags whose value is always an array, even of one. */
  lists?: readonly string[];
  /** Tags read as numbers (`25,50` is 25.5, `€ 1.250,-` 1250); null when none. */
  numbers?: readonly string[];
  /** Key every pair by its rule name instead of by tag (no tags needed). */
  byRule?: boolean;
  /** Trim whitespace off every text. */
  trim?: boolean;
}

/**
 * Panic-mode error recovery: a rule named here that breaks after matching
 * something skips to the next synchronising string; an `ERROR` pair stands
 * for what it skipped, and the parse goes on.
 */
export interface Recovery {
  /** Rules that may recover (typically the repeated item: a statement, an act, a row). */
  rules: readonly string[];
  /** Strings to pick up again at (`";"`, `"\n"`, `"</li>"`). */
  sync: readonly string[];
  /** Skip the synchronising string too (default: leave it to the parse). */
  consume?: boolean;
}

/** One place a recovering parse skipped. Offsets count UTF-16 units. */
export interface Recovered {
  readonly rule: string;
  /** Where the rule started, where it broke (how far it matched), where the skip ended. */
  readonly start: number;
  readonly at: number;
  readonly end: number;
  /** What pest says the rule expected there. */
  readonly error: ParseProblem;
}

/** One match of {@link Grammar.find}. */
export interface Match {
  readonly start: number;
  readonly end: number;
  readonly text: string;
  readonly tree: ParseTree;
}

const NO_TAG = 0xffffffff;
const STRIDE = 5;

/**
 * A compiled grammar.
 *
 * ```ts
 * const time = new Grammar(`
 *   time = { #hour = num ~ ":" ~ #minute = num }
 *   num  = @{ ASCII_DIGIT{2} }
 * `);
 * time.shape("20:30"); // { hour: "20", minute: "30" }
 * ```
 */
export class Grammar {
  readonly source: string;
  /** The rules, in the order the grammar defines them. */
  readonly rules: readonly RuleInfo[];
  /** Every node tag the grammar uses. */
  readonly tags: readonly string[];
  /** The rule a parse starts from when none is named. */
  readonly defaultRule: string | null;
  /** @internal */
  readonly native: NativeGrammar;
  /** @internal Rule names by id: the rules, then `EOI`. */
  readonly names: readonly string[];

  constructor(source: string) {
    this.source = source;
    requireGrammars();
    try {
      this.native = new native.NativeGrammar(source);
    } catch (e) {
      rethrow(e);
    }
    this.rules = this.native.rules;
    this.names = this.native.names;
    this.tags = this.native.tags;
    this.defaultRule = this.native.defaultRule ?? null;
  }

  /**
   * Parses `input` from its start. Like pest, a rule need not read all of
   * it unless it ends with `EOI`; {@link ParseTree.end} says how far it read.
   * @throws {ParseError}
   */
  parse(input: string, rule?: string): ParseTree;
  /**
   * With `recover`: a tree even through errors, the places it recovered in
   * `tree.recovered` (`ERROR` pairs in the tree stand for what was skipped).
   * Throws only when the parse cannot complete even so.
   */
  parse(input: string, options: { rule?: string; recover?: Recovery }): ParseTree;
  parse(input: string, ruleOrOptions?: string | { rule?: string; recover?: Recovery }): ParseTree {
    const options = typeof ruleOrOptions === "string" ? { rule: ruleOrOptions } : (ruleOrOptions ?? {});
    if (options.recover) {
      const { rules, sync, consume } = options.recover;
      const r = this.native.parseRecovering(input, options.rule ?? null, {
        rules: [...rules],
        sync: [...sync],
        ...(consume !== undefined && { consume }),
      });
      if (r.error) throw new ParseError(r.error);
      return new ParseTree(this, input, r.tree!, r.recovered);
    }
    const r = this.native.parse(input, options.rule ?? null);
    if (r.error) throw new ParseError(r.error);
    return new ParseTree(this, input, r.tree!);
  }

  /** Whether `rule` matches at the start of `input`. */
  matches(input: string, rule?: string): boolean {
    return !this.native.parse(input, rule ?? null).error;
  }

  /**
   * Parses `input` and returns the value its node tags shape: a pair with
   * tagged pairs beneath it is an object of them by tag (a tag met twice an
   * array), one without is its text.
   * @throws {ParseError}
   */
  shape<T = unknown>(input: string, options?: ShapeOptions): T {
    const shaper = this.shaper(options);
    try {
      return JSON.parse(shaper.json(input)) as T;
    } catch (e) {
      throw parseFailure(e);
    }
  }

  /**
   * Many inputs shaped in one call into Rust (the fastest way to read many
   * values): each one's value, or `null` when it does not parse.
   */
  shapeAll<T = unknown>(inputs: readonly string[], options?: ShapeOptions): (T | null)[] {
    return JSON.parse(this.shaper(options).jsonAll(inputs as string[])) as (T | null)[];
  }

  #defaultShaper?: NativeShaper;
  #shapers = new WeakMap<ShapeOptions, NativeShaper>();

  /** Shape options compiled once per options object. */
  private shaper(options: ShapeOptions | undefined): NativeShaper {
    if (!options) return (this.#defaultShaper ??= this.native.shaper(null));
    let s = this.#shapers.get(options);
    if (!s) {
      const o: NativeShapeOptions = {};
      if (options.rule !== undefined) o.rule = options.rule;
      if (options.lists) o.lists = [...options.lists];
      if (options.numbers) o.numbers = [...options.numbers];
      if (options.byRule !== undefined) o.byRule = options.byRule;
      if (options.trim !== undefined) o.trim = options.trim;
      s = this.native.shaper(o);
      this.#shapers.set(options, s);
    }
    return s;
  }

  /** The first place `rule` matches in `input` (a search, as a regex finds). */
  find(input: string, rule?: string): Match | null {
    const f = this.native.find(input, rule ?? null);
    return f ? this.match(input, f.start, f.end, f.tree) : null;
  }

  /** Every match of `rule` in `input`, left to right, none overlapping. */
  findAll(input: string, options: { rule?: string; limit?: number } = {}): Match[] {
    return this.native
      .findAll(input, options.rule ?? null, options.limit ?? null)
      .map((f) => this.match(input, f.start, f.end, f.tree));
  }

  private match(input: string, start: number, end: number, tree: NativeTree): Match {
    return { start, end, text: input.slice(start, end), tree: new ParseTree(this, input, tree) };
  }
}

/** The pairs of one parse, in document order. */
export class ParseTree implements Iterable<Pair> {
  readonly input: string;
  /** How far the parse read (UTF-16 units). */
  readonly end: number;
  /** @internal */
  readonly grammar: Grammar;
  /** @internal */
  readonly nodes: Uint32Array;
  /** Where a recovering parse skipped (empty without `recover`). */
  readonly recovered: readonly Recovered[];

  constructor(grammar: Grammar, input: string, tree: NativeTree, recovered: readonly Recovered[] = []) {
    this.grammar = grammar;
    this.input = input;
    this.end = tree.end;
    this.nodes = tree.nodes;
    this.recovered = recovered;
  }

  /** How many pairs, at any depth. */
  get size(): number {
    return this.nodes.length / STRIDE;
  }

  /** The top-level pairs. */
  get pairs(): Pair[] {
    return siblings(this, 0, this.size);
  }

  [Symbol.iterator](): Iterator<Pair> {
    return this.pairs[Symbol.iterator]();
  }

  /** Every pair, a pair before the pairs inside it. */
  flatten(): Pair[] {
    return Array.from({ length: this.size }, (_, i) => new Pair(this, i));
  }

  /** The first pair tagged `tag`. */
  find(tag: string): Pair | undefined {
    return this.flatten().find((p) => p.tag === tag);
  }

  /** Every pair tagged `tag`. */
  findAll(tag: string): Pair[] {
    return this.flatten().filter((p) => p.tag === tag);
  }

  toJSON(): PairJSON[] {
    return this.pairs.map((p) => p.toJSON());
  }
}

/** A pair as JSON. */
export interface PairJSON {
  rule: string;
  tag?: string;
  text: string;
  start: number;
  end: number;
  children?: PairJSON[];
}

function siblings(tree: ParseTree, from: number, until: number): Pair[] {
  const out: Pair[] = [];
  for (let i = from; i < until; i += 1 + tree.nodes[i * STRIDE + 4]!) out.push(new Pair(tree, i));
  return out;
}

/** One matched rule: pest's `Pair`. */
export class Pair {
  /** @internal */
  readonly tree: ParseTree;
  /** @internal */
  readonly index: number;

  constructor(tree: ParseTree, index: number) {
    this.tree = tree;
    this.index = index;
  }

  private at(field: number): number {
    return this.tree.nodes[this.index * STRIDE + field]!;
  }

  get rule(): string {
    return this.tree.grammar.names[this.at(0)]!;
  }

  /** Its node tag (`#name = …`), or null. */
  get tag(): string | null {
    const t = this.at(1);
    return t === NO_TAG ? null : this.tree.grammar.tags[t]!;
  }

  get start(): number {
    return this.at(2);
  }

  get end(): number {
    return this.at(3);
  }

  get text(): string {
    return this.tree.input.slice(this.start, this.end);
  }

  /** The pairs directly inside it. */
  get children(): Pair[] {
    return siblings(this.tree, this.index + 1, this.index + 1 + this.at(4));
  }

  /** Every pair inside it, at any depth. */
  descendants(): Pair[] {
    return Array.from({ length: this.at(4) }, (_, i) => new Pair(this.tree, this.index + 1 + i));
  }

  /** The first pair inside it tagged `tag`. */
  find(tag: string): Pair | undefined {
    return this.descendants().find((p) => p.tag === tag);
  }

  findAll(tag: string): Pair[] {
    return this.descendants().filter((p) => p.tag === tag);
  }

  /** 1-based line and column (in characters) where it starts. */
  lineCol(): [number, number] {
    const before = this.tree.input.slice(0, this.start);
    const lines = before.split(/\r\n|\r|\n/);
    return [lines.length, [...lines[lines.length - 1]!].length + 1];
  }

  toJSON(): PairJSON {
    const children = this.children.map((c) => c.toJSON());
    const tag = this.tag;
    return {
      rule: this.rule,
      ...(tag !== null && { tag }),
      text: this.text,
      start: this.start,
      end: this.end,
      ...(children.length > 0 && { children }),
    };
  }
}

/**
 * Caps every parse, process-wide, at this many parser calls, so a
 * pathological grammar or input fails ("call limit reached") instead of
 * backtracking for ever. `null` lifts the cap. Deep recursion is always
 * guarded: a parse that would run out of stack fails, it does not crash.
 */
export function setCallLimit(limit: number | null): void {
  requireGrammars();
  native.setCallLimit(limit);
}

/**
 * Whether this build of the native addon reads pest grammars. The default
 * build does not (fields keep values as written, or read them with `number`,
 * `json` or `pattern`); one built with `yarn build:grammar` (cargo feature
 * `grammar`) does.
 */
export function hasGrammars(): boolean {
  return typeof native.hasGrammars === "function" ? native.hasGrammars() : typeof native.NativeGrammar === "function";
}

function requireGrammars(): void {
  if (!hasGrammars()) {
    throw new Error(
      "pestle: this build reads no pest grammars (built without the `grammar` feature). " +
        "Read values with a field's `number`, `json` or `pattern`, or build with `yarn build:grammar`.",
    );
  }
}
