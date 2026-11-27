//! 6.4 `build_index`: the core of `hew-cli library index`.
//!
//! Ordering decision (the doc leaves it open; documented here as the
//! oracle): `items` are sorted by `path`, ascending by UTF-8 byte order.
//! Paths are unique, so the order is total. Input order never matters, so
//! a folder walk in any order gives byte-identical output.
mod common;
use common::*;
use proptest::prelude::*;
use sha2::{Digest, Sha256};
use sources::{Category, IndexHeader, LocalItem, LocalMeta, SourceIndex, build_index, parse_index};

fn meta(n: u32, cat: Category) -> LocalMeta {
    LocalMeta {
        id: Some(uuid(n)),
        name: Some(format!("Item {n}")),
        category: Some(cat),
        keywords: vec!["a".into(), "b".into()],
        collection: Some("Coll".into()),
    }
}

fn local(n: u32, path: &str, cat: Category) -> LocalItem {
    LocalItem {
        path: path.into(),
        bytes: format!("content-{n}").into_bytes(),
        meta: meta(n, cat),
    }
}

fn header() -> IndexHeader {
    IndexHeader {
        name: "Shop".into(),
        description: Some("d".into()),
        author: Some("a".into()),
        homepage: Some("https://example.com".into()),
        license: Some("MIT".into()),
        license_url: None,
    }
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn reparse(idx: &SourceIndex) -> SourceIndex {
    parse_index(&serde_json::to_vec(idx).unwrap()).expect("build_index output must re-parse")
}

#[test]
fn header_fields_are_carried_and_hew_is_1() {
    let idx = build_index(&[], header());
    assert_eq!(idx.hew, 1);
    assert_eq!(idx.name, "Shop");
    assert_eq!(idx.description.as_deref(), Some("d"));
    assert_eq!(idx.author.as_deref(), Some("a"));
    assert_eq!(idx.homepage.as_deref(), Some("https://example.com"));
    assert_eq!(idx.license.as_deref(), Some("MIT"));
    assert_eq!(idx.license_url, None);
    assert!(idx.items.is_empty());
    assert!(idx.plugins.is_empty());
}

#[test]
fn an_empty_folder_round_trips() {
    let idx = build_index(&[], header());
    assert_eq!(reparse(&idx), idx);
}

#[test]
fn each_item_gets_the_hash_size_and_meta_of_its_file() {
    let it = local(1, "Components/a.hew", Category::Components);
    let idx = build_index(std::slice::from_ref(&it), header());
    let e = &idx.items[0];
    assert_eq!(e.id, uuid(1));
    assert_eq!(e.path, "Components/a.hew");
    assert_eq!(e.name, "Item 1");
    assert_eq!(e.sha256, hex(&it.bytes));
    assert_eq!(e.bytes, it.bytes.len() as u64);
    assert_eq!(e.keywords, vec!["a", "b"]);
    assert_eq!(e.collection.as_deref(), Some("Coll"));
}

#[test]
fn library_categories_map_to_index_categories() {
    for (l, c) in [
        (Category::Components, Category::Components),
        (Category::Materials, Category::Materials),
        (Category::Models, Category::Models),
        (Category::Profiles, Category::Profiles),
    ] {
        let idx = build_index(&[local(1, "a.hew", l)], header());
        assert_eq!(idx.items[0].category, c);
    }
}

#[test]
fn a_thumbnail_when_present_is_named_by_the_items_hash() {
    let it = local(1, "a.hew", Category::Models);
    let idx = build_index(std::slice::from_ref(&it), header());
    if let Some(t) = &idx.items[0].thumbnail {
        assert_eq!(*t, format!("thumbnails/{}.png", hex(&it.bytes)));
    }
}

#[test]
fn items_are_sorted_by_path_in_byte_order() {
    let a = local(1, "b/z.hew", Category::Models);
    let b = local(2, "B/a.hew", Category::Models);
    let c = local(3, "a.hew", Category::Models);
    let d = local(4, "b/a.hew", Category::Models);
    let idx = build_index(&[a, b, c, d], header());
    let paths: Vec<&str> = idx.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, vec!["B/a.hew", "a.hew", "b/a.hew", "b/z.hew"]);
}

