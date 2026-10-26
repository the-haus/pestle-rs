//! A spec applied as the document streams by.
//!
//! Every field of every open scope (the document, and each element a field
//! with `fields` matched) is tested on each element as it opens; a match
//! starts a capture that collects the element's text, attribute or markup
//! until it closes, and then becomes the field's value. A match that waits
//! on something only the rest of the document can tell (`:has()`,
//! `:last-child`, …) holds its place, so values keep document order, and
//! is settled when its element or its parent closes.
//!
//! A `closest` field that finds nothing under its scope reads the scope's
//! ancestors, nearest first: the first match under each, else the ancestor
//! itself. Every element its selector matches anywhere is read as a
//! candidate, and each open element knows where its candidates start; what
//! an ancestor holds is known for sure only when it closes (a match may yet
//! come), so until then the object waits, parked, in its place.

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

/// Where a capture's value goes.
#[derive(Clone, Copy)]
enum Target {
    /// A slot of an open instance's field.
    Slot { instance: usize, field: usize, slot: usize },
    /// A candidate of a `closest` field's track.
    Cand { track: usize, cand: usize },
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

    /// Whether an object parked inside it is still to come.
    fn waits(&self) -> bool {
        self.slots.iter().flatten().any(|s| matches!(s, Slot::Waiting))
    }
}

/// A match being read.
struct Capture {
    target: Target,
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
    /// Its text, or its markup, reached the most one value may hold: nothing more joins on.
    full: bool,
    html_full: bool,
    /// How it reads text (its field's `read`, kept here for the hot loops).
    mode: Mode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Visible,
    /// Visible text, a line per block.
    Lines,
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
    value: Held,
}

/// A value read, or an object still waiting on a `closest` field (its [`Engine::parked`] entry).
enum Held {
    Ready(Value),
    Parked(usize),
}

/// What holds the slot a parked object's value goes to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Instance(usize),
    Parked(usize),
}

/// An object whose scope closed with values still to come: a `closest` field waiting on an
/// ancestor, or an object of its own parked.
struct Parked {
    inst: Instance,
    /// How many values are still to come.
    pending: usize,
    owner: Owner,
    field: usize,
    slot: usize,
    /// No sibling condition holds it back (or it held): its value goes as soon as it is complete.
    released: bool,
    /// A condition failed, or what it goes into was dropped: its value goes nowhere.
    discarded: bool,
    /// Complete, waiting on its sibling conditions.
    done: Option<Value>,
}

/// A `closest` field waiting on the ancestor at frame `cur` to close.
struct Lookup {
    parked: usize,
    field: usize,
    track: usize,
    cur: usize,
    /// The farthest ancestor it may read (a frame).
    min: usize,
}

/// What a `closest` field's selector matched anywhere, as the page is read.
#[derive(Default)]
struct Track {
    /// Each match's value (once its element closed), in page order. When an element closes,
    /// only the first value among the matches inside it is kept: no ancestor reads past it.
    cands: Vec<Slot>,
    /// Per open element: where the candidates inside it start, and whether it is one itself
    /// (the one just before).
    frames: Vec<(usize, bool)>,
}

impl Track {
    /// The first value under the element at `frame`, when it is sure: none before it waits.
    /// None too when nothing has one (yet).
    fn first(&self, frame: usize) -> Option<&Value> {
        for slot in &self.cands[self.frames[frame].0..] {
            match slot {
                Slot::Value(v) => return Some(v),
                Slot::Waiting => return None,
                Slot::Dead => {}
            }
        }
        None
    }

