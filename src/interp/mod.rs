//! Backend intérprete de N#: evalúa el AST ya verificado.

mod display;
mod intrinsics;
mod runner;
pub mod value;

use crate::ast::*;
use crate::diag::{SourceMap, Span};
use crate::program::Program;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::Rc;
use value::*;

pub enum Ctl {
    Return(Value),
    Break,
    Panic(String),
    Exit(i32),
}

pub type R<T> = Result<T, Ctl>;

pub type IntrinsicFn = fn(&mut Interp, Option<Place>, Vec<Value>) -> R<Value>;

/// Etiqueta de tipo simplificada para resolver sobrecargas en tiempo de ejecución.
#[derive(Debug, Clone)]
pub enum Tag {
    Any,
    Int,
    Float,
    Bool,
    Char,
    Str,
    Nothing,
    Named(String),
    Array,
    Tuple,
    Func,
    Ref(Box<Tag>),
    Ptr,
    Opt(Box<Tag>),
    Union(Vec<Tag>),
}

/// Cuanto más específica la etiqueta, mayor puntuación (para elegir entre sobrecargas aplicables).
fn tag_specificity(t: &Tag) -> i32 {
    match t {
        Tag::Any => 0,
        Tag::Union(_) | Tag::Opt(_) => 1,
        Tag::Ref(_) => 2,
        _ => 3,
    }
}

fn tag_of(t: &TypeExpr, generics: &[String]) -> Tag {
    match &t.kind {
        TypeKind::Prim(p) => match p {
            Prim::Int => Tag::Int,
            Prim::Float => Tag::Float,
            Prim::Bool => Tag::Bool,
            Prim::Char => Tag::Char,
            Prim::Str => Tag::Str,
            Prim::Nothing => Tag::Nothing,
        },
        TypeKind::Named { path, .. } => {
            let n = &path.last().unwrap().name;
            if generics.iter().any(|g| g == n) || n == "Self" { Tag::Any } else { Tag::Named(n.clone()) }
        }
        TypeKind::Ref(i) => Tag::Ref(Box::new(tag_of(i, generics))),
        TypeKind::Ptr(_) => Tag::Ptr,
        TypeKind::Optional(i) => Tag::Opt(Box::new(tag_of(i, generics))),
        TypeKind::Union(v) => Tag::Union(v.iter().map(|x| tag_of(x, generics)).collect()),
        TypeKind::Func { .. } => Tag::Func,
        TypeKind::Tuple(_) => Tag::Tuple,
        TypeKind::Array { .. } => Tag::Array,
    }
}

#[derive(Debug)]
pub struct FnDef {
    pub decl: &'static FnDecl,
    pub module: usize,
    pub name: String,
    pub owner: Option<String>,
    pub tags: Vec<Tag>,
    pub required: usize,
    pub intrinsic: Option<IntrinsicFn>,
    pub intrinsic_key: String,
}

pub struct ThreadSlot {
    pub task: Option<Value>,
    pub result: Option<Value>,
}

pub struct ConstCell {
    pub expr: &'static Expr,
    pub module: usize,
    pub cell: Cell,
    pub state: std::cell::Cell<u8>,
}

pub struct ModRt {
    pub name: String,
    pub fns: HashMap<String, Rc<Vec<Rc<FnDef>>>>,
    pub consts: HashMap<String, Rc<ConstCell>>,
}

struct Frame {
    def: Option<Rc<FnDef>>,
    span: Span,
}

impl Frame {
    fn label(&self) -> String {
        match &self.def {
            None => "<closure>".to_string(),
            Some(d) => match &d.owner {
                Some(o) => format!("{}.{}", o.rsplit("::").next().unwrap_or(o), d.name),
                None => d.name.clone(),
            },
        }
    }
}

