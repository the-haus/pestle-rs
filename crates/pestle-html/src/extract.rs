//! A spec applied as the document streams by.
//!
//! Every field of every open scope (the document, and each element a field
//! with `fields` matched) is tested on each element as it opens; a match
//! starts a capture that collects the element's text, attribute or markup
//! until it closes, and then becomes the field's value. A match that waits
//! on something only the rest of the document can tell (`:has()`,
//! `:last-child`, …) holds its place, so values keep document order, and
//! is settled when its element or its parent closes.

use std::sync::Arc;

use crate::out::Value;
use smallvec::SmallVec;
use url::Url;

use crate::decode::utf8;
use crate::selector::Matched;
use crate::spec::{FieldDef, Read, Spec};
use crate::tree::{Handler, Text, Tree};
use crate::value::{finish, Context};

enum Slot {
    Waiting,
    Value(Value),
    Dead,
}

/// One open scope: an object being filled.
struct Instance {
    def: usize,
    /// The scope element's frame; `None` for the document.
    depth: Option<usize>,
    slots: SmallVec<[SmallVec<[Slot; 1]>; 4]>,
    /// Fields that need no more matches.
    done: SmallVec<[bool; 8]>,
    /// Emitted fields: how many slots were handed over.
    emitted: SmallVec<[usize; 4]>,
}

impl Instance {
    fn new(spec: &Spec, def: usize, depth: Option<usize>) -> Instance {
        let n = spec.objects[def].fields.len();
        Instance {
            def,
            depth,
            slots: (0..n).map(|_| SmallVec::new()).collect(),
            done: smallvec::smallvec![false; n],
            emitted: smallvec::smallvec![0; n],
        }
    }
}

/// A match being read.
struct Capture {
    instance: usize,
    field: usize,
    slot: usize,
    /// The matched element's frame.
    depth: usize,
    text: String,
    /// Inside an element whose text is never shown (its frame).
    skip: Option<usize>,
    attr: Option<String>,
    html: Option<String>,
    /// The alternatives that matched, when some condition waits.
    alts: Matched,
    waits: bool,
    /// Per alternative, per `:has()`: whether something inside matched.
    has: Vec<Vec<bool>>,
    /// All its text, for `:contains()`.
    all_text: Option<String>,
    nonempty: bool,
    /// The instance its object fills.
    child: Option<usize>,
    full: bool,
    /// How it reads text (its field's `read`, kept here for the hot loops).
    mode: Mode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Visible,
    Raw,
    Own,
    /// No text (an attribute, markup, existence).
    None,
}

/// A value waiting for its parent to close (`:last-child` and the like).
struct SiblingWait {
    parent: Option<usize>,
    instance: usize,
    field: usize,
    slot: usize,
    alts: Matched,
    index: u32,
    type_index: u32,
    name: String,
    value: Value,
}

/// The extraction pass.
pub(crate) struct Engine {
    spec: Arc<Spec>,
    instances: Vec<Instance>,
    captures: Vec<Capture>,
    waits: Vec<SiblingWait>,
    page: Option<Url>,
    base: Option<Url>,
    base_seen: bool,
    pub(crate) errors: Vec<String>,
    emitted: Vec<(String, Value)>,
    /// The last token's raw markup, while some field reads HTML.
    last_markup: Option<Vec<u8>>,
    max_capture: usize,
    pub(crate) matches: u64,
    /// Some top-level field emits its items.
    emits: bool,
}

const MAX_ERRORS: usize = 100;

impl Engine {
    pub(crate) fn new(spec: Arc<Spec>, page: Option<Url>, max_capture: usize) -> Engine {
        let reads_html = spec
            .objects
            .iter()
            .flat_map(|o| &o.fields)
            .any(|f| matches!(f.read, Read::Html));
        let root = Instance::new(&spec, 0, None);
        let emits = spec.objects[0].fields.iter().any(|f| f.emit);
        Engine {
            spec,
            instances: vec![root],
            captures: Vec::new(),
            waits: Vec::new(),
            base: page.clone(),
            page,
            base_seen: false,
            errors: Vec::new(),
            emitted: Vec::new(),
            last_markup: reads_html.then(Vec::new),
            max_capture,
            matches: 0,
            emits,
        }
    }

    /// The spec, borrowed apart from `self` so the engine's state can change while it is read.
    fn spec<'s>(&self) -> &'s Spec {
        // SAFETY: the engine holds this `Arc` for its whole life and never replaces it, so the
        // `Spec` outlives every borrow handed out here; the `Spec` itself is never mutated.
        unsafe { &*Arc::as_ptr(&self.spec) }
    }

