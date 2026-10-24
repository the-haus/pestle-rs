//! Vocabularies: schema.org recognised however a page writes it, the
//! RDFa initial context's prefixes, and names put back in short form.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::schema_org::{MEMBERS, PROPERTIES, TYPES};

pub const SCHEMA: &str = "https://schema.org/";
pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// The prefixes of the RDFa 1.1 initial context (https://www.w3.org/2011/rdfa-context/rdfa-1.1).
pub const PREFIXES: &[(&str, &str)] = &[
    ("as", "https://www.w3.org/ns/activitystreams#"),
    ("cat", "http://www.w3.org/ns/dcat#"),
    ("cc", "http://creativecommons.org/ns#"),
    ("cnt", "http://www.w3.org/2008/content#"),
    ("csvw", "http://www.w3.org/ns/csvw#"),
    ("ctag", "http://commontag.org/ns#"),
    ("dc", "http://purl.org/dc/terms/"),
    ("dc11", "http://purl.org/dc/elements/1.1/"),
    ("dcat", "http://www.w3.org/ns/dcat#"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("dqv", "http://www.w3.org/ns/dqv#"),
    ("duv", "https://www.w3.org/ns/duv#"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("gr", "http://purl.org/goodrelations/v1#"),
    ("grddl", "http://www.w3.org/2003/g/data-view#"),
    ("ical", "http://www.w3.org/2002/12/cal/icaltzd#"),
    ("jsonld", "http://www.w3.org/ns/json-ld#"),
    ("ldp", "http://www.w3.org/ns/ldp#"),
    ("ma", "http://www.w3.org/ns/ma-ont#"),
    ("oa", "http://www.w3.org/ns/oa#"),
    ("odrl", "http://www.w3.org/ns/odrl/2/"),
    ("og", "http://ogp.me/ns#"),
    ("org", "http://www.w3.org/ns/org#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("qb", "http://purl.org/linked-data/cube#"),
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfa", "http://www.w3.org/ns/rdfa#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("rev", "http://purl.org/stuff/rev#"),
    ("rif", "http://www.w3.org/2007/rif#"),
    ("rr", "http://www.w3.org/ns/r2rml#"),
    ("schema", "http://schema.org/"),
    ("sd", "http://www.w3.org/ns/sparql-service-description#"),
    ("sioc", "http://rdfs.org/sioc/ns#"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("skosxl", "http://www.w3.org/2008/05/skos-xl#"),
    ("sosa", "http://www.w3.org/ns/sosa/"),
    ("ssn", "http://www.w3.org/ns/ssn/"),
    ("time", "http://www.w3.org/2006/time#"),
    ("v", "http://rdf.data-vocabulary.org/#"),
    ("vcard", "http://www.w3.org/2006/vcard/ns#"),
    ("void", "http://rdfs.org/ns/void#"),
    ("wdr", "http://www.w3.org/2007/05/powder#"),
    ("wdrs", "http://www.w3.org/2007/05/powder-s#"),
    ("xhv", "http://www.w3.org/1999/xhtml/vocab#"),
    ("xml", "http://www.w3.org/XML/1998/namespace"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
];

/// Short names for output, beyond the initial context (Facebook's and Twitter's).
const OUTPUT_PREFIXES: &[(&str, &str)] = &[
    ("og", "http://ogp.me/ns#"),
    // OpenGraph as pages often write it; read as the namespace it means.
    ("og", "https://ogp.me/ns#"),
    ("fb", "http://ogp.me/ns/fb#"),
    ("article", "http://ogp.me/ns/article#"),
    ("music", "http://ogp.me/ns/music#"),
    ("video", "http://ogp.me/ns/video#"),
    ("book", "http://ogp.me/ns/book#"),
    ("profile", "http://ogp.me/ns/profile#"),
    ("event", "http://ogp.me/ns/event#"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
];

/// The local name of a schema.org IRI, however it is written: `http`/`https`,
/// with `www.` or not, a trailing `/` or `#`, `schema:`, the old
/// `data-vocabulary.org` (Google's predecessor of schema.org).
pub fn schema_local(iri: &str) -> Option<&str> {
    let iri = iri.trim();
    let rest = [
        "https://schema.org/",
        "http://schema.org/",
        "https://www.schema.org/",
        "http://www.schema.org/",
        "//schema.org/",
        "schema.org/",
        "www.schema.org/",
        "schema:",
        "sdo:",
        "http://data-vocabulary.org/",
        "https://data-vocabulary.org/",
    ]
    .iter()
    .find_map(|p| strip_prefix_ci(iri, p))?;
    let rest = rest.trim_matches(['/', '#']);
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

struct Index {
    types: HashMap<String, usize>,
    members: HashMap<String, usize>,
    props: HashMap<String, usize>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| Index {
        types: TYPES
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.to_ascii_lowercase(), i))
            .collect(),
        members: MEMBERS
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.to_ascii_lowercase(), i))
            .collect(),
        props: PROPERTIES.iter().enumerate().map(|(i, n)| (loose(n), i)).collect(),
    })
}

/// A schema.org type's canonical name (`musicevent` → `MusicEvent`), if it is one.
pub fn canonical_type(name: &str) -> Option<&'static str> {
    if let Ok(i) = TYPES.binary_search_by(|(n, _)| (*n).cmp(name)) {
        return Some(TYPES[i].0);
    }
    if name.is_empty() || name.len() > 64 || name.contains([':', '/']) {
        return None;
    }
    index().types.get(&loose(name)).map(|&i| TYPES[i].0)
}

/// A name as sites mistype it, for lookup: lowercase, without `_`, `-` and spaces
/// (`start_date`, `Start-Date`, `startdate ` are all `startdate`).
fn loose(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '_' | '-' | ' ' | '\t'))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// An enumeration member's canonical name (`eventcancelled` → `EventCancelled`), if it is one.
pub fn canonical_member(name: &str) -> Option<&'static str> {
    if let Ok(i) = MEMBERS.binary_search_by(|(n, _)| (*n).cmp(name)) {
        return Some(MEMBERS[i].0);
    }
    index().members.get(&name.to_ascii_lowercase()).map(|&i| MEMBERS[i].0)
}

