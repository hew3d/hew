// Copyright (c) 2020 lumi <lumi@pew.im>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#![allow(unused)]

use minidom::Element;

const DATA: &str = r#"<articles xmlns="article">
    <article>
        <title>10 Terrible Bugs You Would NEVER Believe Happened</title>
        <body>
            Rust fixed them all. &lt;3
        </body>
    </article>
    <article>
        <title>BREAKING NEWS: Physical Bug Jumps Out Of Programmer's Screen</title>
        <body>
            Just kidding!
        </body>
    </article>
</articles>"#;

const ARTICLE_NS: &str = "article";

#[derive(Debug)]
pub struct Article {
    title: String,
    body: String,
}

fn main() {
    let root: Element = DATA.parse().unwrap();

    let mut articles: Vec<Article> = Vec::new();

    for child in root.children() {
        if child.is("article", ARTICLE_NS) {
            let title = child.get_child("title", ARTICLE_NS).unwrap().text();
            let body = child.get_child("body", ARTICLE_NS).unwrap().text();
            articles.push(Article {
                title,
                body: body.trim().to_owned(),
            });
        }
    }

    println!("{:?}", articles);
}