    fn field(&self, instance: usize, field: usize) -> &FieldDef {
        &self.spec.objects[self.instances[instance].def].fields[field]
    }

    /// Items of emitted fields completed since the last call.
    pub(crate) fn take(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.emitted)
    }

    /// After the document's end: the top-level object.
    pub(crate) fn finish(&mut self, tree: &Tree) -> Value {
        while let Some(w) = self.waits.pop() {
            self.settle(w, tree);
        }
        self.drain_emitted();
        let root = self.instances.swap_remove(0);
        self.object(root)
    }

    fn start(&mut self, instance: usize, field: usize, alts: Matched, tree: &Tree, depth: usize) {
        self.matches += 1;
        let spec = self.spec();
        let def = &spec.objects[self.instances[instance].def].fields[field];
        let inst = &mut self.instances[instance];
        let slot = inst.slots[field].len();
        inst.slots[field].push(Slot::Waiting);
        // A limit counts the matches not yet ruled out (only when there is one: counting is linear).
        if def.all
            && def.limit != usize::MAX
            && inst.slots[field].iter().filter(|s| !matches!(s, Slot::Dead)).count() >= def.limit
        {
            inst.done[field] = true;
        }
        let waits = def
            .selector
            .as_ref()
            .is_some_and(|sel| !alts.iter().any(|&a| sel.deferred(a).is_empty()));
        let has = match (&def.selector, waits) {
            (Some(sel), true) => alts.iter().map(|&a| vec![false; sel.deferred(a).has.len()]).collect(),
            _ => Vec::new(),
        };
        let needs_all_text = waits
            && def
                .selector
                .as_ref()
                .is_some_and(|sel| alts.iter().any(|&a| !sel.deferred(a).contains.is_empty()));
        let el = &tree.frames[depth].el;
        self.captures.push(Capture {
            instance,
            field,
            slot,
            depth,
            text: String::new(),
            skip: None,
            attr: match &def.read {
                Read::Attr(a) => el.attr(a).map(str::to_owned),
                _ => None,
            },
            html: match (&def.read, &self.last_markup) {
                (Read::Html, Some(raw)) => Some(utf8(raw).into_owned()),
                _ => None,
            },
            alts: if waits { alts } else { Matched::new() },
            waits,
            has,
            all_text: needs_all_text.then(String::new),
            nonempty: false,
            child: None,
            full: false,
            mode: match def.read {
                Read::Visible => Mode::Visible,
                Read::Raw => Mode::Raw,
                Read::Own => Mode::Own,
                _ => Mode::None,
            },
        });
        if let Some(object) = def.object {
            let child = self.instances.len();
            self.instances.push(Instance::new(spec, object, Some(depth)));
            if let Some(cap) = self.captures.last_mut() {
                cap.child = Some(child);
            }
            // Fields of the element itself.
            for (i, f) in spec.objects[object].fields.iter().enumerate() {
                if f.selector.is_none() {
                    self.start(child, i, Matched::new(), tree, depth);
                }
            }
        }
    }

    fn resolve(&mut self, instance: usize, field: usize, slot: usize, value: Option<Value>) {
        let all = self.field(instance, field).all;
        let inst = &mut self.instances[instance];
        inst.slots[field][slot] = match value {
            Some(v) => Slot::Value(v),
            None => Slot::Dead,
        };
        // The first value found ends a single-valued field.
        if !all {
            if let Some(first) = inst.slots[field].iter().find(|s| !matches!(s, Slot::Dead)) {
                if matches!(first, Slot::Value(_)) {
                    inst.done[field] = true;
                }
            }
        }
        if instance == 0 {
            self.drain_emitted();
        }
    }

    fn drain_emitted(&mut self) {
        if !self.emits {
            return;
        }
        let Some(root) = self.instances.first_mut() else {
            return;
        };
        if root.depth.is_some() {
            return;
        }
        for (i, f) in self.spec.objects[root.def].fields.iter().enumerate() {
            if !f.emit {
                continue;
            }
            let slots = &mut root.slots[i];
            while root.emitted[i] < slots.len() {
                match std::mem::replace(&mut slots[root.emitted[i]], Slot::Dead) {
                    Slot::Value(v) => self.emitted.push((f.name.clone(), v)),
                    Slot::Dead => {}
                    Slot::Waiting => {
                        slots[root.emitted[i]] = Slot::Waiting;
                        break;
                    }
                }
                root.emitted[i] += 1;
            }
        }
    }

    fn object(&self, inst: Instance) -> Value {
        let def = &self.spec.objects[inst.def];
        let mut out = Vec::with_capacity(def.keys.len());
        for (f, slots) in def.fields.iter().zip(inst.slots) {
            if f.emit {
                continue;
            }
            let mut values = slots.into_iter().filter_map(|s| match s {
                Slot::Value(v) => Some(v),
                _ => None,
            });
            let value = if f.all {
                let items: Vec<Value> = values.take(f.limit).collect();
                match (&f.default, items.is_empty()) {
                    (Some(d), true) => Value::from(d.clone()),
                    _ => Value::Array(items),
                }
            } else {
                values
                    .next()
                    .or_else(|| f.default.clone().map(Value::from))
                    .unwrap_or(match f.read {
                        Read::Exists => Value::Bool(false),
                        _ => Value::Null,
                    })
            };
            out.push(value);
        }
        Value::Record(def.keys.clone(), out.into_boxed_slice())
    }

    fn close_capture(&mut self, mut cap: Capture, tree: &Tree) {
        let spec = self.spec();
        let def = &spec.objects[self.instances[cap.instance].def].fields[cap.field];
        let object = cap.child.map(|_| {
            let child = self
                .instances
                .pop()
                .expect("a capture's object is the innermost instance");
            self.object(child)
        });
        if cap.waits {
            let sel = def.selector.as_ref().expect("only a selector waits");
            let alts = std::mem::take(&mut cap.alts);
            cap.alts = alts
                .into_iter()
                .enumerate()
                .filter(|&(i, a)| {
                    let d = sel.deferred(a);
                    cap.has.get(i).is_none_or(|flags| flags.iter().all(|&f| f))
                        && (!d.empty || !cap.nonempty)
                        && d.contains.iter().all(|(text, ci)| {
                            let all = cap.all_text.as_deref().unwrap_or("");
                            if *ci {
                                all.to_lowercase().contains(&text.to_lowercase())
                            } else {
                                all.contains(text.as_str())
                            }
                        })
                })
                .map(|(_, a)| a)
                .collect();
            if cap.alts.is_empty() {
                self.resolve(cap.instance, cap.field, cap.slot, None);
                return;
            }
        }
        let value = match object {
            Some(o) => Some(o),
            None => {
                let raw = match def.read {
                    Read::Attr(_) => cap.attr.take(),
                    Read::Html => cap.html.take(),
                    Read::Exists => Some(String::new()),
                    Read::Visible | Read::Raw | Read::Own => Some(std::mem::take(&mut cap.text)),
                };
                let cx = Context {
                    base: self.base.as_ref(),
                };
                let mut errors = Vec::new();
                let v = raw.and_then(|raw| finish(def, raw, &cx, &mut errors));
                for e in errors {
                    self.error(e);
                }
                v
            }
        };
        let Some(value) = value else {
            self.resolve(cap.instance, cap.field, cap.slot, None);
            return;
        };
        let sel = def.selector.as_ref();
        if cap.waits && sel.is_some_and(|sel| !cap.alts.iter().any(|&a| sel.deferred(a).siblings.is_empty())) {
            let frame = &tree.frames[cap.depth];
            self.waits.push(SiblingWait {
                parent: cap.depth.checked_sub(1),
                instance: cap.instance,
                field: cap.field,
                slot: cap.slot,
                alts: cap.alts,
                index: frame.index,
                type_index: frame.type_index,
                name: frame.el.name().to_owned(),
                value,
            });
            return;
        }
        self.resolve(cap.instance, cap.field, cap.slot, Some(value));
    }

    fn settle(&mut self, w: SiblingWait, tree: &Tree) {
        let spec = self.spec();
        let def = &spec.objects[self.instances[w.instance].def].fields[w.field];
        let siblings = tree.children_of(w.parent);
        let (count, type_count) = (siblings.count, siblings.of_type(&w.name));
        let holds = def.selector.as_ref().is_some_and(|sel| {
            w.alts.iter().any(|&a| {
                sel.deferred(a)
                    .siblings
                    .iter()
                    .all(|c| c.holds(w.index, w.type_index, count, type_count))
            })
        });
        self.resolve(w.instance, w.field, w.slot, holds.then_some(w.value));
    }

    fn error(&mut self, e: String) {
        if self.errors.len() < MAX_ERRORS && !self.errors.contains(&e) {
            self.errors.push(e);
        }
    }

    fn push_text(cap: &mut Capture, text: &str, max: usize) {
        if cap.full {
            return;
        }
        if cap.text.len() + text.len() > max {
            let room = crate::text::cut(text, max.saturating_sub(cap.text.len())).len();
            cap.text.push_str(&text[..room.min(text.len())]);
            cap.full = true;
        } else {
            cap.text.push_str(text);
        }
    }
}