/// A schema.org property's canonical name (`startdate` → `startDate`), if it is one.
pub fn canonical_property(name: &str) -> Option<&'static str> {
    if let Ok(i) = PROPERTIES.binary_search(&name) {
        return Some(PROPERTIES[i]);
    }
    index().props.get(&loose(name)).map(|&i| PROPERTIES[i])
}

/// schema.org properties whose values are IRIs (its context types them `@id`): a relative
/// value resolves against the page.
pub(crate) const URL_PROPERTIES: &[&str] = &[
    "url",
    "sameAs",
    "image",
    "logo",
    "photo",
    "thumbnailUrl",
    "contentUrl",
    "embedUrl",
    "mainEntityOfPage",
    "additionalType",
    "hasMap",
    "downloadUrl",
    "installUrl",
    "codeRepository",
    "discussionUrl",
    "significantLink",
    "relatedLink",
    "archivedAt",
    "isBasedOn",
    "license",
    "acquireLicensePage",
    "replyToUrl",
    "thumbnail",
    "schemaVersion",
    "trailer",
    "video",
    "audio",
];

/// Whether a property (schema.org's, by local name; OpenGraph's media and urls) holds an IRI.
pub fn is_url_property(iri: &str) -> bool {
    if let Some(local) = schema_local(iri) {
        return URL_PROPERTIES.iter().any(|u| u.eq_ignore_ascii_case(local));
    }
    ["http://ogp.me/ns#", "https://ogp.me/ns#"].iter().any(|ns| {
        iri.strip_prefix(ns).is_some_and(|l| {
            matches!(
                l,
                "url"
                    | "image"
                    | "video"
                    | "audio"
                    | "image:url"
                    | "image:secure_url"
                    | "video:url"
                    | "video:secure_url"
                    | "audio:url"
                    | "audio:secure_url"
            )
        })
    })
}

/// Whether a property (by its short name) takes a schema.org enumeration member as its value.
pub fn takes_enumeration(property: &str) -> bool {
    matches!(
        property,
        "eventStatus"
            | "eventAttendanceMode"
            | "availability"
            | "itemCondition"
            | "businessFunction"
            | "deliveryMethod"
            | "acceptedPaymentMethod"
            | "dayOfWeek"
            | "gender"
            | "bookFormat"
            | "priceType"
            | "returnPolicyCategory"
            | "actionStatus"
            | "musicReleaseFormat"
            | "musicAlbumProductionType"
    )
}

