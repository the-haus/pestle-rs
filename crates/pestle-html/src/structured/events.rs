//! Events, whatever format a page states them in: one flat shape with the
//! fields a calendar needs, from schema.org (JSON-LD, microdata, RDFa) and
//! microformats (`h-event`, hCalendar).
//!
//! Values are as the page states them (dates and prices are not rewritten).
//! The same event stated twice (in
//! JSON-LD and in microdata, or twice in one block) is one event: events
//! with the same `@id`, or the same name and url, whose dates do not differ,
//! are merged, their formats listed in `formats`.
//!
//! ```json
//! { "name": "…", "startDate": "…", "endDate": "…", "doorTime": "…",
//!   "status": "EventScheduled", "attendanceMode": "OfflineEventAttendanceMode",
//!   "url": "…", "image": ["…"], "description": "…",
//!   "location": [{ "name", "address", "street", "locality", "region", "postalCode", "country", "lat", "lon", "url" }],
//!   "performers": [{ "name", "url", "sameAs": [], "type" }], "organizer": [{ "name", "url" }],
//!   "offers": [{ "price", "lowPrice", "highPrice", "currency", "url", "availability", "validFrom", "name" }],
//!   "keywords": ["…"], "superEvent": "…", "types": ["MusicEvent"], "format": "json-ld", "id": "…" }
//! ```

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::graph::{matches_type, Format, Graph, Render, Writer};

/// The values of a property: always a list.
fn all<'v>(o: &'v Value, key: &str) -> Vec<&'v Value> {
    match o.get(key) {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(v) => vec![v],
        None => Vec::new(),
    }
}

/// A value as text: a string, a number, an `@value`, an item's name, a link's `@id`.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Object(o) => o
            .get("@value")
            .or_else(|| o.get("name"))
            .or_else(|| o.get("value"))
            .or_else(|| o.get("@id"))
            .or_else(|| o.get("url"))
            .and_then(|x| match x {
                Value::Array(a) => a.first().and_then(text),
                x => text(x),
            }),
        Value::Array(a) => a.first().and_then(text),
        _ => None,
    }
}

fn first(o: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| all(o, k).into_iter().find_map(text))
}