impl Engine {
    /// Whether text and markup matter right now (something captures, or reads markup).
    pub(crate) fn wants_text(&self) -> bool {
        !self.captures.is_empty() || self.last_markup.is_some()
    }
}

impl Handler for Engine {
    fn open(&mut self, tree: &Tree) {
        let d = tree.frames.len() - 1;
        let el = &tree.frames[d].el;
        if !self.base_seen && el.kind == crate::tags::BASE {
            if let Some(href) = el.attr("href") {
                self.base_seen = true;
                let base = match &self.page {
                    Some(page) => page.join(href.trim()).ok(),
                    None => Url::parse(href.trim()).ok(),
                };
                if base.is_some() {
                    self.base = base;
                }
            }
        }
        let spec = self.spec();
        let max = self.max_capture;
        let mut checks = Vec::new();
        for (ci, cap) in self.captures.iter_mut().enumerate() {
            cap.nonempty = true;
            if cap.mode == Mode::Visible && cap.skip.is_none() {
                if crate::tags::is_never_text(el.kind) {
                    cap.skip = Some(d);
                } else if !crate::tags::is_inline(el.kind) {
                    Self::push_text(cap, " ", max);
                }
            }
            if !cap.has.is_empty() {
                checks.push(ci);
            }
        }
        // Something inside a match waiting on `:has()`.
        for ci in checks {
            let cap = &self.captures[ci];
            let def = &spec.objects[self.instances[cap.instance].def].fields[cap.field];
            let Some(sel) = &def.selector else { continue };
            let mut found = Vec::new();
            for (i, &a) in cap.alts.iter().enumerate() {
                for (h, inner) in sel.deferred(a).has.iter().enumerate() {
                    if !cap.has[i][h] && inner.matches_now(tree, Some(cap.depth)) {
                        found.push((i, h));
                    }
                }
            }
            for (i, h) in found {
                self.captures[ci].has[i][h] = true;
            }
        }
        let open = self.instances.len();
        let el = &tree.frames[d].el;
        for i in 0..open {
            let def = &spec.objects[self.instances[i].def];
            // Only the fields whose selector can select this tag (and those that can select any).
            let (any, tagged) = def.candidates(el);
            for &f in any.iter().chain(tagged) {
                if self.instances[i].done[f] {
                    continue;
                }
                let Some(sel) = &def.fields[f].selector else {
                    continue;
                };
                let alts = sel.matches(tree, self.instances[i].depth);
                if !alts.is_empty() {
                    self.start(i, f, alts, tree, d);
                }
            }
        }
    }