    /// The element at `frame` itself, when it is a candidate with a value.
    fn own(&self, frame: usize) -> Option<&Value> {
        match self.frames[frame] {
            (start, true) => match &self.cands[start - 1] {
                Slot::Value(v) => Some(v),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The extraction pass.
pub(crate) struct Engine {
    spec: Arc<Spec>,
    instances: Vec<Instance>,
    captures: Vec<Capture>,
    waits: Vec<SiblingWait>,
    tracks: Vec<Track>,
    /// Objects waiting on a `closest` field (`None`: a free entry).
    parked: Vec<Option<Parked>>,
    free: Vec<usize>,
    /// Waiting `closest` fields, by the frame they wait on (outermost first).
    lookups: Vec<Lookup>,
    /// Lookups made while an element closes, joining `lookups` after it closed.
    fresh: Vec<Lookup>,
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
        let tracks = spec.tracks.iter().map(|_| Track::default()).collect();
        Engine {
            spec,
            instances: vec![root],
            captures: Vec::new(),
            waits: Vec::new(),
            tracks,
            parked: Vec::new(),
            free: Vec::new(),
            lookups: Vec::new(),
            fresh: Vec::new(),
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

    /// The field a capture reads for.
    fn def_of<'s>(&self, target: Target) -> &'s FieldDef {
        let spec = self.spec();
        match target {
            Target::Slot { instance, field, .. } => &spec.objects[self.instances[instance].def].fields[field],
            Target::Cand { track, .. } => {
                let t = &spec.tracks[track];
                &spec.objects[t.object].fields[t.field]
            }
        }
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
        // Every element has closed, and every lookup with it; none is left but by a bug.
        self.lookups.append(&mut self.fresh);
        while let Some(l) = self.lookups.pop() {
            self.fill(l.parked, l.field, None);
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
        let cap = self.capture(Target::Slot { instance, field, slot }, def, alts, tree, depth);
        self.captures.push(cap);
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

    /// A capture of the element at `depth` for `def`.
    fn capture(&self, target: Target, def: &FieldDef, alts: Matched, tree: &Tree, depth: usize) -> Capture {
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
        Capture {
            target,
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
            html_full: false,
            mode: match def.read {
                Read::Visible => Mode::Visible,
                Read::Lines => Mode::Lines,
                Read::Raw => Mode::Raw,
                Read::Own => Mode::Own,
                _ => Mode::None,
            },
        }
    }

    /// A capture's value (none: no value) into where it goes.
    fn put(&mut self, target: Target, value: Option<Value>) {
        match target {
            Target::Slot { instance, field, slot } => self.resolve(instance, field, slot, value),
            Target::Cand { track, cand } => {
                self.tracks[track].cands[cand] = match value {
                    Some(v) => Slot::Value(v),
                    None => Slot::Dead,
                };
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
        let def = self.def_of(cap.target);
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
                if cap.child.is_some() {
                    let dropped = self.instances.pop();
                    if dropped.is_some_and(|i| i.waits()) {
                        self.redirect(self.instances.len(), None);
                    }
                }
                self.put(cap.target, None);
                return;
            }
        }
        let held = match cap.child {
            Some(_) => {
                let child = self
                    .instances
                    .pop()
                    .expect("a capture's object is the innermost instance");
                self.complete(child, cap.target, cap.depth)
            }
            None => {
                let raw = match def.read {
                    Read::Attr(_) => cap.attr.take(),
                    Read::Html => cap.html.take(),
                    Read::Exists => Some(String::new()),
                    Read::Visible | Read::Lines | Read::Raw | Read::Own => Some(std::mem::take(&mut cap.text)),
                };
                let cx = Context {
                    base: self.base.as_ref(),
                };
                let mut errors = Vec::new();
                let v = raw.and_then(|raw| finish(def, raw, &cx, &mut errors));
                for e in errors {
                    self.error(e);
                }
                match v {
                    Some(v) => Held::Ready(v),
                    None => {
                        self.put(cap.target, None);
                        return;
                    }
                }
            }
        };
        let sel = def.selector.as_ref();
        if cap.waits && sel.is_some_and(|sel| !cap.alts.iter().any(|&a| sel.deferred(a).siblings.is_empty())) {
            let Target::Slot { instance, field, slot } = cap.target else {
                unreachable!("a closest field's selector waits on no sibling")
            };
            let frame = &tree.frames[cap.depth];
            self.waits.push(SiblingWait {
                parent: cap.depth.checked_sub(1),
                instance,
                field,
                slot,
                alts: cap.alts,
                index: frame.index,
                type_index: frame.type_index,
                name: frame.el.name().to_owned(),
                value: held,
            });
            return;
        }
        match held {
            Held::Ready(v) => self.put(cap.target, Some(v)),
            Held::Parked(p) => self.release(p),
        }
    }

    /// The object of an instance whose scope closed at frame `depth`, or, when a `closest` field
    /// or an object inside it is still to come, its entry in [`Engine::parked`].
    fn complete(&mut self, mut inst: Instance, target: Target, depth: usize) -> Held {
        let spec = self.spec();
        // Its index while it was open.
        let index = self.instances.len();
        let mut lookups: SmallVec<[Lookup; 2]> = SmallVec::new();
        for (f, def) in spec.objects[inst.def].fields.iter().enumerate() {
            let Some(c) = &def.closest else { continue };
            if depth == 0 || inst.slots[f].iter().any(|s| matches!(s, Slot::Value(_))) {
                continue;
            }
            // The parent is open: what it holds so far settles it only if it has a value.
            match self.tracks[c.track].first(depth - 1) {
                Some(v) => inst.slots[f].push(Slot::Value(v.clone())),
                None => lookups.push(Lookup {
                    parked: usize::MAX,
                    field: f,
                    track: c.track,
                    cur: depth - 1,
                    min: depth.saturating_sub(c.depth),
                }),
            }
        }
        let waiting = inst
            .slots
            .iter()
            .flatten()
            .filter(|s| matches!(s, Slot::Waiting))
            .count();
        if waiting == 0 && lookups.is_empty() {
            return Held::Ready(self.object(inst));
        }
        let Target::Slot { instance, field, slot } = target else {
            unreachable!("a candidate holds no object")
        };
        let entry = Parked {
            inst,
            pending: waiting + lookups.len(),
            owner: Owner::Instance(instance),
            field,
            slot,
            released: false,
            discarded: false,
            done: None,
        };
        let id = match self.free.pop() {
            Some(id) => {
                self.parked[id] = Some(entry);
                id
            }
            None => {
                self.parked.push(Some(entry));
                self.parked.len() - 1
            }
        };
        if waiting > 0 {
            self.redirect(index, Some(id));
        }
        for mut l in lookups {
            l.parked = id;
            self.fresh.push(l);
        }
        Held::Parked(id)
    }

    /// Parked objects that go into the instance at `index`, which closed: into its parked
    /// entry now (`to`), or nowhere.
    fn redirect(&mut self, index: usize, to: Option<usize>) {
        for p in self.parked.iter_mut().flatten() {
            if p.owner == Owner::Instance(index) {
                match to {
                    Some(id) => p.owner = Owner::Parked(id),
                    None => p.discarded = true,
                }
            }
        }
    }

    /// A parked object's conditions held: its value goes as soon as it is complete.
    fn release(&mut self, id: usize) {
        let Some(p) = self.parked[id].as_mut() else { return };
        match p.done.take() {
            Some(v) => {
                let (owner, field, slot) = (p.owner, p.field, p.slot);
                self.unpark(id);
                self.deliver(owner, field, slot, Some(v));
            }
            None => p.released = true,
        }
    }

    /// A parked object's conditions failed: its value goes nowhere.
    fn discard(&mut self, id: usize) {
        let Some(p) = self.parked[id].as_mut() else { return };
        if p.done.is_some() {
            self.unpark(id);
        } else {
            p.discarded = true;
        }
    }

    fn unpark(&mut self, id: usize) {
        self.parked[id] = None;
        self.free.push(id);
    }

    /// A value still to come of a parked object, for its field (a `closest` one).
    fn fill(&mut self, id: usize, field: usize, value: Option<Value>) {
        let Some(p) = self.parked[id].as_mut() else { return };
        if let Some(v) = value {
            p.inst.slots[field].push(Slot::Value(v));
        }
        self.arrived(id);
    }

    /// One of the values a parked object waits on arrived.
    fn arrived(&mut self, id: usize) {
        let Some(p) = self.parked[id].as_mut() else { return };
        p.pending -= 1;
        if p.pending > 0 {
            return;
        }
        let inst = std::mem::replace(
            &mut p.inst,
            Instance {
                def: 0,
                depth: None,
                slots: SmallVec::new(),
                done: SmallVec::new(),
                emitted: SmallVec::new(),
            },
        );
        let (discarded, released, owner, field, slot) = (p.discarded, p.released, p.owner, p.field, p.slot);
        let value = self.object(inst);
        if discarded {
            self.unpark(id);
        } else if released {
            self.unpark(id);
            self.deliver(owner, field, slot, Some(value));
        } else if let Some(p) = self.parked[id].as_mut() {
            p.done = Some(value);
        }
    }

    /// A parked object's value into its slot.
    fn deliver(&mut self, owner: Owner, field: usize, slot: usize, value: Option<Value>) {
        match owner {
            Owner::Instance(i) => self.resolve(i, field, slot, value),
            Owner::Parked(id) => {
                let Some(p) = self.parked[id].as_mut() else { return };
                p.inst.slots[field][slot] = match value {
                    Some(v) => Slot::Value(v),
                    None => Slot::Dead,
                };
                self.arrived(id);
            }
        }
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
        match (w.value, holds) {
            (Held::Ready(v), true) => self.resolve(w.instance, w.field, w.slot, Some(v)),
            (Held::Ready(_), false) => self.resolve(w.instance, w.field, w.slot, None),
            (Held::Parked(id), true) => self.release(id),
            (Held::Parked(id), false) => {
                self.discard(id);
                self.resolve(w.instance, w.field, w.slot, None);
            }
        }
    }

    /// The `closest` fields waiting on the element at frame `d`, which closed: settled by what
    /// it holds or is, else passed on to its parent.
    fn lookups_at(&mut self, d: usize) {
        let mut moved = Vec::new();
        while self.lookups.last().is_some_and(|l| l.cur == d) {
            let Some(mut l) = self.lookups.pop() else { break };
            let track = &self.tracks[l.track];
            let mut value = track.first(d).or_else(|| track.own(d)).cloned();
            if value.is_none() && d > l.min {
                l.cur = d - 1;
                value = self.tracks[l.track].first(d - 1).cloned();
                if value.is_none() {
                    moved.push(l);
                    continue;
                }
            }
            self.fill(l.parked, l.field, value);
        }
        // All of them wait on the parent now: still outermost first.
        self.lookups.append(&mut moved);
        self.lookups.append(&mut self.fresh);
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
            cap.text
                .push_str(crate::text::cut_bytes(text, max.saturating_sub(cap.text.len())));
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
            if matches!(cap.mode, Mode::Visible | Mode::Lines) && cap.skip.is_none() {
                if crate::tags::is_never_text(el.kind) {
                    cap.skip = Some(d);
                } else if let Some(c) = crate::tags::text_break(el.kind, cap.mode == Mode::Lines) {
                    Self::push_text(cap, if c == '\n' { "\n" } else { " " }, max);
                }
            }
            if !cap.has.is_empty() {
                checks.push(ci);
            }
        }
        // Something inside a match waiting on `:has()`.
        for ci in checks {
            let cap = &self.captures[ci];
            let def = self.def_of(cap.target);
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
        // `closest` fields' matches anywhere, and where this element's start.
        for (t, track) in spec.tracks.iter().enumerate() {
            let alts = track.selector.matches(tree, None);
            let own = !alts.is_empty();
            if own {
                let cand = self.tracks[t].cands.len();
                self.tracks[t].cands.push(Slot::Waiting);
                let def = &spec.objects[track.object].fields[track.field];
                let cap = self.capture(Target::Cand { track: t, cand }, def, alts, tree, d);
                self.captures.push(cap);
            }
            let start = self.tracks[t].cands.len();
            self.tracks[t].frames.push((start, own));
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
            } else if cap.skip.is_none() && matches!(cap.mode, Mode::Visible | Mode::Lines) {
                if let Some(c) = crate::tags::text_break(kind, cap.mode == Mode::Lines) {
                    Self::push_text(cap, if c == '\n' { "\n" } else { " " }, max);
                }
            }
        }
        if self.tracks.is_empty() {
            return;
        }
        self.lookups_at(d);
        for track in &mut self.tracks {
            let Some((start, _)) = track.frames.pop() else { continue };
            // Every match inside has its value now; past the first, no ancestor reads them.
            let first = track.cands[start..].iter().position(|s| matches!(s, Slot::Value(_)));
            if track.cands[start..].iter().all(|s| !matches!(s, Slot::Waiting)) {
                let kept = first.map(|i| std::mem::replace(&mut track.cands[start + i], Slot::Dead));
                track.cands.truncate(start);
                track.cands.extend(kept);
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
                // Its own line breaks are spaces: lines break where blocks do.
                Mode::Lines if cap.skip.is_none() => {
                    if text.contains(['\n', '\r']) {
                        Self::push_text(cap, &text.replace(['\n', '\r'], " "), max);
                    } else {
                        Self::push_text(cap, text, max);
                    }
                }
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
            if let Some(html) = cap.html.as_mut().filter(|_| !cap.html_full) {
                let raw = utf8(raw);
                if html.len() + raw.len() <= max {
                    html.push_str(&raw);
                } else {
                    // The first `max` bytes, however the page came in chunks.
                    html.push_str(crate::text::cut_bytes(&raw, max - html.len()));
                    cap.html_full = true;
                }
            }
        }
    }
}
