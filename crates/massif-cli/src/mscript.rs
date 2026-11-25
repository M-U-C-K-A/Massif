//! `massif-cli mscript …`: MassifScript from the command line: the guide a
//! model reads, a script built into a terrain graph, a graph written as a
//! script, a script checked.

use super::{Args, Res};
use massif_terrain::model::Graph;
use massif_terrain::mscript;
use std::path::Path;

fn read(path: &Path) -> Res<String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Compiles the script at `path`, its errors and warnings printed; the graph.
fn compile(path: &Path) -> Res<Graph> {
    let text = read(path)?;
    let mut loader = mscript::FileLoader { dir: path.parent().map(|p| p.to_path_buf()) };
    let out = mscript::compile_named(&path.display().to_string(), &text, &mut loader);
    for e in &out.errors {
        let file = e.file.clone().unwrap_or_else(|| path.display().to_string());
        eprintln!("{file}:{}:{}: {}{}", e.span.line, e.span.col, if e.warning { "warning: " } else { "error: " }, e.message);
    }
    out.graph.ok_or_else(|| "the script has mistakes: no graph made".to_owned())
}

pub fn cmd(mut args: Args) -> Res<()> {
    if args.0.is_empty() {
        return Err("mscript needs a command: guide, build, check or decompile".into());
    }
    let sub = args.0.remove(0);
    match sub.as_str() {
        "guide" => {
            let graph = args.option("--graph")?;
            let reference = args.flag("--reference");
            args.positional([])?;
            if reference {
                print!("{}", mscript::reference());
                return Ok(());
            }
            let current = match graph {
                Some(g) => Some(decompiled(Path::new(&g))?),
                None => None,
            };
            print!("{}", mscript::guide(current.as_deref()));
            Ok(())
        }
        "build" => {
            let [script, out] = args.positional(["<in.mscript>", "<out.json>"])?;
            let g = compile(&script)?;
            std::fs::write(&out, g.to_json()).map_err(|e| format!("{}: {e}", out.display()))?;
            eprintln!("{}: {} nodes, {} groups, {} composites", out.display(), g.nodes.len(), g.frames.len(), g.components.len());
            Ok(())
        }
        "check" => {
            let [script] = args.positional(["<in.mscript>"])?;
            let g = compile(&script)?;
            let diags = massif_terrain::format::check(&g);
            for d in &diags {
                eprintln!("{d}");
            }
            eprintln!("{}: a script that makes a graph of {} nodes", script.display(), g.nodes.len());
            Ok(())
        }
        "decompile" => {
            let title = args.option("--title")?;
            let [graph] = args.positional(["<graph.json>"])?;
            let g = Graph::from_json(&read(&graph)?).map_err(|e| format!("{}: {e}", graph.display()))?;
            print!("{}", mscript::decompile(&g, title.as_deref()));
            Ok(())
        }
        other => Err(format!("unknown mscript command \"{other}\": guide, build, check or decompile")),
    }
}

fn decompiled(path: &Path) -> Res<String> {
    let text = read(path)?;
    if path.extension().is_some_and(|e| e == "mscript") {
        return Ok(text);
    }
    let g = Graph::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(mscript::decompile(&g, None))
}
