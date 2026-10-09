//! Análisis semántico: resolución de nombres, tipos, inferencia local, traits, patrones,
//! asignación definida, `unsafe`, `async` y diagnósticos.

mod body;
mod call;
mod decls;
mod expr;
mod pat;
mod unify;

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::program::Program;
use crate::types::Ty;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `ncom archivo.ncs`: se permite código global.
    Script,
    /// `ncom build proyecto/`: requiere `fn main`; el código global está prohibido.
    Project,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub mode: Mode,
    /// Categorías de warning silenciadas.
    pub allow: HashSet<String>,
    /// Categorías de warning promovidas a error.
    pub deny: HashSet<String>,
    /// Exigir `fn main` (falso para `ncom check` de librerías y para el REPL).
    pub require_main: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { mode: Mode::Script, allow: HashSet::new(), deny: HashSet::new(), require_main: false }
    }
}

pub const WARNING_CATEGORIES: &[&str] = &["unused-variable", "unreachable-code", "unreachable-pattern"];

#[derive(Clone, Debug)]
pub struct TraitRef {
    pub key: String,
    pub args: Vec<Ty>,
}

#[derive(Clone, Debug)]
pub struct GenericInfo {
    pub name: Rc<str>,
    pub bounds: Vec<TraitRef>,
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
    pub is_pub: bool,
}

#[derive(Clone, Debug)]
pub struct VariantInfo {
    pub name: String,
    pub fields: Vec<Ty>,
}

#[derive(Clone, Debug)]
pub enum DefKind {
    Struct(Vec<FieldInfo>),
    Enum(Vec<VariantInfo>),
    Alias,
    Trait,
}

#[derive(Clone, Debug)]
pub struct TypeDef {
    pub name: String,
    pub key: String,
    pub module: usize,
    pub generics: Vec<GenericInfo>,
    pub kind: DefKind,
    pub is_pub: bool,
    pub span: Span,
    /// Tipo del runtime sin campos (List, Map, ...): no se puede construir con un literal.
    pub opaque: bool,
}

#[derive(Clone, Debug)]
pub struct FnSig {
    pub name: String,
    pub module: usize,
    pub generics: Vec<GenericInfo>,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub variadic: bool,
    pub is_unsafe: bool,
    pub is_async: bool,
    pub is_pub: bool,
    pub is_extern: bool,
    pub has_body: bool,
    /// Se puede invocar como `Tipo.nombre(..)` (no usa `self`).
    pub is_static: bool,
    pub span: Span,
}

impl FnSig {
    /// Número mínimo de argumentos: los parámetros opcionales (`T?`) finales pueden omitirse.
    pub fn required(&self) -> usize {
        let mut n = self.params.len();
        while n > 0 && self.params[n - 1].is_option() {
            n -= 1;
        }
        n
    }
}

#[derive(Clone, Debug)]
pub struct ImplInfo {
    pub module: usize,
    pub generics: Vec<GenericInfo>,
    pub trait_ref: Option<TraitRef>,
    pub target: Ty,
    /// (nombre, índice en `sigs`)
    pub methods: Vec<(String, usize)>,
}

#[derive(Clone, Debug)]
pub struct TraitInfo {
    pub key: String,
    pub name: String,
    pub module: usize,
    pub generics: Vec<GenericInfo>,
    /// (nombre, índice en `sigs`)
    pub methods: Vec<(String, usize)>,
}

#[derive(Clone, Debug)]
pub enum ImportTarget {
    Module(usize),
    Item(usize, String),
}