#[test]
fn output_does_not_depend_on_input_order() {
    let items: Vec<LocalItem> = (0..6)
        .map(|n| local(n, &format!("f{}/x{n}.hew", n % 3), Category::Components))
        .collect();
    let want = serde_json::to_vec(&build_index(&items, header())).unwrap();
    let mut rev = items.clone();
    rev.reverse();
    assert_eq!(
        serde_json::to_vec(&build_index(&rev, header())).unwrap(),
        want
    );
    let mut rot = items.clone();
    rot.rotate_left(2);
    assert_eq!(
        serde_json::to_vec(&build_index(&rot, header())).unwrap(),
        want
    );
}

#[test]
fn build_is_deterministic() {
    let items: Vec<LocalItem> = (0..4)
        .map(|n| local(n, &format!("x{n}.hew"), Category::Materials))
        .collect();
    let a = serde_json::to_vec(&build_index(&items, header())).unwrap();
    let b = serde_json::to_vec(&build_index(&items, header())).unwrap();
    assert_eq!(a, b);
}

#[test]
fn built_index_round_trips_through_parse_index() {
    let items: Vec<LocalItem> = (0..5)
        .map(|n| local(n, &format!("d/x {n}.hew"), Category::Components))
        .collect();
    let idx = build_index(&items, header());
    assert_eq!(reparse(&idx), idx);
}

fn arb_segment() -> impl Strategy<Value = String> {
    "[A-Za-z0-9][A-Za-z0-9 _.-]{0,10}"
        .prop_filter("no trailing dot-leading", |s| !s.starts_with('.'))
}

fn arb_items() -> impl Strategy<Value = Vec<LocalItem>> {
    prop::collection::btree_map(
        prop::collection::vec(arb_segment(), 1..4).prop_map(|v| format!("{}.hew", v.join("/"))),
        (prop::collection::vec(any::<u8>(), 1..64), 0u8..4),
        0..12,
    )
    .prop_map(|m| {
        m.into_iter()
            .enumerate()
            .map(|(n, (path, (bytes, c)))| {
                let cat = [
                    Category::Components,
                    Category::Materials,
                    Category::Models,
                    Category::Profiles,
                ][c as usize];
                LocalItem {
                    path,
                    bytes,
                    meta: meta(n as u32, cat),
                }
            })
            .collect()
    })
}

proptest! {
    #[test]
    fn build_then_parse_is_the_identity(items in arb_items()) {
        let idx = build_index(&items, header());
        prop_assert_eq!(idx.items.len(), items.len());
        prop_assert_eq!(&reparse(&idx), &idx);
    }

    #[test]
    fn build_is_sorted_and_order_independent(items in arb_items(), k in 0usize..12) {
        let idx = build_index(&items, header());
        let paths: Vec<&str> = idx.items.iter().map(|i| i.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        prop_assert_eq!(&paths, &sorted);
        let mut rot = items.clone();
        if !rot.is_empty() { let n = k % rot.len(); rot.rotate_left(n); }
        prop_assert_eq!(serde_json::to_vec(&build_index(&rot, header())).unwrap(), serde_json::to_vec(&idx).unwrap());
    }

    #[test]
    fn every_entry_matches_its_file(items in arb_items()) {
        let idx = build_index(&items, header());
        for e in &idx.items {
            let src = items.iter().find(|i| i.path == e.path).expect("entry has a source file");
            prop_assert_eq!(&e.sha256, &hex(&src.bytes));
            prop_assert_eq!(e.bytes, src.bytes.len() as u64);
        }
    }
}
