# Contributing

Thanks for helping. Bug reports with a page that shows the problem (or the
smallest piece of HTML that does) are the most useful thing you can send.

## Setup

```bash
git clone https://github.com/the-haus/pestle-rs.git
cd pestle-rs
corepack enable
corepack yarn install --immutable
corepack yarn workspace @pestle-rs/node build
```

You need Rust stable (`rust-toolchain.toml` pins it), a C compiler, and
Node.js 20+.

## Before you push

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude pestle-node
cargo test -p pestle-html --features grammar
cd crates/bindings/pestle-node && corepack yarn typecheck && corepack yarn test
```

CI runs the same on Linux, macOS and Windows, plus the conformance suites.

## Rules of the house

- **A fix comes with a test** that fails without it. A page that broke
  pestle goes into `crates/pestle-html/tests/edge_cases.rs` (structured
  data) or the matching test file, cut down to what matters.
- **Never throw on a page.** Every reader is bounded and reports what it
  cut in `warnings`; native entry points catch panics.
- **Festimap parity is fixed.** Text (`textOf`), skeleton and strip outputs
  stay byte for byte what festimap's pipeline gives; the differential tests
  guard them. New behaviour is a new output or an option, never a change to
  those.
- **The Node API is the TypeScript in `ts/`**; `src/` is its native half.
  A new native export goes in `ts/native.d.ts`, `ts/native.js` and
  `ts/native.esm.js` (`test/package.test.ts` checks the two loaders agree).
- **Docs move with the code**: the Node guide
  (`crates/bindings/pestle-node/README.md`), `docs/`, and a line in
  `CHANGELOG.md` under the unreleased version.

## Pull requests

Keep them focused; open an issue first for anything larger than a bug fix
or docs change. Describe what changes for users, and how you tested it.
