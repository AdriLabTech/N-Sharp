//! REPL de N#: cada entrada se verifica junto con el historial y solo se ejecuta lo nuevo;
//! las variables persisten en un ámbito raíz que se reutiliza entre ejecuciones.

use crate::check::{Mode, Options, WARNING_CATEGORIES};
use crate::diag;
use crate::interp::value::Scope;
use crate::interp::{Ctl, Interp};
use crate::program::SourceConfig;
use std::io::{BufRead, IsTerminal, Write};

fn starts_decl(s: &str) -> bool {
    let t = s.trim_start();
    ["fn ", "struct ", "enum ", "trait ", "impl", "import ", "const ", "static ", "type ", "pub ", "@", "async fn", "unsafe fn", "test "]
        .iter()
        .any(|k| t.starts_with(k))
}

/// ¿Faltan cierres de `(`, `[`, `{` o comillas? (continuación de línea)
fn incomplete(s: &str) -> bool {
    let (mut depth, mut in_str, mut esc, mut triple) = (0i32, false, false, false);
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if triple {
            if c == '"' && b.get(i + 1) == Some(&'"') && b.get(i + 2) == Some(&'"') { triple = false; i += 2; }
        } else if in_str {
            if esc { esc = false } else if c == '\\' { esc = true } else if c == '"' { in_str = false }
        } else {
            match c {
                '"' => {
                    if b.get(i + 1) == Some(&'"') && b.get(i + 2) == Some(&'"') { triple = true; i += 2; } else { in_str = true }
                }
                '#' => {
                    while i < b.len() && b[i] != '\n' { i += 1; }
                }
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
    }
    depth > 0 || in_str || triple
}

fn opts() -> Options {
    Options { mode: Mode::Script, allow: WARNING_CATEGORIES.iter().map(|s| s.to_string()).collect(), deny: Default::default(), require_main: false }
}

pub fn run() -> i32 {
    std::thread::Builder::new().stack_size(1 << 30).spawn(repl_loop).expect("hilo").join().unwrap_or(70)
}

fn repl_loop() -> i32 {
    let tty = std::io::stdin().is_terminal();
    if tty {
        println!("N# {} — REPL. Escribe :help para ayuda, :quit para salir.", crate::cli::VERSION);
    }
    let mut items = String::new();
    let mut stmts: Vec<String> = vec![];
    let mut root = Scope::new(None);
    let stdin = std::io::stdin();
    loop {
        let mut input = String::new();
        let mut first = true;
        loop {
            if tty {
                print!("{}", if first { "ncom> " } else { "  ... " });
                let _ = std::io::stdout().flush();
            }
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => {
                    if tty { println!(); }
                    return 0;
                }
                Ok(_) => {}
                Err(_) => return 1,
            }
            input.push_str(&line);
            first = false;
            if !incomplete(&input) { break; }
        }
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            ":quit" | ":q" | ":exit" => return 0,
            ":help" | ":h" => {
                println!(":quit  salir\n:reset olvida declaraciones y variables\n:items muestra las declaraciones acumuladas\nCualquier expresión muestra su valor; las declaraciones (fn, struct, ...) se acumulan.");
                continue;
            }
            ":reset" => {
                items.clear();
                stmts.clear();
                root = Scope::new(None);
                println!("sesión reiniciada");
                continue;
            }
            ":items" => {
                print!("{items}");
                continue;
            }
            _ => {}
        }
        if starts_decl(trimmed) {
            let candidate = format!("{items}{trimmed}\n");
            let src = format!("{candidate}{}", stmts.join("\n"));
            let c = crate::compile("<repl>", &src, SourceConfig::default(), opts());
            if c.has_errors() {
                eprint!("{}", diag::render_all(&c.diags.iter().filter(|d| d.is_error()).cloned().collect::<Vec<_>>(), &c.map));
            } else {
                items = candidate;
            }
            continue;
        }
        // ¿expresión? → mostrar su valor; si no compila así, se trata como sentencia
        let as_stmt = if trimmed.ends_with(';') || trimmed.ends_with('}') { trimmed.to_string() } else { format!("{trimmed};") };
        let candidates: Vec<String> = {
            let body = trimmed.trim_end_matches(';');
            if crate::parser::parse_expr_source(0, body).0.is_some() && !trimmed.contains(":=") {
                vec![format!("__repl_show({body});"), as_stmt.clone()]
            } else {
                vec![as_stmt.clone()]
            }
        };
        let mut compiled = None;
        let mut last_diags = None;
        for cand in &candidates {
            let src = format!("{items}{}\n{cand}", stmts.join("\n"));
            let c = crate::compile("<repl>", &src, SourceConfig::default(), opts());
            if !c.has_errors() {
                compiled = Some((c, cand.clone()));
                break;
            }
            last_diags = Some(c);
        }
        let Some((c, stmt_src)) = compiled else {
            if let Some(c) = last_diags {
                eprint!("{}", diag::render_all(&c.diags.iter().filter(|d| d.is_error()).cloned().collect::<Vec<_>>(), &c.map));
            }
            continue;
        };
        let prog: &'static crate::program::Program = Box::leak(Box::new(c.prog));
        let map: &'static diag::SourceMap = Box::leak(Box::new(c.map));
        let mut it = match Interp::new(prog, map, 1, vec![]) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("error interno: {e}");
                continue;
            }
        };
        let before = root.vars.borrow().len();
        match it.exec_globals(&root, stmts.len()) {
            Ok(_) => {
                it.flush();
                stmts.push(stmt_src);
            }
            Err(Ctl::Panic(m)) => {
                it.report_panic(&m);
                root.vars.borrow_mut().truncate(before);
            }
            Err(Ctl::Exit(code)) => {
                it.flush();
                return code;
            }
            Err(_) => {
                it.flush();
            }
        }
    }
}