fn texts(o: &Value, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for k in keys {
        for v in all(o, k) {
            if let Some(t) = text(v) {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out
}

fn put(m: &mut Map<String, Value>, k: &str, v: Option<String>) {
    if let Some(v) = v {
        m.insert(k.into(), Value::String(v));
    }
}

fn put_list(m: &mut Map<String, Value>, k: &str, v: Vec<Value>) {
    if !v.is_empty() {
        m.insert(k.into(), Value::Array(v));
    }
}

fn place(v: &Value) -> Value {
    let mut m = Map::new();
    match v {
        Value::Object(_) => {
            put(&mut m, "name", first(v, &["name", "p-name"]));
            put(&mut m, "url", first(v, &["url", "sameAs"]));
            for a in all(v, "address").into_iter().chain(all(v, "adr")) {
                match a {
                    Value::Object(_) => {
                        put(&mut m, "street", first(a, &["streetAddress", "street-address"]));
                        put(&mut m, "locality", first(a, &["addressLocality", "locality"]));
                        put(&mut m, "region", first(a, &["addressRegion", "region"]));
                        put(&mut m, "postalCode", first(a, &["postalCode", "postal-code"]));
                        put(&mut m, "country", first(a, &["addressCountry", "country-name"]));
                        let parts: Vec<String> = ["street", "postalCode", "locality", "country"]
                            .iter()
                            .filter_map(|k| m.get(*k).and_then(Value::as_str).map(str::to_owned))
                            .collect();
                        if !parts.is_empty() && !m.contains_key("address") {
                            m.insert("address".into(), json!(parts.join(", ")));
                        }
                    }
                    other => put(&mut m, "address", text(other)),
                }
            }
            // Microformats: the address fields on the item itself.
            for (k, src) in [
                ("street", "street-address"),
                ("locality", "locality"),
                ("region", "region"),
                ("postalCode", "postal-code"),
                ("country", "country-name"),
            ] {
                if !m.contains_key(k) {
                    put(&mut m, k, first(v, &[src]));
                }
            }
            for g in all(v, "geo").into_iter().chain(std::iter::once(v)) {
                put(&mut m, "lat", first(g, &["latitude"]));
                put(&mut m, "lon", first(g, &["longitude"]));
                if m.contains_key("lat") {
                    break;
                }
            }
            if !m.contains_key("name") {
                put(&mut m, "name", text(v));
            }
        }
        other => put(&mut m, "name", text(other)),
    }
    Value::Object(m)
}

fn party(v: &Value) -> Value {
    let mut m = Map::new();
    put(&mut m, "name", text(v));
    if v.is_object() {
        put(&mut m, "url", first(v, &["url"]));
        let same = texts(v, &["sameAs"]);
        if !same.is_empty() {
            m.insert("sameAs".into(), json!(same));
        }
        put(&mut m, "type", first(v, &["@type", "type"]));
    }
    Value::Object(m)
}

fn offer(v: &Value) -> Value {
    let mut m = Map::new();
    if v.is_object() {
        put(&mut m, "name", first(v, &["name"]));
        put(&mut m, "currency", first(v, &["priceCurrency"]));
        put(&mut m, "price", first(v, &["price"]));
        put(&mut m, "lowPrice", first(v, &["lowPrice"]));
        put(&mut m, "highPrice", first(v, &["highPrice"]));
        put(&mut m, "url", first(v, &["url"]));
        put(&mut m, "availability", first(v, &["availability"]));
        put(&mut m, "validFrom", first(v, &["validFrom"]));
        // A price stated in a price specification.
        if !m.contains_key("price") {
            for spec in all(v, "priceSpecification") {
                if !m.contains_key("currency") {
                    put(&mut m, "currency", first(spec, &["priceCurrency"]));
                }
                put(&mut m, "price", first(spec, &["price"]));
            }
        }
    } else {
        put(&mut m, "price", text(v));
    }
    Value::Object(m)
}

/// One event, flat.
fn event(e: &Value, format: Format) -> Value {
    let mut m = Map::new();
    let mf = format == Format::Microformats;
    put(&mut m, "name", first(e, &["name", "summary"]));
    put(
        &mut m,
        "startDate",
        first(e, if mf { &["start"] } else { &["startDate"] }),
    );
    put(&mut m, "endDate", first(e, if mf { &["end"] } else { &["endDate"] }));
    put(&mut m, "doorTime", first(e, &["doorTime"]));
    put(&mut m, "duration", first(e, &["duration"]));
    put(&mut m, "status", first(e, &["eventStatus"]));
    put(&mut m, "attendanceMode", first(e, &["eventAttendanceMode"]));
    put(&mut m, "url", first(e, &["url"]));
    let name = m.get("name").and_then(Value::as_str).map(str::to_owned);
    put(
        &mut m,
        "description",
        first(e, &["description", "summary"]).filter(|d| Some(d) != name.as_ref()),
    );
    let images: Vec<Value> = texts(e, &["image", "photo"]).into_iter().map(Value::String).collect();
    put_list(&mut m, "image", images);
    put_list(&mut m, "location", all(e, "location").into_iter().map(place).collect());
    let performers: Vec<Value> = ["performer", "performers", "attendee"]
        .iter()
        .flat_map(|k| all(e, k))
        .map(party)
        .collect();
    put_list(&mut m, "performers", performers);
    put_list(
        &mut m,
        "organizer",
        all(e, "organizer").into_iter().map(party).collect(),
    );
    put_list(&mut m, "offers", all(e, "offers").into_iter().map(offer).collect());
    let keywords: Vec<Value> = texts(e, &["keywords", "genre", "category"])
        .into_iter()
        .flat_map(|k| {
            k.split(',')
                .map(|x| x.trim().to_owned())
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
        })
        .map(Value::String)
        .collect();
    put_list(&mut m, "keywords", keywords);
    put(&mut m, "superEvent", first(e, &["superEvent"]));
    let types: Vec<Value> = all(e, "@type").into_iter().cloned().collect();
    put_list(&mut m, "types", types);
    m.insert("format".into(), json!(format.as_str()));
    put(&mut m, "id", e.get("@id").and_then(text));
    Value::Object(m)
}

/// Every event in the graph, flat; with `merge`, the same event stated twice is one.
pub fn events(graph: &Graph, base: &Render, merge: bool) -> Vec<Value> {
    let render = Render {
        arrays: true,
        detailed: false,
        types: Vec::new(),
        ..base.clone()
    };
    let writer = Writer::new(graph, &render);
    let wanted = ["Event".to_owned(), "h-event".to_owned()];
    let all = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| matches_type(n, &wanted))
        .map(|(i, n)| event(&writer.node(i, &mut Vec::new(), true), n.format));
    if !merge {
        return all.collect();
    }
    // Only events that could be the same (see `same`) are compared: the same id, the same name
    // with the same url or day, or the same url where one has no name or no date.
    let mut out: Vec<Value> = Vec::new();
    let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
    let day = |e: &Value| s(e, "startDate").map(|d| d.get(..10).unwrap_or(d).to_owned());
    let partial = |e: &Value| s(e, "name").is_none() || s(e, "startDate").is_none();
    // The keys an event is found by, and the keys to look for it with.
    let keys = |e: &Value, lookup: bool| -> Vec<String> {
        let name = s(e, "name").map(|n| n.trim().to_lowercase());
        let url = s(e, "url");
        let mut k = Vec::with_capacity(5);
        if let Some(id) = s(e, "id") {
            k.push(format!("i{id}"));
        }
        if let (Some(n), Some(u)) = (&name, url) {
            k.push(format!("nu{n}\u{0}{u}"));
        }
        if let (Some(n), Some(d)) = (&name, day(e)) {
            k.push(format!("nd{n}\u{0}{d}"));
        }
        if let Some(u) = url {
            // Every event is found by its url (`u`), a partial one also as partial (`p`). A partial
            // event looks for every event of its url, a full one only for the partial ones.
            match (lookup, partial(e)) {
                (false, false) => k.push(format!("u{u}")),
                (false, true) => k.extend([format!("u{u}"), format!("p{u}")]),
                (true, false) => k.push(format!("p{u}")),
                (true, true) => k.push(format!("u{u}")),
            }
        }
        k
    };
    for e in all {
        let mut candidates: Vec<usize> = keys(&e, true)
            .iter()
            .filter_map(|k| by_key.get(k))
            .flatten()
            .copied()
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        let at = match candidates.into_iter().find(|&c| same(&out[c], &e)) {
            Some(c) => {
                absorb(&mut out[c], e);
                c
            }
            None => {
                out.push(e);
                out.len() - 1
            }
        };
        for k in keys(&out[at], false) {
            let list = by_key.entry(k).or_default();
            if list.last() != Some(&at) {
                list.push(at);
            }
        }
    }
    // Merged events list their formats.
    for o in &mut out {
        if let Some(Value::Array(f)) = o.get("formats") {
            if f.len() < 2 {
                o.as_object_mut().map(|m| m.remove("formats"));
            }
        }
    }
    out
}

fn s<'v>(e: &'v Value, k: &str) -> Option<&'v str> {
    e.get(k).and_then(Value::as_str)
}

