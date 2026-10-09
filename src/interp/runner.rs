//! Ejecución de tests, benchmarks y sesiones de script (REPL).

use super::*;
use std::time::Instant;

impl Interp {
    fn test_label(&self, name: &str, mi: usize) -> String {
        if mi == self.prog.main { name.to_string() } else { format!("{}::{name}", self.prog.modules[mi].name) }
    }

    /// Ejecuta los `test "nombre" { ... }`. Devuelve (superados, fallidos).
    pub fn run_tests(&mut self, filter: Option<&str>) -> (usize, usize) {
        let tests: Vec<_> = self
            .tests
            .iter()
            .filter(|(n, _, mi)| !self.prog.modules[*mi].is_std && (!self.only_main_tests || *mi == self.prog.main) && filter.is_none_or(|f| self.test_label(n, *mi).contains(f)))
            .cloned()
            .collect();
        println!("ejecutando {} test(s)", tests.len());
        let (mut ok, mut failed) = (0, 0);
        let mut failures: Vec<(String, String)> = vec![];
        for (name, body, mi) in tests {
            let label = self.test_label(&name, mi);
            self.cur_module = mi;
            self.frames.clear();
            self.depth = 0;
            let scope = Scope::new(None);
            let r = self.eval_block_in(body, &scope);
            if self.has_drop {
                let _ = self.drop_scope(&scope);
            }
            let _ = self.out.flush();
            match r {
                Ok(_) | Err(Ctl::Return(_)) | Err(Ctl::Break) => {
                    println!("test {label} ... ok");
                    ok += 1;
                }
                Err(Ctl::Panic(m)) => {
                    println!("test {label} ... FALLÓ");
                    let loc = if self.cur_span != Span::default() {
                        let f = self.map.get(self.cur_span.file);
                        let (l, c) = f.line_col(self.cur_span.lo as usize);
                        format!(" ({}:{}:{})", f.name, l, c)
                    } else {
                        String::new()
                    };
                    failures.push((label, format!("{m}{loc}")));
                    failed += 1;
                }
                Err(Ctl::Exit(c)) => {
                    println!("test {label} ... FALLÓ");
                    failures.push((label, format!("el test llamó a exit({c})")));
                    failed += 1;
                }
            }
        }
        if !failures.is_empty() {
            println!("\nfallos:");
            for (n, m) in &failures {
                println!("  ---- {n} ----\n  {}", m.replace('\n', "\n  "));
            }
        }
        println!("\nresultado: {}. {ok} superados; {failed} fallidos", if failed == 0 { "OK" } else { "FALLÓ" });
        (ok, failed)
    }

    /// Ejecuta los `bench "nombre" { ... }` midiendo el tiempo medio por iteración.
    pub fn run_benches(&mut self, filter: Option<&str>) -> usize {
        let benches: Vec<_> = self
            .benches
            .iter()
            .filter(|(n, _, mi)| !self.prog.modules[*mi].is_std && (!self.only_main_tests || *mi == self.prog.main) && filter.is_none_or(|f| self.test_label(n, *mi).contains(f)))
            .cloned()
            .collect();
        println!("ejecutando {} benchmark(s)", benches.len());
        let mut failed = 0;
        for (name, body, mi) in benches {
            let label = self.test_label(&name, mi);
            self.cur_module = mi;
            let mut iters = 0u64;
            let start = Instant::now();
            let mut err: Option<String> = None;
            while iters < 1_000_000 && (iters < 3 || start.elapsed().as_millis() < 300) {
                let scope = Scope::new(None);
                match self.eval_block_in(body, &scope) {
                    Ok(_) | Err(Ctl::Return(_)) | Err(Ctl::Break) => {}
                    Err(Ctl::Panic(m)) => { err = Some(m); break; }
                    Err(Ctl::Exit(c)) => { err = Some(format!("exit({c})")); break; }
                }
                iters += 1;
            }
            let _ = self.out.flush();
            match err {
                None => {
                    let per = start.elapsed().as_nanos() as f64 / iters as f64;
                    println!("bench {label} ... {} /iter ({iters} iteraciones)", human_ns(per));
                }
                Some(m) => {
                    println!("bench {label} ... FALLÓ: {m}");
                    failed += 1;
                }
            }
        }
        failed
    }

    /// Ejecuta las sentencias globales del módulo principal a partir de la `skip`-ésima, usando `root`
    /// como ámbito (las variables persisten entre ejecuciones: así funciona el REPL).
    pub fn exec_globals(&mut self, root: &Rc<Scope>, skip: usize) -> R<usize> {
        let main_idx = self.prog.main;
        self.cur_module = main_idx;
        let mut n = 0;
        for it in &self.prog.modules[main_idx].ast.items {
            if let ItemKind::Stmt(s) = &it.kind {
                if n >= skip {
                    self.exec_stmt(s, root)?;
                }
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
    }
}

fn human_ns(ns: f64) -> String {
    if ns < 1_000.0 {
        format!("{ns:.0} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.2} µs", ns / 1_000.0)
    } else if ns < 1_000_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else {
        format!("{:.2} s", ns / 1_000_000_000.0)
    }
}