#[derive(Default, Clone)]
pub struct ModScope {
    pub fns: HashMap<String, Vec<usize>>,
    pub types: HashMap<String, String>,
    pub consts: HashMap<String, (Ty, bool)>,
    pub imports: HashMap<String, ImportTarget>,
    pub wildcards: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct VarInfo {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
    pub used: bool,
    pub warn_unused: bool,
}

pub struct FnCtx {
    pub ret: Ty,
    pub is_async: bool,
    pub unsafe_depth: u32,
    pub loop_depth: u32,
    /// Para cada bucle activo: ¿se ha visto un `break`?
    pub breaks: Vec<bool>,
    pub generics: Vec<GenericInfo>,
    pub self_ty: Option<Ty>,
    pub module: usize,
    pub is_main: bool,
    pub scope_base: usize,
}

pub struct Checker {
    pub diags: Vec<Diagnostic>,
    pub opts: Options,
    mod_names: Vec<String>,
    mod_std: Vec<bool>,
    mods: Vec<ModScope>,
    types: HashMap<String, TypeDef>,
    traits: HashMap<String, TraitInfo>,
    alias_defs: HashMap<String, (usize, Vec<String>, TypeExpr)>,
    sigs: Vec<FnSig>,
    impls: Vec<ImplInfo>,
    impls_by_head: HashMap<String, Vec<usize>>,
    trait_impls: HashSet<(String, String)>,
    // estado de inferencia y de cuerpo de función
    infer: Vec<Option<Ty>>,
    trail: Vec<u32>,
    pending_bounds: Vec<(Ty, TraitRef, Span)>,
    pending_infer: Vec<(Ty, Span, String)>,
    fcx: Vec<FnCtx>,
    vars: Vec<VarInfo>,
    scopes: Vec<HashMap<String, usize>>,
    maybe_uninit: HashSet<usize>,
    cur_module: usize,
    /// Por módulo y por item: ids de `sigs` (funciones, métodos de impl/trait).
    item_sigs: Vec<Vec<Vec<usize>>>,
    impl_of_item: HashMap<(usize, usize), usize>,
    /// Consumido por `check_expr`: la expresión es una sentencia y su valor se descarta.
    discard: bool,
    last_tail_ty: Option<Ty>,
    /// Tipos resueltos de los patrones `x $tipo` (por offset del identificador).
    typed_binds: HashMap<(u32, u32), Ty>,
}

/// Verifica el programa completo (módulo `core` incluido). Muta el AST (resolución y coerciones).
pub fn check_program(prog: &mut Program, opts: Options) -> Vec<Diagnostic> {
    let mut asts: Vec<Module> = prog.modules.iter_mut().map(|m| std::mem::take(&mut m.ast)).collect();
    let mut ck = Checker {
        diags: vec![],
        opts,
        mod_names: prog.modules.iter().map(|m| m.name.clone()).collect(),
        mod_std: prog.modules.iter().map(|m| m.is_std).collect(),
        mods: vec![ModScope::default(); prog.modules.len()],
        types: HashMap::new(),
        traits: HashMap::new(),
        alias_defs: HashMap::new(),
        sigs: vec![],
        impls: vec![],
        impls_by_head: HashMap::new(),
        trait_impls: HashSet::new(),
        infer: vec![],
        trail: vec![],
        pending_bounds: vec![],
        pending_infer: vec![],
        fcx: vec![],
        vars: vec![],
        scopes: vec![],
        maybe_uninit: HashSet::new(),
        cur_module: 0,
        item_sigs: vec![],
        impl_of_item: HashMap::new(),
        discard: false,
        last_tail_ty: None,
        typed_binds: HashMap::new(),
    };
    ck.collect_all(&mut asts);
    let has_decl_errors = ck.diags.iter().any(|d| d.is_error());
    if !has_decl_errors || true {
        for mi in 0..asts.len() {
            let mut ast = std::mem::take(&mut asts[mi]);
            ck.check_module(mi, &mut ast, prog.main);
            asts[mi] = ast;
        }
    }
    for (m, ast) in prog.modules.iter_mut().zip(asts) {
        m.ast = ast;
    }
    ck.finish()
}

impl Checker {
    fn finish(mut self) -> Vec<Diagnostic> {
        // promover warnings denegados; filtrar permitidos
        let deny = self.opts.deny.clone();
        let mut out = Vec::new();
        for mut d in std::mem::take(&mut self.diags) {
            if !d.is_error() {
                if let Some(cat) = d.notes.iter().find_map(|n| n.strip_prefix("categoría: ").map(|s| s.to_string())) {
                    if deny.contains(&cat) {
                        d.severity = crate::diag::Severity::Error;
                    }
                    d.notes.retain(|n| !n.starts_with("categoría: "));
                }
            }
            out.push(d);
        }
        out
    }

    pub fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg));
    }

    pub fn err_label(&mut self, span: Span, msg: impl Into<String>, label: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, msg).with_label(label));
    }

    pub fn warn(&mut self, cat: &str, span: Span, msg: impl Into<String>) {
        if self.mod_std.get(self.cur_module).copied().unwrap_or(false) || self.opts.allow.contains(cat) {
            return;
        }
        self.diags.push(Diagnostic::warning(span, msg).with_note(format!("categoría: {cat}")));
    }

    pub fn suggest(&self, name: &str, candidates: impl Iterator<Item = String>) -> Option<String> {
        let mut best: Option<(usize, String)> = None;
        for c in candidates {
            let d = edit_distance(name, &c);
            if d <= (name.chars().count() / 3).max(1) && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, c));
            }
        }
        best.map(|(_, c)| c)
    }
}

pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}
