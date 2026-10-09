//! Formateador oficial (`ncom fmt`): un único estilo canónico y determinista.
//!
//! Trabaja sobre el AST y reinyecta los comentarios por posición. Los literales (números, chars,
//! strings) se copian tal cual del fuente.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lexer::{self, Tok};
use crate::parser::parse_module;

const WIDTH: usize = 100;

struct Comment {
    lo: usize,
    hi: usize,
    text: String,
}

pub struct Fmt<'a> {
    src: &'a str,
    comments: Vec<Comment>,
    ci: usize,
    out: String,
    indent: usize,
    prev_end: usize,
    /// Columnas ya ocupadas en la línea antes de la expresión (p. ej. `x := `).
    prefix: usize,
}

/// Formatea un fichero fuente completo. Devuelve los diagnósticos de sintaxis si no se puede parsear.
pub fn format_source(src: &str) -> Result<String, Vec<Diagnostic>> {
    let (module, diags) = parse_module(0, src);
    if diags.iter().any(|d| d.is_error()) {
        return Err(diags);
    }
    let (toks, _) = lexer::lex(0, src, true);
    let comments = toks
        .into_iter()
        .filter_map(|t| match t.tok {
            Tok::Comment(text) => Some(Comment { lo: t.span.lo as usize, hi: t.span.hi as usize, text }),
            _ => None,
        })
        .collect();
    let mut f = Fmt { src, comments, ci: 0, out: String::new(), indent: 0, prev_end: 0, prefix: 0 };
    f.module(&module);
    let mut out = f.out;
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

pub fn type_str(t: &TypeExpr) -> String {
    match &t.kind {
        TypeKind::Prim(Prim::Nothing) => "nothing".into(),
        TypeKind::Prim(p) => p.name().into(),
        TypeKind::Named { path, args } => {
            let mut s = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
            if !args.is_empty() {
                s.push('<');
                s.push_str(&args.iter().map(type_str).collect::<Vec<_>>().join(", "));
                s.push('>');
            }
            s
        }
        TypeKind::Ref(i) => format!("&{}", type_str(i)),
        TypeKind::Ptr(i) => format!("*{}", type_str(i)),
        TypeKind::Optional(i) => format!("{}?", type_str(i)),
        TypeKind::Union(v) => v.iter().map(type_str).collect::<Vec<_>>().join(" | "),
        TypeKind::Func { params, ret } => format!("({}) -> {}", params.iter().map(type_str).collect::<Vec<_>>().join(", "), type_str(ret)),
        TypeKind::Tuple(v) => format!("({})", v.iter().map(type_str).collect::<Vec<_>>().join(", ")),
        TypeKind::Array { elem, len } => format!("{}[{len}]", type_str(elem)),
    }
}

pub fn generics_str(gs: &[GenericParam]) -> String {
    if gs.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = gs
        .iter()
        .map(|g| {
            if g.bounds.is_empty() {
                g.name.name.clone()
            } else {
                format!("{}: {}", g.name.name, g.bounds.iter().map(type_str).collect::<Vec<_>>().join(" + "))
            }
        })
        .collect();
    format!("<{}>", parts.join(", "))
}

/// `pub unsafe fn nombre<T>(a $int) -> $int` (sin cuerpo).
pub fn fn_signature(f: &FnDecl) -> String {
    let mut s = String::new();
    if f.is_pub { s.push_str("pub "); }
    if f.is_unsafe { s.push_str("unsafe "); }
    if f.is_async { s.push_str("async "); }
    if f.is_extern && f.body.is_some() { s.push_str("extern \"C\" "); }
    s.push_str("fn ");
    s.push_str(&f.name.name);
    s.push_str(&generics_str(&f.generics));
    let mut params: Vec<String> = f.params.iter().map(|p| format!("{} {}", p.name.name, type_str(&p.ty))).collect();
    if f.variadic { params.push("...".into()); }
    s.push_str(&format!("({}) -> {}", params.join(", "), type_str(&f.ret)));
    s
}

fn prec_of(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Assign(..) => 1,
        ExprKind::Ternary(..) => 2,
        ExprKind::Binary(op, ..) => match op {
            BinOp::Or => 3,
            BinOp::And => 4,
            BinOp::BitOr => 5,
            BinOp::BitXor => 6,
            BinOp::BitAnd => 7,
            BinOp::Eq | BinOp::Ne | BinOp::RefEq => 8,
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => 9,
            BinOp::Shl | BinOp::Shr => 10,
            BinOp::Add | BinOp::Sub => 11,
            BinOp::Mul | BinOp::Div | BinOp::Rem => 12,
            BinOp::Pow => 13,
        },
        ExprKind::Cast { .. } => 14,
        ExprKind::Unary(..) | ExprKind::Await(_) => 15,
        ExprKind::IncDec { prefix: true, .. } => 15,
        ExprKind::Return(_) | ExprKind::Closure { .. } => 1,
        _ => 17,
    }
}