/// The same event: one `@id`, or one name and url (or one name, one start); dates that agree.
fn same(a: &Value, b: &Value) -> bool {
    let agree = |k: &str| match (s(a, k), s(b, k)) {
        (Some(x), Some(y)) => {
            x == y || x.get(..16) == y.get(..16) || (x.len() == 10 || y.len() == 10) && x.get(..10) == y.get(..10)
        }
        _ => true,
    };
    if !(agree("startDate") && agree("endDate")) {
        return false;
    }
    if let (Some(x), Some(y)) = (s(a, "id"), s(b, "id")) {
        return x == y;
    }
    let name = |e: &Value| s(e, "name").map(|n| n.trim().to_lowercase());
    let same_name = name(a).is_some() && name(a) == name(b);
    let same_url = s(a, "url").is_some() && s(a, "url") == s(b, "url");
    let same_start = s(a, "startDate").is_some()
        && s(a, "startDate").map(|x| &x[..x.len().min(10)]) == s(b, "startDate").map(|x| &x[..x.len().min(10)]);
    // A name and a url, a name and a day, or a url where one has no name or no date.
    (same_name && (same_url || same_start))
        || (same_url
            && (name(a).is_none() || name(b).is_none() || s(a, "startDate").is_none() || s(b, "startDate").is_none()))
}

/// What `b` says that `a` does not: missing fields, more precise dates, more list entries.
fn absorb(a: &mut Value, b: Value) {
    let (Some(am), Value::Object(bm)) = (a.as_object_mut(), b) else {
        return;
    };
    let fa = am.get("format").cloned();
    let fb = bm.get("format").cloned();
    for (k, v) in bm {
        match (am.get_mut(&k), v) {
            (None, v) => {
                am.insert(k, v);
            }
            (Some(Value::String(x)), Value::String(y)) if k.ends_with("Date") || k == "doorTime" => {
                // The one with a time (or an offset) is the more precise.
                if y.len() > x.len() && y.starts_with(x.get(..10).unwrap_or("")) {
                    *x = y;
                }
            }
            (Some(Value::Array(x)), Value::Array(y)) => {
                for v in y {
                    // An entry of the same name (a place, a performer) is one entry: its fields filled in.
                    let named = |e: &Value| e.get("name").and_then(Value::as_str).map(str::to_lowercase);
                    match x
                        .iter_mut()
                        .find(|e| **e == v || (named(e).is_some() && named(e) == named(&v)))
                    {
                        Some(Value::Object(e)) => {
                            if let Value::Object(more) = v {
                                for (k, v) in more {
                                    e.entry(k).or_insert(v);
                                }
                            }
                        }
                        Some(_) => {}
                        None => x.push(v),
                    }
                }
            }
            _ => {}
        }
    }
    let mut formats: Vec<Value> = match am.remove("formats") {
        Some(Value::Array(f)) => f,
        _ => fa.into_iter().collect(),
    };
    if let Some(f) = fb {
        if !formats.contains(&f) {
            formats.push(f);
        }
    }
    am.insert("formats".into(), Value::Array(formats));
}