pub struct Interp {
    pub prog: &'static Program,
    pub map: &'static SourceMap,
    pub modules: Vec<ModRt>,
    pub methods: HashMap<(String, String), Rc<Vec<Rc<FnDef>>>>,
    pub structs: HashMap<String, Rc<StructInfo>>,
    pub enums: HashMap<String, Rc<EnumInfo>>,
    pub drop_types: HashSet<String>,
    pub has_drop: bool,
    pub opt_info: Rc<EnumInfo>,
    pub res_info: Rc<EnumInfo>,
    frames: Vec<Frame>,
    pub depth: usize,
    pub out: std::io::BufWriter<std::io::Stdout>,
    /// 0 = release (mensajes mínimos), 1 = normal, 2 = debug (backtrace completo).
    pub verbosity: u8,
    pub cur_span: Span,
    pub args: Vec<String>,
    pub heap: Vec<Option<Vec<Cell>>>,
    pub threads: Vec<ThreadSlot>,
    pub tests: Vec<(String, &'static Block, usize)>,
    pub benches: Vec<(String, &'static Block, usize)>,
    pub only_main_tests: bool,
    pub(crate) cur_module: usize,
}

const MAX_DEPTH: usize = 12_000;

impl Interp {
    pub fn new(prog: &'static Program, map: &'static SourceMap, verbosity: u8, args: Vec<String>) -> Result<Interp, String> {
        let opt_info = Rc::new(EnumInfo { key: "Option".into(), name: "Option".into(), variants: vec![("Some".into(), 1), ("None".into(), 0)] });
        let res_info = Rc::new(EnumInfo { key: "Result".into(), name: "Result".into(), variants: vec![("Ok".into(), 1), ("Error".into(), 1)] });
        let mut it = Interp {
            prog,
            map,
            modules: vec![],
            methods: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            drop_types: HashSet::new(),
            has_drop: false,
            opt_info: opt_info.clone(),
            res_info: res_info.clone(),
            frames: vec![],
            depth: 0,
            out: std::io::BufWriter::with_capacity(1 << 16, std::io::stdout()),
            verbosity,
            cur_span: Span::default(),
            args,
            heap: vec![],
            threads: vec![],
            tests: vec![],
            benches: vec![],
            only_main_tests: false,
            cur_module: 0,
        };
        it.enums.insert("Option".into(), opt_info);
        it.enums.insert("Result".into(), res_info);
        it.load()?;
        Ok(it)
    }

    fn load(&mut self) -> Result<(), String> {
        let table = intrinsics::table();
        let prog = self.prog;
        let mut mod_fns: Vec<HashMap<String, Vec<Rc<FnDef>>>> = Vec::new();
        let mut methods: HashMap<(String, String), Vec<Rc<FnDef>>> = HashMap::new();
        let mut trait_decls: HashMap<String, (&'static TraitDecl, usize)> = HashMap::new();
        for (mi, m) in prog.modules.iter().enumerate() {
            for it in &m.ast.items {
                if let ItemKind::Trait(t) = &it.kind {
                    trait_decls.insert(crate::types::qualify(&m.name, &t.name.name), (t, mi));
                }
            }
        }
        let mk = |decl: &'static FnDecl, mi: usize, owner: Option<String>, generics: &[String], modname: &str| -> Result<Rc<FnDef>, String> {
            let mut gens: Vec<String> = generics.to_vec();
            gens.extend(decl.generics.iter().map(|g| g.name.name.clone()));
            let tags: Vec<Tag> = decl.params.iter().map(|p| tag_of(&p.ty, &gens)).collect();
            let mut required = tags.len();
            while required > 0 && matches!(tags[required - 1], Tag::Opt(_)) {
                required -= 1;
            }
            let key = match &owner {
                Some(k) => format!("{k}.{}", decl.name.name),
                None => if modname == "core" { format!("global.{}", decl.name.name) } else { format!("{modname}.{}", decl.name.name) },
            };
            let intrinsic = if decl.body.is_none() {
                match table.get(key.as_str()) {
                    Some(f) => Some(*f),
                    None if decl.is_extern => Some(intrinsics::extern_c as IntrinsicFn),
                    None => {
                        // métodos de traits sin cuerpo (requeridos) no tienen intrínseca
                        None
                    }
                }
            } else {
                None
            };
            Ok(Rc::new(FnDef { decl, module: mi, name: decl.name.name.clone(), owner, tags, required, intrinsic, intrinsic_key: key }))
        };
        let mut missing: Vec<String> = Vec::new();
        for (mi, m) in prog.modules.iter().enumerate() {
            let mut fns: HashMap<String, Vec<Rc<FnDef>>> = HashMap::new();
            for it in &m.ast.items {
                match &it.kind {
                    ItemKind::Fn(f) => {
                        let d = mk(f, mi, None, &[], &m.name)?;
                        if f.body.is_none() && d.intrinsic.is_none() {
                            missing.push(d.intrinsic_key.clone());
                        }
                        fns.entry(f.name.name.clone()).or_default().push(d);
                    }
                    ItemKind::Extern { fns: efs, .. } => {
                        for f in efs {
                            let d = mk(f, mi, None, &[], &m.name)?;
                            fns.entry(f.name.name.clone()).or_default().push(d);
                        }
                    }
                    ItemKind::Struct(s) => {
                        let key = crate::types::qualify(&m.name, &s.name.name);
                        self.structs.insert(key.clone(), Rc::new(StructInfo { key, name: s.name.name.clone(), fields: s.fields.iter().map(|f| f.name.name.clone()).collect() }));
                    }
                    ItemKind::Enum(e) => {
                        let key = crate::types::qualify(&m.name, &e.name.name);
                        if key != "Option" && key != "Result" {
                            self.enums.insert(key.clone(), Rc::new(EnumInfo { key, name: e.name.name.clone(), variants: e.variants.iter().map(|v| (v.name.name.clone(), v.fields.len())).collect() }));
                        }
                    }
                    ItemKind::Test { name, body } => self.tests.push((name.clone(), body, mi)),
                    ItemKind::Bench { name, body } => self.benches.push((name.clone(), body, mi)),
                    _ => {}
                }
            }
            mod_fns.push(fns);
        }
        for (mi, m) in prog.modules.iter().enumerate() {
            for it in &m.ast.items {
                let ItemKind::Impl(imp) = &it.kind else { continue };
                let gens: Vec<String> = imp.generics.iter().map(|g| g.name.name.clone()).collect();
                // los genéricos implícitos (`impl Stack<T>`) también son etiquetas «cualquiera»
                let mut gens = gens;
                gens.extend(implicit_generics(&imp.target));
                let mut defined: HashSet<String> = HashSet::new();
                for f in &imp.methods {
                    let d = mk(f, mi, Some(imp.key.clone()), &gens, &m.name)?;
                    if f.body.is_none() && d.intrinsic.is_none() {
                        missing.push(d.intrinsic_key.clone());
                    }
                    defined.insert(f.name.name.clone());
                    methods.entry((imp.key.clone(), f.name.name.clone())).or_default().push(d);
                }
                if let Some(tk) = &imp.trait_key {
                    if tk == "Drop" {
                        self.drop_types.insert(imp.key.clone());
                        self.has_drop = true;
                    }
                    if let Some((td, tmi)) = trait_decls.get(tk) {
                        for f in &td.methods {
                            if f.body.is_some() && !defined.contains(&f.name.name) {
                                let d = mk(f, *tmi, Some(imp.key.clone()), &gens, &prog.modules[*tmi].name)?;
                                methods.entry((imp.key.clone(), f.name.name.clone())).or_default().push(d);
                            }
                        }
                    }
                }
            }
        }
        if !missing.is_empty() {
            return Err(format!("intrínsecas sin implementar en el backend: {}", missing.join(", ")));
        }
        self.methods = methods.into_iter().map(|(k, v)| (k, Rc::new(v))).collect();
        for (mi, m) in prog.modules.iter().enumerate() {
            let mut consts = HashMap::new();
            for it in &m.ast.items {
                if let ItemKind::Const(c) | ItemKind::Static(c) = &it.kind {
                    consts.insert(c.name.name.clone(), Rc::new(ConstCell { expr: &c.value, module: mi, cell: cell(Value::Nothing), state: std::cell::Cell::new(0) }));
                }
            }
            self.modules.push(ModRt { name: m.name.clone(), fns: std::mem::take(&mut mod_fns[mi]).into_iter().map(|(k, v)| (k, Rc::new(v))).collect(), consts });
        }
        Ok(())
    }

    // ---------- utilidades de valores ----------

    pub fn none(&self) -> Value {
        Value::Variant(Rc::new(VariantVal { info: self.opt_info.clone(), idx: 1, vals: vec![] }))
    }
    pub fn some(&self, v: Value) -> Value {
        Value::Variant(Rc::new(VariantVal { info: self.opt_info.clone(), idx: 0, vals: vec![v] }))
    }
    pub fn ok(&self, v: Value) -> Value {
        Value::Variant(Rc::new(VariantVal { info: self.res_info.clone(), idx: 0, vals: vec![v] }))
    }
    pub fn err(&self, v: Value) -> Value {
        Value::Variant(Rc::new(VariantVal { info: self.res_info.clone(), idx: 1, vals: vec![v] }))
    }
    pub fn opt(&self, v: Option<Value>) -> Value {
        match v {
            Some(x) => self.some(x),
            None => self.none(),
        }
    }

    pub fn panic<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(Ctl::Panic(msg.into()))
    }

    /// Sigue referencias, `Box` y `Rc` hasta el valor final.
    pub fn deref_value(&self, v: &Value) -> R<Value> {
        let mut cur = v.clone();
        for _ in 0..64 {
            match &cur {
                Value::Ref(r) => {
                    cur = match read_path(&r.cell, &r.path) {
                        Ok(x) => x,
                        Err(m) => return self.panic(m),
                    }
                }
                Value::Box(c) | Value::Rc(c) => {
                    let x = c.borrow().clone();
                    cur = x;
                }
                _ => return Ok(cur),
            }
        }
        Ok(cur)
    }

    /// Sólo sigue `Ref` (no Box/Rc).
    pub fn deref_ref(&self, v: &Value) -> R<Value> {
        let mut cur = v.clone();
        while let Value::Ref(r) = &cur {
            let next = match read_path(&r.cell, &r.path) {
                Ok(x) => x,
                Err(m) => return self.panic(m),
            };
            cur = next;
        }
        Ok(cur)
    }

    fn pr<T>(&self, r: PResult<T>) -> R<T> {
        r.map_err(Ctl::Panic)
    }

    // ---------- ejecución de programas ----------

    /// Ejecuta el programa: sentencias globales o `main`. Devuelve el código de salida.
    pub fn run_main(&mut self) -> i32 {
        let main_idx = self.prog.main;
        let r = self.run_main_inner(main_idx);
        let code = match r {
            Ok(c) => c,
            Err(Ctl::Exit(c)) => c,
            Err(Ctl::Panic(m)) => {
                self.report_panic(&m);
                101
            }
            Err(_) => 0,
        };
        let _ = self.out.flush();
        code
    }

    fn run_main_inner(&mut self, main_idx: usize) -> R<i32> {
        self.cur_module = main_idx;
        let root = Scope::new(None);
        // sentencias globales (script mode)
        let mut stmts = false;
        for it in &self.prog.modules[main_idx].ast.items {
            if let ItemKind::Stmt(s) = &it.kind {
                stmts = true;
                self.exec_stmt(s, &root)?;
            }
        }
        if !stmts {
            if let Some(defs) = self.modules[main_idx].fns.get("main").cloned() {
                let r = self.call_def(&defs[0], vec![], None, Span::default())?;
                self.finish_threads()?;
                return self.exit_code_of(&r);
            }
        }
        self.finish_threads()?;
        Ok(0)
    }

    fn exit_code_of(&mut self, r: &Value) -> R<i32> {
        match r {
            Value::Int(i) => Ok(*i as i32),
            Value::Variant(v) if v.info.key == "Result" && v.idx == 1 => {
                let msg = self.display(&v.vals[0])?;
                let _ = self.out.flush();
                eprintln!("Error: {msg}");
                Ok(1)
            }
            _ => Ok(0),
        }
    }

    pub fn report_panic(&mut self, msg: &str) {
        let _ = self.out.flush();
        eprintln!("panic: {msg}");
        if self.verbosity == 0 {
            return;
        }
        let sp = self.cur_span;
        if sp != Span::default() && (sp.file as usize) < self.map.files.len() {
            let file = self.map.get(sp.file);
            let (l, c) = file.line_col(sp.lo as usize);
            eprintln!("  --> {}:{}:{}", file.name, l, c);
            eprintln!("   | {}", file.line_text(l).trim());
        }
        if self.verbosity >= 2 {
            eprintln!("backtrace (llamadas más recientes primero):");
            for f in self.frames.iter().rev().take(60) {
                if f.span != Span::default() {
                    let file = self.map.get(f.span.file);
                    let (l, c) = file.line_col(f.span.lo as usize);
                    eprintln!("  en {} (llamada en {}:{}:{})", f.label(), file.name, l, c);
                } else {
                    eprintln!("  en {}", f.label());
                }
            }
        }
    }

    pub fn finish_threads(&mut self) -> R<()> {
        while let Some(id) = self.threads.iter().position(|t| t.task.is_some()) {
            self.run_thread(id)?;
        }
        Ok(())
    }

    // ---------- evaluación ----------

    fn lookup_def_set(&self, key: &str, name: &str) -> Option<Rc<Vec<Rc<FnDef>>>> {
        self.methods.get(&(key.to_string(), name.to_string())).cloned()
    }

    fn const_value(&mut self, module: usize, name: &str) -> R<Value> {
        let cc = self.modules[module].consts.get(name).cloned().unwrap();
        match cc.state.get() {
            2 => Ok(cc.cell.borrow().clone()),
            1 => self.panic(format!("dependencia circular al evaluar la constante `{name}`")),
            _ => {
                cc.state.set(1);
                let env = Scope::new(None);
                let saved = std::mem::replace(&mut self.cur_module, cc.module);
                let v = self.eval(cc.expr, &env);
                self.cur_module = saved;
                let v = v?;
                *cc.cell.borrow_mut() = v.clone();
                cc.state.set(2);
                Ok(v)
            }
        }
    }

    fn const_place(&mut self, module: usize, name: &str) -> R<Place> {
        self.const_value(module, name)?;
        let cc = self.modules[module].consts.get(name).cloned().unwrap();
        Ok(Place { cell: cc.cell.clone(), path: vec![] })
    }

    pub fn call_value(&mut self, f: &Value, args: Vec<Value>, span: Span) -> R<Value> {
        let f = self.deref_value(f)?;
        match &f {
            Value::Func(fv) => self.call_func(fv.clone(), args, None, span),
            other => self.panic(format!("el valor `{}` no es invocable", other.type_key())),
        }
    }

    pub fn call_func(&mut self, f: Rc<FuncVal>, args: Vec<Value>, recv: Option<Place>, span: Span) -> R<Value> {
        match &*f {
            FuncVal::Defs(defs) => {
                let def = self.select_def(defs, &args)?;
                self.call_def(&def, args, recv, span)
            }
            FuncVal::Ctor { info, idx } => Ok(Value::Variant(Rc::new(VariantVal { info: info.clone(), idx: *idx, vals: args }))),
            FuncVal::Closure(c) => self.call_closure(c, args, span),
        }
    }

    fn call_closure(&mut self, c: &Rc<ClosureDef>, args: Vec<Value>, span: Span) -> R<Value> {
        self.enter(None, span)?;
        let scope = Scope::new(Some(c.env.clone()));
        for (p, a) in c.params.iter().zip(args) {
            scope.declare(&p.name.name, a);
        }
        let saved = std::mem::replace(&mut self.cur_module, c.module);
        let r = match &c.body.kind {
            ExprKind::Block(b) => self.eval_block_in(b, &scope),
            _ => self.eval(c.body, &scope),
        };
        self.cur_module = saved;
        if self.has_drop && !is_abort(&r) {
            self.drop_scope(&scope)?;
        }
        self.frames.pop();
        self.depth -= 1;
        match r {
            Ok(v) => Ok(v),
            Err(Ctl::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    fn enter(&mut self, def: Option<&Rc<FnDef>>, span: Span) -> R<()> {
        if self.depth >= MAX_DEPTH {
            return self.panic("desbordamiento de pila: recursión demasiado profunda");
        }
        self.depth += 1;
        self.frames.push(Frame { def: def.cloned(), span });
        Ok(())
    }

    fn select_def(&self, defs: &Rc<Vec<Rc<FnDef>>>, args: &[Value]) -> R<Rc<FnDef>> {
        if defs.len() == 1 {
            return Ok(defs[0].clone());
        }
        let n = args.len();
        let mut fit: Vec<&Rc<FnDef>> = defs.iter().filter(|d| n >= d.required && n <= d.tags.len().max(n)).filter(|d| n <= d.tags.len() || d.decl.variadic).collect();
        if fit.len() > 1 {
            let mut scored: Vec<(i32, &Rc<FnDef>)> = vec![];
            for d in &fit {
                if d.tags.iter().zip(args).all(|(t, a)| self.tag_matches(t, a)) {
                    let sc: i32 = d.tags.iter().map(tag_specificity).sum();
                    scored.push((sc, d));
                }
            }
            if let Some(best) = scored.iter().map(|(s, _)| *s).max() {
                fit = scored.into_iter().filter(|(s, _)| *s == best).map(|(_, d)| d).collect();
            }
        }
        match fit.first() {
            Some(d) => Ok((*d).clone()),
            None => self.panic("no hay una sobrecarga aplicable en tiempo de ejecución"),
        }
    }

    fn tag_matches(&self, t: &Tag, v: &Value) -> bool {
        match (t, v) {
            (Tag::Any, _) => true,
            (Tag::Int, Value::Int(_)) | (Tag::Float, Value::Float(_)) | (Tag::Bool, Value::Bool(_)) | (Tag::Char, Value::Char(_)) | (Tag::Str, Value::Str(_)) => true,
            (Tag::Nothing, Value::Nothing) => true,
            (Tag::Array, Value::Array(_)) | (Tag::Tuple, Value::Tuple(_)) | (Tag::Func, Value::Func(_)) | (Tag::Ptr, Value::Ptr(..)) => true,
            (Tag::Ref(inner), Value::Ref(r)) => match read_path(&r.cell, &r.path) {
                Ok(x) => self.tag_matches(inner, &x),
                Err(_) => false,
            },
            (Tag::Ref(inner), other) => self.tag_matches(inner, other),
            (Tag::Opt(inner), Value::Variant(vv)) if vv.info.key == "Option" => vv.idx == 1 || self.tag_matches(inner, &vv.vals[0]),
            (Tag::Union(ts), v) => ts.iter().any(|t| self.tag_matches(t, v)),
            (Tag::Named(n), v) => match v {
                Value::Struct(s) => s.info.name == *n,
                Value::Variant(vv) => vv.info.name == *n,
                Value::List(_) => n == "List",
                Value::Map(_) => n == "Map",
                Value::Set(_) => n == "Set",
                Value::Box(_) => n == "Box",
                Value::Rc(_) => n == "Rc",
                Value::Future(_) => n == "Future",
                Value::Ref(r) => read_path(&r.cell, &r.path).map(|x| self.tag_matches(t, &x)).unwrap_or(false),
                _ => false,
            },
            _ => false,
        }
    }

    pub fn call_def(&mut self, def: &Rc<FnDef>, args: Vec<Value>, recv: Option<Place>, span: Span) -> R<Value> {
        if def.decl.is_extern && def.decl.body.is_none() {
            return self.call_extern(def, args);
        }
        if let Some(f) = def.intrinsic {
            return f(self, recv, args);
        }
        let Some(body) = def.decl.body.as_ref() else {
            return self.panic(format!("`{}` no tiene implementación", def.name));
        };
        if def.decl.is_async {
            let fut = FutureState { thunk: Some((Rc::new(FuncVal::Defs(Rc::new(vec![def.clone()]))), args, recv)), result: None };
            return Ok(Value::Future(Rc::new(RefCell::new(fut))));
        }
        self.enter(Some(def), span)?;
        let scope = Scope::new(None);
        if let Some(p) = recv {
            scope.declare("self", p.to_ref());
        }
        for (p, a) in def.decl.params.iter().zip(args) {
            scope.declare(&p.name.name, a);
        }
        let saved = std::mem::replace(&mut self.cur_module, def.module);
        let r = self.eval_block_in(body, &scope);
        self.cur_module = saved;
        if self.has_drop && !is_abort(&r) {
            self.drop_scope(&scope)?;
        }
        self.frames.pop();
        self.depth -= 1;
        match r {
            Ok(v) => Ok(v),
            Err(Ctl::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    pub fn run_future(&mut self, fut: &Rc<RefCell<FutureState>>) -> R<Value> {
        if let Some(r) = fut.borrow().result.clone() {
            return Ok(r);
        }
        let thunk = fut.borrow_mut().thunk.take();
        let Some((f, args, recv)) = thunk else { return self.panic("future consumido dos veces") };
        // se ejecuta como función normal (sin volver a crear un future)
        let v = match &*f {
            FuncVal::Defs(defs) => {
                let def = defs[0].clone();
                let body = def.decl.body.as_ref().unwrap();
                self.enter(Some(&def), Span::default())?;
                let scope = Scope::new(None);
                if let Some(p) = recv {
                    scope.declare("self", p.to_ref());
                }
                for (p, a) in def.decl.params.iter().zip(args) {
                    scope.declare(&p.name.name, a);
                }
                let saved = std::mem::replace(&mut self.cur_module, def.module);
                let r = self.eval_block_in(body, &scope);
                self.cur_module = saved;
                self.frames.pop();
                self.depth -= 1;
                match r {
                    Ok(v) => v,
                    Err(Ctl::Return(v)) => v,
                    Err(e) => return Err(e),
                }
            }
            _ => self.call_func(f.clone(), args, recv, Span::default())?,
        };
        fut.borrow_mut().result = Some(v.clone());
        Ok(v)
    }

    // ---------- bloques y sentencias ----------

    pub fn eval_block(&mut self, b: &'static Block, parent: &Rc<Scope>) -> R<Value> {
        if !b.stmts.iter().any(|s| !matches!(s.kind, StmtKind::Expr(_))) {
            return self.eval_block_in(b, parent);
        }
        let scope = Scope::new(Some(parent.clone()));
        let r = self.eval_block_in(b, &scope);
        if self.has_drop && !is_abort(&r) {
            self.drop_scope(&scope)?;
        }
        r
    }

    pub(crate) fn eval_block_in(&mut self, b: &'static Block, scope: &Rc<Scope>) -> R<Value> {
        for s in &b.stmts {
            self.exec_stmt(s, scope)?;
        }
        match &b.tail {
            Some(t) => self.eval(t, scope),
            None => Ok(Value::Nothing),
        }
    }

    pub(crate) fn exec_stmt(&mut self, s: &'static Stmt, env: &Rc<Scope>) -> R<()> {
        match &s.kind {
            StmtKind::Expr(e) => {
                self.eval(e, env)?;
            }
            StmtKind::Let { name, init, .. } => {
                let v = match init {
                    Some(e) => self.eval(e, env)?,
                    None => Value::Nothing,
                };
                env.declare(&name.name, v);
            }
            StmtKind::Destructure { targets, init } => {
                let v = self.eval(init, env)?;
                let v = self.deref_ref(&v)?;
                let Value::Tuple(items) = v else { return self.panic("se esperaba una tupla") };
                for (t, x) in targets.iter().zip(items.iter()) {
                    env.declare(&t.name.name, x.clone());
                }
            }
        }
        Ok(())
    }

    pub fn drop_scope(&mut self, scope: &Rc<Scope>) -> R<()> {
        let own_closures = scope
            .vars
            .borrow()
            .iter()
            .filter(|(_, c)| match &*c.borrow() {
                Value::Func(f) => match &**f {
                    FuncVal::Closure(cd) => Rc::strong_count(f) == 1 && Rc::ptr_eq(&cd.env, scope),
                    _ => false,
                },
                _ => false,
            })
            .count();
        if Rc::strong_count(scope) > 1 + own_closures {
            // capturado por una clausura que escapa: las variables siguen vivas
            return Ok(());
        }
        let vars = std::mem::take(&mut *scope.vars.borrow_mut());
        for (_, c) in vars.into_iter().rev() {
            if Rc::strong_count(&c) == 1 {
                let v = std::mem::replace(&mut *c.borrow_mut(), Value::Nothing);
                self.drop_value(v)?;
            }
        }
        Ok(())
    }

    pub fn drop_value(&mut self, v: Value) -> R<()> {
        match v {
            Value::Struct(rc) => {
                if Rc::strong_count(&rc) != 1 {
                    return Ok(());
                }
                if self.drop_types.contains(&rc.info.key) {
                    if let Some(defs) = self.lookup_def_set(&rc.info.key, "drop") {
                        let place = Place::temp(Value::Struct(rc));
                        self.call_def(&defs[0].clone(), vec![], Some(place.clone()), Span::default())?;
                        let inner = std::mem::replace(&mut *place.cell.borrow_mut(), Value::Nothing);
                        if let Value::Struct(rc2) = inner {
                            if let Ok(sv) = Rc::try_unwrap(rc2) {
                                for f in sv.vals { self.drop_value(f)?; }
                            }
                        }
                        return Ok(());
                    }
                    return Ok(());
                }
                if let Ok(sv) = Rc::try_unwrap(rc) {
                    for f in sv.vals { self.drop_value(f)?; }
                }
            }
            Value::Variant(rc) => {
                if let Ok(vv) = Rc::try_unwrap(rc) {
                    for f in vv.vals { self.drop_value(f)?; }
                }
            }
            Value::List(rc) | Value::Array(rc) | Value::Tuple(rc) => {
                if let Ok(vs) = Rc::try_unwrap(rc) {
                    for f in vs { self.drop_value(f)?; }
                }
            }
            Value::Map(rc) | Value::Set(rc) => {
                if let Ok(m) = Rc::try_unwrap(rc) {
                    for (k, val) in m.entries {
                        self.drop_value(val)?;
                        self.drop_value(k)?;
                    }
                }
            }
            Value::Box(c) | Value::Rc(c) => {
                if Rc::strong_count(&c) == 1 {
                    let inner = std::mem::replace(&mut *c.borrow_mut(), Value::Nothing);
                    self.drop_value(inner)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Un pánico o `exit` termina el programa sin ejecutar destructores.
pub(crate) fn is_abort<T>(r: &R<T>) -> bool {
    matches!(r, Err(Ctl::Panic(_)) | Err(Ctl::Exit(_)))
}

fn implicit_generics(t: &TypeExpr) -> Vec<String> {
    let mut out = vec![];
    fn go(t: &TypeExpr, out: &mut Vec<String>) {
        match &t.kind {
            TypeKind::Named { args, .. } => {
                for a in args {
                    if let TypeKind::Named { path, args: a2 } = &a.kind {
                        if path.len() == 1 && a2.is_empty() {
                            out.push(path[0].name.clone());
                        }
                    }
                    go(a, out);
                }
            }
            TypeKind::Ref(i) | TypeKind::Ptr(i) | TypeKind::Optional(i) => go(i, out),
            _ => {}
        }
    }
    go(t, &mut out);
    out
}

mod eval;