impl<'a> Fmt<'a> {
    fn pad(&self) -> String {
        "    ".repeat(self.indent)
    }

    fn line(&mut self, text: &str) {
        self.out.push_str(&self.pad());
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn has_blank_between(&self, a: usize, b: usize) -> bool {
        if a >= b || b > self.src.len() {
            return false;
        }
        self.src[a..b].matches('\n').count() >= 2
    }

    /// Emite los comentarios independientes anteriores a `pos`. Si el último queda separado del
    /// siguiente elemento por una línea en blanco, la conserva (comentario «suelto»).
    fn flush_before(&mut self, pos: usize) {
        let mut emitted = false;
        while self.ci < self.comments.len() && self.comments[self.ci].lo < pos {
            let (lo, hi, text) = {
                let c = &self.comments[self.ci];
                (c.lo, c.hi, c.text.clone())
            };
            self.ci += 1;
            if !self.out.is_empty() && self.has_blank_between(self.prev_end, lo) && !self.out.ends_with("{\n") && !self.out.ends_with("\n\n") {
                self.out.push('\n');
            }
            for l in text.lines() {
                let l = l.trim_end();
                self.out.push_str(&self.pad());
                self.out.push_str(l.trim_start());
                self.out.push('\n');
            }
            self.prev_end = hi;
            emitted = true;
        }
        if emitted && pos != usize::MAX && self.has_blank_between(self.prev_end, pos) && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    /// Posición del primer comentario pendiente anterior a `pos` (o `pos`).
    fn first_pos(&self, pos: usize) -> usize {
        match self.comments.get(self.ci) {
            Some(c) if c.lo < pos => c.lo,
            _ => pos,
        }
    }

    /// Comentario en la misma línea que el final de una sentencia.
    fn trailing_comment(&mut self, end: usize) -> Option<String> {
        if self.ci < self.comments.len() {
            let c = &self.comments[self.ci];
            if c.lo >= end && !self.src[end..c.lo].contains('\n') && !c.text.contains('\n') {
                let t = c.text.clone();
                self.prev_end = c.hi;
                self.ci += 1;
                return Some(t);
            }
        }
        None
    }

    fn line_with_trailing(&mut self, text: &str, end: usize) {
        self.out.push_str(&self.pad());
        self.out.push_str(text);
        if let Some(c) = self.trailing_comment(end) {
            self.out.push_str("  ");
            self.out.push_str(&c);
        }
        self.out.push('\n');
        // comentarios incrustados dentro de la sentencia (no recogidos): se colocan a continuación
        self.flush_before(end);
        self.prev_end = self.prev_end.max(end);
    }

    fn blank_if_needed(&mut self, lo: usize) {
        if !self.out.is_empty() && !self.out.ends_with("{\n") && !self.out.ends_with("\n\n") && self.has_blank_between(self.prev_end, lo) {
            self.out.push('\n');
        }
    }

    // ---------- módulo e items ----------

    fn module(&mut self, m: &Module) {
        let mut prev_kind: Option<u8> = None;
        for it in &m.items {
            let lo = self.item_start(it);
            let first = self.first_pos(lo);
            let kind = match &it.kind {
                ItemKind::Import(_) => 1,
                ItemKind::Const(_) | ItemKind::Static(_) => 2,
                ItemKind::Alias(_) => 3,
                ItemKind::Stmt(_) => 4,
                ItemKind::Fn(f) if f.body.is_none() => 5,
                _ => 0,
            };
            if let Some(pk) = prev_kind {
                let light = pk == kind && kind != 0;
                if !self.out.ends_with("\n\n") {
                    if light {
                        if self.has_blank_between(self.prev_end, first) { self.out.push('\n'); }
                    } else {
                        self.out.push('\n');
                    }
                }
            }
            self.flush_before(lo);
            prev_kind = Some(kind);
            self.item(it);
        }
        self.flush_before(usize::MAX);
    }

    fn item_start(&self, it: &Item) -> usize {
        it.span.lo as usize
    }

    fn attrs(&mut self, attrs: &[Attribute]) {
        for a in attrs {
            if a.args.is_empty() {
                self.line(&format!("@{}", a.name.name));
            } else {
                self.line(&format!("@{}({})", a.name.name, a.args.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")));
            }
        }
    }

    fn item(&mut self, it: &Item) {
        let end = it.span.hi as usize;
        match &it.kind {
            ItemKind::Fn(f) => self.fn_decl(f, end),
            ItemKind::Struct(s) => {
                self.attrs(&s.attrs);
                let head = format!("{}struct {}{}", if s.is_pub { "pub " } else { "" }, s.name.name, generics_str(&s.generics));
                if s.fields.is_empty() {
                    self.line_with_trailing(&format!("{head} {{}}"), end);
                } else {
                    self.line(&format!("{head} {{"));
                    self.indent += 1;
                    for f in &s.fields {
                        self.line(&format!("{}{} {};", if f.is_pub { "pub " } else { "" }, f.name.name, type_str(&f.ty)));
                    }
                    self.indent -= 1;
                    self.line_with_trailing("}", end);
                }
            }
            ItemKind::Enum(e) => {
                self.attrs(&e.attrs);
                self.line(&format!("{}enum {}{} {{", if e.is_pub { "pub " } else { "" }, e.name.name, generics_str(&e.generics)));
                self.indent += 1;
                let n = e.variants.len();
                for (i, v) in e.variants.iter().enumerate() {
                    let mut s = v.name.name.clone();
                    if !v.fields.is_empty() {
                        s.push_str(&format!("({})", v.fields.iter().map(type_str).collect::<Vec<_>>().join(", ")));
                    }
                    if i + 1 < n { s.push(','); }
                    self.line(&s);
                }
                self.indent -= 1;
                self.line_with_trailing("}", end);
            }
            ItemKind::Trait(t) => {
                self.line(&format!("{}trait {}{} {{", if t.is_pub { "pub " } else { "" }, t.name.name, generics_str(&t.generics)));
                self.indent += 1;
                self.methods(&t.methods);
                self.indent -= 1;
                self.flush_before(end.saturating_sub(1));
                self.line_with_trailing("}", end);
            }
            ItemKind::Impl(i) => {
                let target = type_str(&i.target);
                let head = match &i.trait_ {
                    Some(t) => format!("impl{} {} for {}", generics_str(&i.generics), type_str(t), target),
                    None => format!("impl{} {}", generics_str(&i.generics), target),
                };
                self.line(&format!("{head} {{"));
                self.indent += 1;
                self.methods(&i.methods);
                self.indent -= 1;
                self.flush_before(end.saturating_sub(1));
                self.line_with_trailing("}", end);
            }
            ItemKind::Import(paths) => {
                let parts: Vec<String> = paths
                    .iter()
                    .map(|p| {
                        let mut s = p.path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
                        if p.wildcard { s.push_str(".*"); }
                        if let Some(a) = &p.alias { s.push_str(&format!(" as {}", a.name)); }
                        s
                    })
                    .collect();
                self.line_with_trailing(&format!("import {};", parts.join(", ")), end);
            }
            ItemKind::Const(c) | ItemKind::Static(c) => {
                let kw = if matches!(it.kind, ItemKind::Const(_)) { "const" } else { "static" };
                let v = self.expr(&c.value, 0);
                self.line_with_trailing(&format!("{}{kw} {} {} = {v};", if c.is_pub { "pub " } else { "" }, c.name.name, type_str(&c.ty)), end);
            }
            ItemKind::Alias(a) => {
                self.line_with_trailing(&format!("{}type {}{} = {};", if a.is_pub { "pub " } else { "" }, a.name.name, generics_str(&a.generics), type_str(&a.ty)), end);
            }
            ItemKind::Test { name, body } => {
                let b = self.block_str(body);
                self.line_with_trailing(&format!("test {} {b}", quote(name)), end);
            }
            ItemKind::Bench { name, body } => {
                let b = self.block_str(body);
                self.line_with_trailing(&format!("bench {} {b}", quote(name)), end);
            }
            ItemKind::Extern { abi, fns } => {
                self.line(&format!("extern \"{abi}\" {{"));
                self.indent += 1;
                for f in fns {
                    self.line(&format!("{};", fn_signature(f)));
                }
                self.indent -= 1;
                self.line_with_trailing("}", end);
            }
            ItemKind::Macro { name } => {
                let raw = &self.src[it.span.lo as usize..it.span.hi as usize];
                let _ = name;
                for l in raw.lines() {
                    self.line(l.trim_end());
                }
            }
            ItemKind::Stmt(s) => self.stmt(s),
        }
        self.prev_end = self.prev_end.max(end);
    }

    fn methods(&mut self, ms: &[FnDecl]) {
        for (i, m) in ms.iter().enumerate() {
            let lo = m.span.lo as usize;
            self.flush_before(lo);
            if i > 0 && !self.out.ends_with("\n\n") {
                self.out.push('\n');
            }
            self.fn_decl(m, m.span.hi as usize);
        }
    }

    fn fn_decl(&mut self, f: &FnDecl, end: usize) {
        self.attrs(&f.attrs);
        let sig = fn_signature(f);
        match &f.body {
            None => self.line_with_trailing(&format!("{sig};"), end),
            Some(b) => {
                let body = self.block_str(b);
                self.line_with_trailing(&format!("{sig} {body}"), end);
            }
        }
    }

    // ---------- bloques y sentencias ----------

    fn block_str(&mut self, b: &Block) -> String {
        if b.stmts.is_empty() && b.tail.is_none() && !self.comment_inside(b.span.lo as usize, b.span.hi as usize) {
            return "{}".into();
        }
        let saved = std::mem::take(&mut self.out);
        self.indent += 1;
        self.block_body(b);
        self.indent -= 1;
        let body = std::mem::replace(&mut self.out, saved);
        format!("{{\n{body}{}}}", self.pad())
    }

    fn comment_inside(&self, lo: usize, hi: usize) -> bool {
        self.comments[self.ci..].iter().any(|c| c.lo > lo && c.lo < hi)
    }

    fn block_body(&mut self, b: &Block) {
        for s in &b.stmts {
            self.flush_before(s.span.lo as usize);
            self.blank_if_needed(s.span.lo as usize);
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.flush_before(t.span.lo as usize);
            self.blank_if_needed(t.span.lo as usize);
            let text = self.expr(t, 0);
            self.line_with_trailing(&text, t.span.hi as usize);
        }
        self.flush_before(b.span.hi as usize);
    }

    fn stmt(&mut self, s: &Stmt) {
        let end = s.span.hi as usize;
        match &s.kind {
            StmtKind::Let { name, ty, init, walrus } => {
                self.prefix = name.name.len() + 5 + ty.as_ref().map(|t| type_str(t).len() + 1).unwrap_or(0);
                let text = match (ty, init) {
                    (Some(t), Some(e)) => format!("let {} {} = {};", name.name, type_str(t), self.expr(e, 0)),
                    (Some(t), None) => format!("let {} {};", name.name, type_str(t)),
                    (None, Some(e)) if *walrus => format!("{} := {};", name.name, self.expr(e, 0)),
                    (None, Some(e)) => format!("let {} = {};", name.name, self.expr(e, 0)),
                    (None, None) => format!("let {};", name.name),
                };
                self.prefix = 0;
                self.line_with_trailing(&text, end);
            }
            StmtKind::Destructure { targets, init } => {
                let typed = targets.iter().any(|t| t.ty.is_some());
                let names: Vec<String> = targets
                    .iter()
                    .map(|t| match &t.ty {
                        Some(ty) => format!("{} {}", t.name.name, type_str(ty)),
                        None => t.name.name.clone(),
                    })
                    .collect();
                let text = format!("({}) {} {};", names.join(", "), if typed { "=" } else { ":=" }, self.expr(init, 0));
                self.line_with_trailing(&text, end);
            }
            StmtKind::Expr(e) => {
                let mut text = self.expr(e, 0);
                if !is_block_like(e) {
                    text.push(';');
                }
                self.line_with_trailing(&text, end);
            }
        }
    }

    // ---------- expresiones ----------

    fn literal_src(&self, e: &Expr) -> String {
        let mut s = self.src[e.span.lo as usize..e.span.hi as usize].trim();
        while s.starts_with('(') && s.ends_with(')') && !matches!(e.kind, ExprKind::Str(_) | ExprKind::Char(_)) {
            s = s[1..s.len() - 1].trim();
        }
        s.to_string()
    }

    fn wrap(&self, s: String, e: &Expr, min: u8) -> String {
        if prec_of(e) < min { format!("({s})") } else { s }
    }

    fn args(&mut self, items: &[Expr], open: &str, close: &str, lead: usize) -> String {
        let prefix = std::mem::take(&mut self.prefix);
        let parts: Vec<String> = items.iter().map(|a| self.expr(a, 1)).collect();
        self.prefix = prefix;
        let flat = format!("{open}{}{close}", parts.join(", "));
        if !flat.contains('\n') && self.indent * 4 + prefix + lead + flat.len() <= WIDTH {
            return flat;
        }
        if flat.contains('\n') && parts.len() == 1 {
            return flat;
        }
        let inner = "    ".repeat(self.indent + 1);
        let mut s = format!("{open}\n");
        for p in &parts {
            s.push_str(&inner);
            s.push_str(p);
            s.push_str(",\n");
        }
        s.push_str(&self.pad());
        s.push_str(close);
        s
    }

    fn expr(&mut self, e: &Expr, min: u8) -> String {
        use ExprKind::*;
        let s = match &e.kind {
            Int(_) | Float(_) | Char(_) | Str(_) => self.literal_src(e),
            Bool(b) => b.to_string(),
            Null => "null".into(),
            Ident(n) => n.clone(),
            Asm(s) => format!("asm!({})", quote(s)),
            Break => "break".into(),
            Return(v) => match v {
                Some(x) => format!("return {}", self.expr(x, 1)),
                None => "return".into(),
            },
            Unary(op, x) => {
                let sym = match op { UnOp::Neg => "-", UnOp::Not => "!", UnOp::BitNot => "~", UnOp::Ref => "&", UnOp::Deref => "*" };
                let inner = self.expr(x, 15);
                format!("{sym}{inner}")
            }
            Await(x) => format!("await {}", self.expr(x, 15)),
            Binary(op @ (BinOp::And | BinOp::Or), _, _) => {
                let op = *op;
                let p = prec_of(e);
                let mut operands: Vec<&Expr> = vec![];
                let mut cur = e;
                while let Binary(o2, l2, r2) = &cur.kind {
                    if *o2 != op { break; }
                    operands.push(r2);
                    cur = l2;
                }
                operands.push(cur);
                operands.reverse();
                let prefix = std::mem::take(&mut self.prefix);
                let parts: Vec<String> = operands.iter().map(|x| self.expr(x, p + 1)).collect();
                self.prefix = prefix;
                let flat = parts.join(&format!(" {} ", op.symbol()));
                if !flat.contains('\n') && self.indent * 4 + prefix + flat.len() <= WIDTH {
                    flat
                } else {
                    let pad = "    ".repeat(self.indent + 1);
                    parts.join(&format!("\n{pad}{} ", op.symbol()))
                }
            }
            Binary(op, l, r) => {
                let p = prec_of(e);
                let (lp, rp) = if *op == BinOp::Pow { (p + 1, p) } else { (p, p + 1) };
                let ls = self.expr(l, lp);
                let rs = self.expr(r, rp);
                format!("{ls} {} {rs}", op.symbol())
            }
            Assign(op, l, r) => {
                let sym = match op { None => "=".to_string(), Some(o) => format!("{}=", o.symbol()) };
                let ls = self.expr(l, 2);
                let rs = self.expr(r, 1);
                format!("{ls} {sym} {rs}")
            }
            IncDec { target, inc, prefix } => {
                let sym = if *inc { "++" } else { "--" };
                if *prefix { format!("{sym}{}", self.expr(target, 15)) } else { format!("{}{sym}", self.expr(target, 17)) }
            }
            Ternary(c, a, b) => {
                let (cs, a_s, bs) = (self.expr(c, 3), self.expr(a, 1), self.expr(b, 2));
                format!("{cs} ? {a_s} : {bs}")
            }
            Call { callee, args } => {
                let c = self.expr(callee, 17);
                let a = self.args(args, "(", ")", c.len());
                format!("{c}{a}")
            }
            Field(b, n) => format!("{}.{}", self.expr(b, 17), n.name),
            Index(b, i) => format!("{}[{}]", self.expr(b, 17), self.expr(i, 0)),
            Slice { base, lo, hi } => {
                let b = self.expr(base, 17);
                let l = lo.as_ref().map(|x| self.expr(x, 0)).unwrap_or_default();
                let h = hi.as_ref().map(|x| self.expr(x, 0)).unwrap_or_default();
                format!("{b}[{l}..{h}]")
            }
            Array(items) => self.args(items, "[", "]", 0),
            Tuple(items) => self.args(items, "(", ")", 0),
            Set(items) => {
                if items.is_empty() { "set {}".into() } else {
                    let a = self.args(items, "{ ", " }", 4);
                    format!("set {a}")
                }
            }
            Map(entries) => {
                if entries.is_empty() { "map {}".into() } else {
                    let parts: Vec<String> = entries.iter().map(|(k, v)| format!("{}: {}", self.expr(k, 1), self.expr(v, 1))).collect();
                    let flat = format!("map {{ {} }}", parts.join(", "));
                    if !flat.contains('\n') && self.indent * 4 + flat.len() <= WIDTH { flat } else {
                        let inner = "    ".repeat(self.indent + 1);
                        let mut s = String::from("map {\n");
                        for p in &parts { s.push_str(&format!("{inner}{p},\n")); }
                        s.push_str(&format!("{}}}", self.pad()));
                        s
                    }
                }
            }
            StructLit { path, fields, .. } => {
                let name = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
                if fields.is_empty() { format!("{name} {{}}") } else {
                    let parts: Vec<String> = fields.iter().map(|(n, v)| format!("{}: {}", n.name, self.expr(v, 1))).collect();
                    let flat = format!("{name} {{ {} }}", parts.join(", "));
                    if !flat.contains('\n') && self.indent * 4 + flat.len() <= WIDTH { flat } else {
                        let inner = "    ".repeat(self.indent + 1);
                        let mut s = format!("{name} {{\n");
                        for p in &parts { s.push_str(&format!("{inner}{p},\n")); }
                        s.push_str(&format!("{}}}", self.pad()));
                        s
                    }
                }
            }
            If { cond, then, els } => {
                let c = self.expr(cond, 0);
                let t = self.block_str(then);
                let mut s = format!("if {c} {t}");
                if let Some(el) = els {
                    s.push_str(" else ");
                    match &el.kind {
                        ExprKind::Block(b) => s.push_str(&self.block_str(b)),
                        _ => s.push_str(&self.expr(el, 0)),
                    }
                }
                s
            }
            Match { scrut, arms } => {
                let sc = self.expr(scrut, 0);
                let saved = std::mem::take(&mut self.out);
                self.indent += 1;
                for a in arms {
                    self.flush_before(a.span.lo as usize);
                    self.blank_if_needed(a.span.lo as usize);
                    let pat = self.pattern(&a.pattern);
                    let guard = a.guard.as_ref().map(|g| format!(" if {}", self.expr(g, 0))).unwrap_or_default();
                    let body = self.expr(&a.body, 1);
                    let text = if is_block_like(&a.body) && matches!(a.body.kind, ExprKind::Block(_) | ExprKind::Unsafe(_)) {
                        format!("{pat}{guard} => {body}")
                    } else {
                        format!("{pat}{guard} => {body};")
                    };
                    self.line_with_trailing(&text, a.span.hi as usize);
                }
                self.flush_before(e.span.hi as usize);
                self.indent -= 1;
                let body = std::mem::replace(&mut self.out, saved);
                format!("match {sc} {{\n{body}{}}}", self.pad())
            }
            Closure { params, ret, body } => {
                let short = ret.is_none() || params.iter().any(|p| p.ty.is_none());
                if short {
                    let ps: Vec<String> = params.iter().map(|p| match &p.ty {
                        Some(t) => format!("{} {}", p.name.name, type_str(t)),
                        None => p.name.name.clone(),
                    }).collect();
                    let head = if ps.is_empty() { "||".to_string() } else { format!("|{}|", ps.join(", ")) };
                    let r = ret.as_ref().map(|t| format!(" -> {}", type_str(t))).unwrap_or_default();
                    let b = match &body.kind {
                        ExprKind::Block(b) => self.block_str(b),
                        _ => self.expr(body, 1),
                    };
                    format!("{head}{r} {b}")
                } else {
                    let ps: Vec<String> = params.iter().map(|p| format!("{} {}", p.name.name, type_str(p.ty.as_ref().unwrap()))).collect();
                    let b = match &body.kind {
                        ExprKind::Block(b) => self.block_str(b),
                        _ => self.expr(body, 0),
                    };
                    format!("({}) -> {} {b}", ps.join(", "), type_str(ret.as_ref().unwrap()))
                }
            }
            Block(b) => self.block_str(b),
            Unsafe(b) => format!("unsafe {}", self.block_str(b)),
            Cast { expr, ty, lossy } => format!("{} {} {}", self.expr(expr, 14), if *lossy { "as!" } else { "as" }, type_str(ty)),
            Try(x) => format!("{}?", self.expr(x, 17)),
            While { cond, body } => {
                let c = self.expr(cond, 0);
                let b = self.block_str(body);
                format!("while {c} {b}")
            }
            Loop { kind, body } => {
                let head = match kind {
                    LoopKind::C { init, cond, update } => {
                        let i = match init.as_deref() {
                            Some(Stmt { kind: StmtKind::Let { name, ty, init, walrus }, .. }) => {
                                let v = init.as_ref().map(|e| self.expr(e, 0)).unwrap_or_default();
                                match ty {
                                    Some(t) => format!("{} {} = {v}", name.name, type_str(t)),
                                    None if *walrus => format!("{} := {v}", name.name),
                                    None => format!("let {} = {v}", name.name),
                                }
                            }
                            Some(Stmt { kind: StmtKind::Expr(e), .. }) => self.expr(e, 0),
                            _ => String::new(),
                        };
                        let c = cond.as_ref().map(|e| self.expr(e, 0)).unwrap_or_default();
                        let u = update.as_ref().map(|e| self.expr(e, 0)).unwrap_or_default();
                        if i.is_empty() && c.is_empty() && u.is_empty() { ";;".to_string() } else { format!("{i}; {c}; {u}").trim_end().to_string() }
                    }
                    LoopKind::Each { var, ty, iter } => {
                        let it = self.expr(iter, 0);
                        match ty {
                            Some(t) => format!("{} {} : {it}", var.name, type_str(t)),
                            None => format!("{} : {it}", var.name),
                        }
                    }
                };
                let b = self.block_str(body);
                format!("loop ({head}) {b}")
            }
            SomeWrap(x) => self.expr(x, min),
            Global { name, .. } => name.clone(),
            TypeStatic { key, name } => format!("{key}.{name}"),
            VariantRef { enum_key, variant } => format!("{enum_key}.{variant}"),
        };
        self.wrap(s, e, min)
    }

    fn pattern(&mut self, p: &Pattern) -> String {
        match p {
            Pattern::Wild(_) => "_".into(),
            Pattern::Bind { name, ty } => match ty {
                Some(t) => format!("{} {}", name.name, type_str(t)),
                None => name.name.clone(),
            },
            Pattern::Lit(e) => self.expr(e, 15),
            Pattern::Tuple(items, _) => format!("({})", items.iter().map(|i| self.pattern(i)).collect::<Vec<_>>().join(", ")),
            Pattern::Variant { path, args, .. } => {
                let name = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
                match args {
                    Some(a) => format!("{name}({})", a.iter().map(|i| self.pattern(i)).collect::<Vec<_>>().join(", ")),
                    None => name,
                }
            }
        }
    }
}

fn is_block_like(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::While { .. } | ExprKind::Loop { .. } | ExprKind::Block(_) | ExprKind::Unsafe(_)
    )
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(s: &str) -> String {
        format_source(s).unwrap_or_else(|d| panic!("{:?}", d.iter().map(|d| d.message.clone()).collect::<Vec<_>>()))
    }

    #[test]
    fn canonical_and_idempotent() {
        let src = "fn  add( a $int,b $int )->$int{return a+b;}\nfn main()->nothing{x:=add(1,2); if x>1{println(x);}else{println(0);} }";
        let once = fmt(src);
        assert_eq!(fmt(&once), once);
        assert!(once.contains("fn add(a $int, b $int) -> $int {\n    return a + b;\n}"));
        assert!(once.contains("x := add(1, 2);"));
    }

    #[test]
    fn keeps_comments_and_literals() {
        let src = "# cabecera\nfn main() -> nothing {\n    x := 0xFF; # hex\n    # antes\n    println(\"a{x}\");\n}\n";
        let out = fmt(src);
        assert!(out.contains("# cabecera"));
        assert!(out.contains("x := 0xFF;  # hex"));
        assert!(out.contains("# antes"));
        assert_eq!(fmt(&out), out);
    }

    #[test]
    fn preserves_let_style_and_comment_spacing() {
        let src = "# cabecera\n\n/// doc\nfn a() -> nothing {\n    let x = 1;\n    y := 2;\n}\nfn b() -> nothing {}\n";
        let out = fmt(src);
        assert!(out.starts_with("# cabecera\n\n/// doc\nfn a()"), "{out}");
        assert!(out.contains("let x = 1;") && out.contains("y := 2;"), "{out}");
        assert!(out.contains("}\n\nfn b() -> nothing {}"), "{out}");
        assert_eq!(fmt(&out), out);
    }

    #[test]
    fn breaks_long_lines() {
        let src = "fn f() -> nothing { r := some_function_with_a_long_name(argument_number_one, argument_number_two, argument_number_three, four_); }";
        let out = fmt(src);
        assert!(out.contains("some_function_with_a_long_name(\n        argument_number_one,\n"), "{out}");
        assert_eq!(fmt(&out), out);
        let cond = "fn f(a $bool) -> $bool { return aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa && bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb && cccccccccccccccccccccccccccccccccccc && ddddddddddddd; }";
        let out = fmt(cond);
        assert!(out.contains("\n        && bbbb"), "{out}");
        assert_eq!(fmt(&out), out);
    }

    #[test]
    fn precedence_parens() {
        let out = fmt("fn f() -> $int { return (1 + 2) * 3 - -(4 ^ 2); }");
        assert!(out.contains("(1 + 2) * 3 - -(4 ^ 2)"), "{out}");
    }
}
