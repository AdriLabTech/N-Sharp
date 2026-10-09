//! Carga de programas: módulo `core` (prelude), módulo principal y sus imports.

use crate::ast::{ItemKind, Module};
use crate::diag::{Diagnostic, SourceMap, Span};
use crate::parser::parse_module;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ModuleInfo {
    pub name: String,
    pub file: u32,
    pub ast: Module,
    pub is_std: bool,
}

pub struct Program {
    pub modules: Vec<ModuleInfo>,
    pub by_name: HashMap<String, usize>,
    pub main: usize,
}

pub const CORE: usize = 0;

pub fn std_source(name: &str) -> Option<&'static str> {
    Some(match name {
        "core" => include_str!("../std/core.ncs"),
        "math" => include_str!("../std/math.ncs"),
        "time" => include_str!("../std/time.ncs"),
        "thread" => include_str!("../std/thread.ncs"),
        "channel" => include_str!("../std/channel.ncs"),
        "fs" => include_str!("../std/fs.ncs"),
        "io" => include_str!("../std/io.ncs"),
        "process" => include_str!("../std/process.ncs"),
        _ => return None,
    })
}

pub const STD_MODULES: &[&str] = &["math", "time", "thread", "channel", "fs", "io", "process"];

pub struct Loader<'a> {
    pub map: &'a mut SourceMap,
    pub diags: Vec<Diagnostic>,
    /// Directorio raíz donde buscar módulos de usuario (`src/` o el directorio del script).
    pub root: Option<PathBuf>,
    /// Módulos añadidos explícitamente (p. ej. dependencias).
    pub extra_roots: Vec<PathBuf>,
    prog: Program,
}

impl<'a> Loader<'a> {
    pub fn new(map: &'a mut SourceMap, root: Option<PathBuf>) -> Self {
        Loader { map, diags: vec![], root, extra_roots: vec![], prog: Program { modules: vec![], by_name: HashMap::new(), main: 1 } }
    }

    /// Carga `core` y el módulo principal (con todos sus imports transitivos).
    pub fn load(mut self, main_name: &str, main_text: &str) -> (Program, Vec<Diagnostic>) {
        self.add_module("core".to_string(), "<std>/core.ncs".to_string(), std_source("core").unwrap().to_string(), true);
        let main = self.add_module(main_name.to_string(), main_name.to_string(), main_text.to_string(), false);
        self.prog.main = main;
        (self.prog, self.diags)
    }

    fn add_module(&mut self, name: String, display: String, text: String, is_std: bool) -> usize {
        let file = self.map.add(display, text.clone());
        let (ast, diags) = parse_module(file, &text);
        self.diags.extend(diags);
        let idx = self.prog.modules.len();
        self.prog.by_name.insert(name.clone(), idx);
        let imports: Vec<(Vec<String>, Span)> = ast
            .items
            .iter()
            .filter_map(|it| match &it.kind {
                ItemKind::Import(paths) => Some(paths.iter().map(|p| (p.path.iter().map(|i| i.name.clone()).collect::<Vec<_>>(), p.path[0].span)).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect();
        self.prog.modules.push(ModuleInfo { name, file, ast, is_std });
        for (path, span) in imports {
            self.resolve_import(&path, span);
        }
        idx
    }

    /// Intenta cargar el módulo más largo que sea prefijo de `path` (el resto es un elemento del módulo).
    fn resolve_import(&mut self, path: &[String], span: Span) {
        for len in (1..=path.len()).rev() {
            if self.ensure_module(&path[..len]) {
                return;
            }
        }
        self.diags.push(
            Diagnostic::error(span, format!("no se encuentra el módulo `{}`", path[0]))
                .with_label("módulo no encontrado")
                .with_note(format!("módulos de la biblioteca estándar: {}", STD_MODULES.join(", "))),
        );
    }

    fn find_user(&self, path: &[String]) -> Option<PathBuf> {
        let roots = self.root.iter().chain(self.extra_roots.iter());
        for root in roots {
            let mut p = root.clone();
            for s in path {
                p.push(s);
            }
            let file = p.with_extension("ncs");
            if file.is_file() {
                return Some(file);
            }
            let modfile = p.join("mod.ncs");
            if modfile.is_file() {
                return Some(modfile);
            }
        }
        None
    }

    fn ensure_module(&mut self, path: &[String]) -> bool {
        let name = path.join(".");
        if self.prog.by_name.contains_key(&name) {
            return true;
        }
        if let Some(file) = self.find_user(path) {
            match std::fs::read_to_string(&file) {
                Ok(text) => {
                    self.add_module(name, file.display().to_string(), text, false);
                }
                Err(e) => {
                    self.diags.push(Diagnostic::error(Span::default(), format!("no se pudo leer `{}`: {e}", file.display())));
                }
            }
            return true;
        }
        if path.len() == 1 {
            if let Some(src) = std_source(&path[0]) {
                if path[0] != "core" {
                    self.add_module(name, format!("<std>/{}.ncs", path[0]), src.to_string(), true);
                    return true;
                }
            }
        }
        false
    }
}
