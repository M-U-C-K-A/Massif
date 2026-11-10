//! Print the object graph of a WorldPainter `.world` file (or any gzipped
//! Java serialization stream), to see what it contains.
//!
//! `cargo run --release -p massif-legacy --example dump -- file.world [depth]`

use massif_legacy::javaser::{Arena, Content, Item, Parser, Value};
use std::collections::HashSet;
use std::io::BufReader;

fn show(a: &Arena, v: Value, depth: usize, max: usize, seen: &mut HashSet<usize>, indent: usize) {
    let pad = "  ".repeat(indent);
    match v {
        Value::Ref(h) => {
            let class = a.class_name(v).unwrap_or("?");
            match a.get(h) {
                Some(Content::String(s)) => println!("{pad}\"{}\"", s.chars().take(80).collect::<String>()),
                Some(Content::Enum { name, .. }) => println!("{pad}{class}::{name}"),
                Some(Content::Array { data, .. }) => println!("{pad}{class}[{}]", data.len()),
                Some(Content::Object(_)) => {
                    if !seen.insert(h) {
                        println!("{pad}{class} (#{h}, seen)");
                        return;
                    }
                    if depth >= max {
                        println!("{pad}{class} (#{h}) …");
                        return;
                    }
                    println!("{pad}{class} (#{h})");
                    if let Some(entries) = a.map_entries(v) {
                        println!("{pad}  {} entries", entries.len());
                        for (k, val) in entries.iter().take(6) {
                            show(a, *k, depth + 1, max, seen, indent + 2);
                            show(a, *val, depth + 1, max, seen, indent + 3);
                        }
                        return;
                    }
                    if let Some(items) = a.list_items(v) {
                        println!("{pad}  {} items", items.len());
                        for item in items.iter().take(6) {
                            show(a, *item, depth + 1, max, seen, indent + 2);
                        }
                        return;
                    }
                    for (name, value) in a.fields(v) {
                        match value {
                            Value::Ref(_) | Value::Null => {
                                println!("{pad}  .{name} =");
                                show(a, value, depth + 1, max, seen, indent + 2);
                            }
                            other => println!("{pad}  .{name} = {other:?}"),
                        }
                    }
                    if let Some(o) = a.object(v) {
                        for cd in &o.data {
                            let blocks: usize = cd.annotation.iter().map(|i| match i { Item::Block(b) => b.len(), _ => 0 }).sum();
                            let values = cd.annotation.iter().filter(|i| matches!(i, Item::Value(_))).count();
                            if blocks + values > 0 {
                                println!("{pad}  [writeObject: {blocks} bytes, {values} objects]");
                            }
                        }
                    }
                }
                Some(Content::Class(_)) => println!("{pad}class"),
                Some(Content::ClassDesc(d)) => println!("{pad}descriptor {}", d.name),
                Some(Content::Pending) => println!("{pad}(pending)"),
                Some(Content::Freed) | None => println!("{pad}(freed)"),
            }
        }
        Value::Null => println!("{pad}null"),
        other => println!("{pad}{other:?}"),
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("a .world file");
    let max: usize = std::env::args().nth(2).and_then(|d| d.parse().ok()).unwrap_or(3);
    let file = std::fs::File::open(&path).expect("open");
    let gz = flate2::read::GzDecoder::new(BufReader::with_capacity(1 << 20, file));
    let mut parser = Parser::new(BufReader::with_capacity(1 << 20, gz)).expect("Java stream");
    let started = std::time::Instant::now();
    let mut tops = Vec::new();
    while let Some(v) = parser.next_object().expect("parse") {
        tops.push(v);
    }
    eprintln!("{} bytes, {} items, {:.2?}", parser.position, parser.arena.items.len(), started.elapsed());
    let mut seen = HashSet::new();
    for v in tops {
        show(&parser.arena, v, 0, max, &mut seen, 0);
    }
}