    fn close(&mut self, tree: &Tree) {
        let d = tree.frames.len() - 1;
        while self.waits.last().is_some_and(|w| w.parent == Some(d)) {
            let w = self.waits.pop().expect("checked");
            self.settle(w, tree);
        }
        while self.captures.last().is_some_and(|c| c.depth == d) {
            let cap = self.captures.pop().expect("checked");
            self.close_capture(cap, tree);
        }
        let kind = tree.frames[d].el.kind;
        let max = self.max_capture;
        for cap in &mut self.captures {
            if cap.skip == Some(d) {
                cap.skip = None;
            } else if cap.skip.is_none() && cap.mode == Mode::Visible && !crate::tags::is_inline(kind) {
                Self::push_text(cap, " ", max);
            }
        }
    }

    fn text(&mut self, tree: &Tree, text: &Text<'_>) {
        if self.captures.is_empty() {
            return;
        }
        let text = text.get();
        let d = tree.frames.len().wrapping_sub(1);
        let meaningful = !text.trim_matches([' ', '\t', '\r', '\n']).is_empty();
        let max = self.max_capture;
        for cap in &mut self.captures {
            match cap.mode {
                Mode::Visible if cap.skip.is_none() => Self::push_text(cap, text, max),
                Mode::Raw => Self::push_text(cap, text, max),
                Mode::Own if d == cap.depth => Self::push_text(cap, text, max),
                _ => {}
            }
            if let Some(all) = &mut cap.all_text {
                if all.len() < max {
                    all.push_str(text);
                }
            }
            if meaningful {
                cap.nonempty = true;
            }
        }
    }

    fn markup(&mut self, _: &Tree, raw: &[u8]) {
        let Some(last) = &mut self.last_markup else {
            return;
        };
        last.clear();
        last.extend_from_slice(raw);
        let max = self.max_capture;
        for cap in &mut self.captures {
            if let Some(html) = &mut cap.html {
                if html.len() + raw.len() <= max {
                    html.push_str(&utf8(raw));
                } else {
                    cap.full = true;
                }
            }
        }
    }
}
