//! NCompiler: implementación de referencia de N# v0.1.
pub mod diag;
pub mod lexer;
pub mod ast;
pub mod parser;
pub mod types;
pub mod program;
pub mod check;
pub mod interp;
pub mod project;
pub mod fmt;
pub mod doc;
pub mod cli;
pub mod repl;

use diag::{Diagnostic, SourceMap};
use program::{Loader, Program, SourceConfig};

pub struct Compiled {
    pub prog: Program,
    pub map: SourceMap,
    pub diags: Vec<Diagnostic>,
}

impl Compiled {
    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.is_error())
    }
    pub fn render(&self) -> String {
        diag::render_all(&self.diags, &self.map)
    }
}

/// Frontend completo: carga (prelude + imports), parsea y verifica.
pub fn compile(display: &str, text: &str, cfg: SourceConfig, opts: check::Options) -> Compiled {
    let mut map = SourceMap::default();
    let (mut prog, mut diags) = Loader::new(&mut map, cfg).load(display, text);
    let has_parse_errors = diags.iter().any(|d| d.is_error());
    if !has_parse_errors {
        diags.extend(check::check_program(&mut prog, opts));
    }
    Compiled { prog, map, diags }
}