/// data-vocabulary.org's names (Google's vocabulary before schema.org) as schema.org's.
fn data_vocabulary(local: &str) -> &str {
    match local {
        "summary" | "fn" => "name",
        "photo" => "image",
        "street-address" => "streetAddress",
        "locality" => "addressLocality",
        "region" => "addressRegion",
        "postal-code" => "postalCode",
        "country-name" => "addressCountry",
        "tel" => "telephone",
        "eventType" => "additionalType",
        other => other,
    }
}

/// Whether schema.org type `t` is `of` or one of its subtypes (`Festival` is an `Event`).
pub fn is_a(t: &str, of: &str) -> bool {
    let (Some(t), Some(of)) = (canonical_type(t), canonical_type(of)) else {
        return false;
    };
    let mut stack = vec![t];
    let mut seen = Vec::new();
    while let Some(x) = stack.pop() {
        if x == of {
            return true;
        }
        if seen.contains(&x) {
            continue;
        }
        seen.push(x);
        if let Ok(i) = TYPES.binary_search_by(|(n, _)| (*n).cmp(x)) {
            stack.extend(TYPES[i].1.iter().copied());
        }
    }
    false
}

/// How a type or property IRI is written in the output: schema.org's by its
/// canonical local name, a known vocabulary's as `prefix:name`, others whole.
pub fn short_name(iri: &str, is_type: bool) -> String {
    if let Some(local) = schema_local(iri) {
        let local = if !is_type && iri.to_ascii_lowercase().contains("data-vocabulary.org/") {
            data_vocabulary(local)
        } else {
            local
        };
        let canonical = if is_type {
            canonical_type(local)
        } else {
            canonical_property(local)
        };
        return canonical.unwrap_or(local).to_owned();
    }
    for (prefix, ns) in OUTPUT_PREFIXES.iter().chain(PREFIXES) {
        if let Some(rest) = iri.strip_prefix(ns) {
            if !rest.is_empty() {
                return format!("{prefix}:{rest}");
            }
        }
    }
    iri.to_owned()
}

/// The namespace a prefix of [`short_name`]'s output stands for (`og` → `http://ogp.me/ns#`).
pub fn prefix_namespace(prefix: &str) -> Option<&'static str> {
    OUTPUT_PREFIXES
        .iter()
        .chain(PREFIXES)
        .find(|(p, _)| *p == prefix)
        .map(|(_, ns)| *ns)
}

/// A value that names a schema.org enumeration member (`https://schema.org/EventCancelled`,
/// `EventCancelled`, `eventcancelled`) by its canonical name.
pub fn enum_value(value: &str) -> Option<&'static str> {
    match schema_local(value) {
        Some(local) => canonical_member(local).or_else(|| canonical_type(local)),
        None => canonical_member(value.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_org_however_written() {
        for iri in [
            "https://schema.org/MusicEvent",
            "http://www.schema.org/MusicEvent/",
            "schema:MusicEvent",
            "HTTPS://SCHEMA.ORG/MusicEvent",
        ] {
            assert_eq!(schema_local(iri), Some("MusicEvent"), "{iri}");
        }
        assert_eq!(schema_local("https://example.org/Event"), None);
        assert_eq!(short_name("http://schema.org/musicevent", true), "MusicEvent");
        assert_eq!(short_name("http://schema.org/startdate", false), "startDate");
        assert_eq!(short_name("http://ogp.me/ns#title", false), "og:title");
        assert_eq!(short_name("https://example.org/x", false), "https://example.org/x");
    }

    #[test]
    fn hierarchy_and_enumerations() {
        assert!(is_a("Festival", "Event"));
        assert!(is_a("MusicEvent", "Thing"));
        assert!(is_a("musicevent", "event"));
        assert!(!is_a("Place", "Event"));
        assert_eq!(enum_value("https://schema.org/EventCancelled"), Some("EventCancelled"));
        assert_eq!(enum_value("eventpostponed"), Some("EventPostponed"));
        assert_eq!(enum_value("http://schema.org/InStock"), Some("InStock"));
        assert_eq!(enum_value("sold out"), None);
    }
}
