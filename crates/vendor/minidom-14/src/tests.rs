// Copyright (c) 2020 lumi <lumi@pew.im>
// Copyright (c) 2020 Emmanuel Gil Peyrot <linkmauve@linkmauve.fr>
// Copyright (c) 2020 Bastien Orivel <eijebong+minidom@bananium.fr>
// Copyright (c) 2020 Astro <astro@spaceboyz.net>
// Copyright (c) 2020 Maxime “pep” Buquet <pep@bouah.net>
// Copyright (c) 2020 Yue Liu <amznyue@amazon.com>
// Copyright (c) 2020 Matt Bilker <me@mbilker.us>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::element::Element;
use crate::error::Error;

use quick_xml::Reader;

const TEST_STRING: &str = r#"<root xmlns="root_ns" a="b" xml:lang="en">meow<child c="d"/><child xmlns="child_ns" d="e" xml:lang="fr"/>nya</root>"#;

fn build_test_tree() -> Element {
    let mut root = Element::builder("root", "root_ns")
        .attr("xml:lang", "en")
        .attr("a", "b")
        .build();
    root.append_text_node("meow");
    let child = Element::builder("child", "root_ns").attr("c", "d").build();
    root.append_child(child);
    let other_child = Element::builder("child", "child_ns")
        .attr("d", "e")
        .attr("xml:lang", "fr")
        .build();
    root.append_child(other_child);
    root.append_text_node("nya");
    root
}

#[test]
fn reader_works() {
    let mut reader = Reader::from_str(TEST_STRING);
    assert_eq!(
        Element::from_reader(&mut reader).unwrap(),
        build_test_tree()
    );
}

#[test]
fn reader_deduplicate_prefixes() {
    // The reader shouldn't complain that "child" doesn't have a namespace. It should reuse the
    // parent ns with the same prefix.
    let _: Element = r#"<root xmlns="ns1"><child/></root>"#.parse().unwrap();
    let _: Element = r#"<p1:root xmlns:p1="ns1"><p1:child/></p1:root>"#.parse().unwrap();
    let _: Element = r#"<root xmlns="ns1"><child xmlns:p1="ns2"><p1:grandchild/></child></root>"#
        .parse()
        .unwrap();

    match r#"<p1:root xmlns:p1="ns1"><child/></p1:root>"#.parse::<Element>() {
        Err(Error::MissingNamespace) => (),
        Err(err) => panic!("No or wrong error: {:?}", err),
        Ok(elem) => panic!(
            "Got Element: {}; was expecting Error::MissingNamespace",
            String::from(&elem)
        ),
    }
}

#[test]
fn reader_no_deduplicate_sibling_prefixes() {
    // The reader shouldn't reuse the sibling's prefixes
    match r#"<root xmlns="ns1"><p1:child1 xmlns:p1="ns2"/><p1:child2/></root>"#.parse::<Element>() {
        Err(Error::MissingNamespace) => (),
        Err(err) => panic!("No or wrong error: {:?}", err),
        Ok(elem) => panic!(
            "Got Element:\n{:?}\n{}\n; was expecting Error::MissingNamespace",
            elem,
            String::from(&elem)
        ),
    }
}

#[test]
fn test_real_data() {
    let correction = Element::builder("replace", "urn:xmpp:message-correct:0").build();
    let body = Element::builder("body", "jabber:client").build();
    let message = Element::builder("message", "jabber:client")
        .append(body)
        .append(correction)
        .build();
    let stream = Element::builder("stream", "http://etherx.jabber.org/streams")
        .prefix(
            Some(String::from("stream")),
            "http://etherx.jabber.org/streams",
        )
        .unwrap()
        .prefix(None, "jabber:client")
        .unwrap()
        .append(message)
        .build();
    println!("{}", String::from(&stream));

    let jid = Element::builder("jid", "urn:xmpp:presence:0").build();
    let nick = Element::builder("nick", "urn:xmpp:presence:0").build();
    let mix = Element::builder("mix", "urn:xmpp:presence:0")
        .append(jid)
        .append(nick)
        .build();
    let show = Element::builder("show", "jabber:client").build();
    let status = Element::builder("status", "jabber:client").build();
    let presence = Element::builder("presence", "jabber:client")
        .append(show)
        .append(status)
        .append(mix)
        .build();
    let item = Element::builder("item", "http://jabber.org/protocol/pubsub")
        .append(presence)
        .build();
    let items = Element::builder("items", "http://jabber.org/protocol/pubsub")
        .append(item)
        .build();
    let pubsub = Element::builder("pubsub", "http://jabber.org/protocol/pubsub")
        .append(items)
        .build();
    let iq = Element::builder("iq", "jabber:client")
        .append(pubsub)
        .build();
    let stream = Element::builder("stream", "http://etherx.jabber.org/streams")
        .prefix(
            Some(String::from("stream")),
            "http://etherx.jabber.org/streams",
        )
        .unwrap()
        .prefix(None, "jabber:client")
        .unwrap()
        .append(iq)
        .build();

    println!("{}", String::from(&stream));
}

