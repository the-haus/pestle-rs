# pestle-grammar

[pest](https://pest.rs) grammars compiled at run time: any grammar
`pest_derive` accepts, with no build step, run by an engine that keeps pest's
semantics (the same pairs, tags and errors as `pest_vm`) and is 3–5× faster
than `pest_derive`.

```rust
use pestle_grammar::{Grammar, ShapeOptions};

let g = Grammar::new(r#"
    time = { #hour = num ~ ":" ~ #minute = num }
    num  = @{ ASCII_DIGIT{2} }
"#).unwrap();
let v = g.shape(None, "20:30", &ShapeOptions::default()).unwrap();
assert_eq!(v, serde_json::json!({ "hour": "20", "minute": "30" }));
```

- **Trees**: `parse` gives the pairs, flat, in document order (`Tree`).
- **Values**: `shape` builds JSON from the grammar's node tags (`lists`,
  `numbers`, by rule), also straight to JSON text.
- **Search**: `find` and `find_all` locate matches inside longer text.
- **Error recovery**: named rules skip to a synchronising string and the
  parse goes on (pest has none).
- **Safety**: `set_call_limit` caps backtracking; deep recursion fails
  with an error instead of overflowing the stack.

Used by [`pestle-html`](../pestle-html/README.md)'s `grammar` feature and the
`Grammar` class of `@pestle-rs/node`'s grammar build. The full guide is in
[docs/grammars.md](../../docs/grammars.md).

## License

MIT OR Apache-2.0. `src/vm.rs` derives from `pest_vm` (MIT OR Apache-2.0).