#[test]
fn writer_works() {
    let root = build_test_tree();
    let mut writer = Vec::new();
    {
        root.write_to(&mut writer).unwrap();
    }
    assert_eq!(String::from_utf8(writer).unwrap(), TEST_STRING);
}

#[test]
fn writer_with_decl_works() {
    let root = build_test_tree();
    let mut writer = Vec::new();
    {
        root.write_to_decl(&mut writer).unwrap();
    }
    let result = format!(r#"<?xml version="1.0" encoding="utf-8"?>{}"#, TEST_STRING);
    assert_eq!(String::from_utf8(writer).unwrap(), result);
}

#[test]
fn writer_with_prefix() {
    let root = Element::builder("root", "ns1")
        .prefix(Some(String::from("p1")), "ns1")
        .unwrap()
        .prefix(None, "ns2")
        .unwrap()
        .build();
    assert_eq!(
        String::from(&root),
        r#"<p1:root xmlns="ns2" xmlns:p1="ns1"/>"#,
    );
}

#[test]
fn writer_no_prefix_namespace() {
    let root = Element::builder("root", "ns1").build();
    // TODO: Note that this isn't exactly equal to a None prefix. it's just that the None prefix is
    // the most obvious when it's not already used. Maybe fix tests so that it only checks that the
    // prefix used equals the one declared for the namespace.
    assert_eq!(String::from(&root), r#"<root xmlns="ns1"/>"#);
}

#[test]
fn writer_no_prefix_namespace_child() {
    let child = Element::builder("child", "ns1").build();
    let root = Element::builder("root", "ns1").append(child).build();
    // TODO: Same remark as `writer_no_prefix_namespace`.
    assert_eq!(String::from(&root), r#"<root xmlns="ns1"><child/></root>"#);

    let child = Element::builder("child", "ns2")
        .prefix(None, "ns3")
        .unwrap()
        .build();
    let root = Element::builder("root", "ns1").append(child).build();
    // TODO: Same remark as `writer_no_prefix_namespace`.
    assert_eq!(
        String::from(&root),
        r#"<root xmlns="ns1"><ns0:child xmlns:ns0="ns2" xmlns="ns3"/></root>"#
    );
}

#[test]
fn writer_prefix_namespace_child() {
    let child = Element::builder("child", "ns1").build();
    let root = Element::builder("root", "ns1")
        .prefix(Some(String::from("p1")), "ns1")
        .unwrap()
        .append(child)
        .build();
    assert_eq!(
        String::from(&root),
        r#"<p1:root xmlns:p1="ns1"><p1:child/></p1:root>"#
    );
}

#[test]
fn writer_with_prefix_deduplicate() {
    let child = Element::builder("child", "ns1")
        // .prefix(Some(String::from("p1")), "ns1")
        .build();
    let root = Element::builder("root", "ns1")
        .prefix(Some(String::from("p1")), "ns1")
        .unwrap()
        .prefix(None, "ns2")
        .unwrap()
        .append(child)
        .build();
    assert_eq!(
        String::from(&root),
        r#"<p1:root xmlns="ns2" xmlns:p1="ns1"><p1:child/></p1:root>"#,
    );

    // Ensure descendants don't just reuse ancestors' prefixes that have been shadowed in between
    let grandchild = Element::builder("grandchild", "ns1").build();
    let child = Element::builder("child", "ns2").append(grandchild).build();
    let root = Element::builder("root", "ns1").append(child).build();
    assert_eq!(
        String::from(&root),
        r#"<root xmlns="ns1"><child xmlns="ns2"><grandchild xmlns="ns1"/></child></root>"#,
    );
}

#[test]
fn writer_escapes_attributes() {
    let root = Element::builder("root", "ns1")
        .attr("a", "\"Air\" quotes")
        .build();
    let mut writer = Vec::new();
    {
        root.write_to(&mut writer).unwrap();
    }
    assert_eq!(
        String::from_utf8(writer).unwrap(),
        r#"<root xmlns="ns1" a="&quot;Air&quot; quotes"/>"#
    );
}

#[test]
fn writer_escapes_text() {
    let root = Element::builder("root", "ns1").append("<3").build();
    let mut writer = Vec::new();
    {
        root.write_to(&mut writer).unwrap();
    }
    assert_eq!(
        String::from_utf8(writer).unwrap(),
        r#"<root xmlns="ns1">&lt;3</root>"#
    );
}

#[test]
fn reader_resolves_entities_and_char_refs() {
    // quick-xml 0.41 emits each reference as its own `GeneralRef` event; verify
    // the five XML predefined entities, decimal and hex character references,
    // and literal text between references all reassemble into one text value.
    let xml = r#"<root xmlns="ns">a&amp;b&lt;c&gt;d&quot;e&apos;f&#49;g&#x30;h</root>"#;
    let mut reader = Reader::from_str(xml);
    let root = Element::from_reader(&mut reader).unwrap();
    assert_eq!(root.text(), r#"a&b<c>d"e'f1g0h"#);
}

#[test]
fn reader_rejects_unknown_entity() {
    // Minidom targets DTD-less documents: an unrecognized general entity must be
    // a hard error rather than silently dropped.
    let xml = r#"<root xmlns="ns">&bogus;</root>"#;
    let mut reader = Reader::from_str(xml);
    assert!(Element::from_reader(&mut reader).is_err());
}

#[test]
fn builder_works() {
    let elem = Element::builder("a", "b")
        .attr("c", "d")
        .append(Element::builder("child", "b"))
        .append("e")
        .build();
    assert_eq!(elem.name(), "a");
    assert_eq!(elem.ns(), "b".to_owned());
    assert_eq!(elem.attr("c"), Some("d"));
    assert_eq!(elem.attr("x"), None);
    assert_eq!(elem.text(), "e");
    assert!(elem.has_child("child", "b"));
    assert!(elem.is("a", "b"));
}

#[test]
fn children_iter_works() {
    let root = build_test_tree();
    let mut iter = root.children();
    assert!(iter.next().unwrap().is("child", "root_ns"));
    assert!(iter.next().unwrap().is("child", "child_ns"));
    assert_eq!(iter.next(), None);
}

#[test]
fn get_child_works() {
    let root = build_test_tree();
    assert_eq!(root.get_child("child", "inexistent_ns"), None);
    assert_eq!(root.get_child("not_a_child", "root_ns"), None);
    assert!(root
        .get_child("child", "root_ns")
        .unwrap()
        .is("child", "root_ns"));
    assert!(root
        .get_child("child", "child_ns")
        .unwrap()
        .is("child", "child_ns"));
    assert_eq!(
        root.get_child("child", "root_ns").unwrap().attr("c"),
        Some("d")
    );
    assert_eq!(
        root.get_child("child", "child_ns").unwrap().attr("d"),
        Some("e")
    );
}

#[test]
fn namespace_propagation_works() {
    let mut root = Element::builder("root", "root_ns").build();
    let mut child = Element::bare("child", "root_ns");
    let grandchild = Element::bare("grandchild", "root_ns");
    child.append_child(grandchild);
    root.append_child(child);

    assert_eq!(root.get_child("child", "root_ns").unwrap().ns(), root.ns());
    assert_eq!(
        root.get_child("child", "root_ns")
            .unwrap()
            .get_child("grandchild", "root_ns")
            .unwrap()
            .ns(),
        root.ns()
    );
}

#[test]
fn two_elements_with_same_arguments_different_order_are_equal() {
    let elem1: Element = "<a b='a' c='' xmlns='ns1'/>".parse().unwrap();
    let elem2: Element = "<a c='' b='a' xmlns='ns1'/>".parse().unwrap();
    assert_eq!(elem1, elem2);

    let elem1: Element = "<a b='a' c='' xmlns='ns1'/>".parse().unwrap();
    let elem2: Element = "<a c='d' b='a' xmlns='ns1'/>".parse().unwrap();
    assert_ne!(elem1, elem2);
}

#[test]
fn namespace_attributes_works() {
    let mut reader = Reader::from_str(TEST_STRING);
    let root = Element::from_reader(&mut reader).unwrap();
    assert_eq!("en", root.attr("xml:lang").unwrap());
    assert_eq!(
        "fr",
        root.get_child("child", "child_ns")
            .unwrap()
            .attr("xml:lang")
            .unwrap()
    );
}

#[test]
fn wrongly_closed_elements_error() {
    let elem1 = "<a xmlns='ns1'></b>".parse::<Element>();
    assert!(elem1.is_err());
    let elem1 = "<a xmlns='ns1'></c></a>".parse::<Element>();
    assert!(elem1.is_err());
    let elem1 = "<a xmlns='ns1'><c xmlns='ns1'><d xmlns='ns1'/></c></a>".parse::<Element>();
    assert!(elem1.is_ok());
}

#[test]
fn namespace_simple() {
    let elem: Element = "<message xmlns='jabber:client'/>".parse().unwrap();
    assert_eq!(elem.name(), "message");
    assert_eq!(elem.ns(), "jabber:client".to_owned());
}

#[test]
fn namespace_prefixed() {
    let elem: Element = "<stream:features xmlns:stream='http://etherx.jabber.org/streams'/>"
        .parse()
        .unwrap();
    assert_eq!(elem.name(), "features");
    assert_eq!(elem.ns(), "http://etherx.jabber.org/streams".to_owned(),);
}

#[test]
fn namespace_inherited_simple() {
    let elem: Element = "<stream xmlns='jabber:client'><message xmlns='jabber:client' /></stream>"
        .parse()
        .unwrap();
    assert_eq!(elem.name(), "stream");
    assert_eq!(elem.ns(), "jabber:client".to_owned());
    let child = elem.children().next().unwrap();
    assert_eq!(child.name(), "message");
    assert_eq!(child.ns(), "jabber:client".to_owned());
}

#[test]
fn namespace_inherited_prefixed1() {
    let elem: Element = "<stream:features xmlns:stream='http://etherx.jabber.org/streams' xmlns='jabber:client'><message xmlns='jabber:client' /></stream:features>"
        .parse().unwrap();
    assert_eq!(elem.name(), "features");
    assert_eq!(elem.ns(), "http://etherx.jabber.org/streams".to_owned(),);
    let child = elem.children().next().unwrap();
    assert_eq!(child.name(), "message");
    assert_eq!(child.ns(), "jabber:client".to_owned());
}

#[test]
fn namespace_inherited_prefixed2() {
    let elem: Element = "<stream xmlns='http://etherx.jabber.org/streams' xmlns:jabber='jabber:client'><jabber:message xmlns:jabber='jabber:client' /></stream>"
        .parse().unwrap();
    assert_eq!(elem.name(), "stream");
    assert_eq!(elem.ns(), "http://etherx.jabber.org/streams".to_owned(),);
    let child = elem.children().next().unwrap();
    assert_eq!(child.name(), "message");
    assert_eq!(child.ns(), "jabber:client".to_owned());
}

#[test]
fn xml_error() {
    match "<a xmlns='ns1'></b>".parse::<Element>() {
        Err(crate::error::Error::XmlError(_)) => (),
        err => panic!("No or wrong error: {:?}", err),
    }

    match "<a xmlns='ns1'></".parse::<Element>() {
        Err(crate::error::Error::XmlError(_)) => (),
        err => panic!("No or wrong error: {:?}", err),
    }
}

#[test]
fn invalid_element_error() {
    match "<a:b:c>".parse::<Element>() {
        Err(crate::error::Error::InvalidElement) => (),
        err => panic!("No or wrong error: {:?}", err),
    }
}

#[test]
fn missing_namespace_error() {
    match "<a/>".parse::<Element>() {
        Err(crate::error::Error::MissingNamespace) => (),
        err => panic!("No or wrong error: {:?}", err),
    }
}
