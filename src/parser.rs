//! Parser recursivo descendente de N# (gramática formal §58 y reglas de desambiguación).

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lexer::{self, StrPart, Tok, Token};

type PResult<T> = Result<T, Diagnostic>;

pub struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    file: u32,
    src: &'a str,
    pub diags: Vec<Diagnostic>,
    no_struct: bool,
    no_union: bool,
}

/// Parsea un módulo completo. Siempre devuelve un AST (posiblemente parcial) y los diagnósticos.
pub fn parse_module(file: u32, src: &str) -> (Module, Vec<Diagnostic>) {
    let (toks, mut diags) = lexer::lex(file, src, false);
    let mut p = Parser { toks, pos: 0, file, src, diags: vec![], no_struct: false, no_union: false };
    let m = p.module();
    diags.extend(p.diags);
    (m, diags)
}

/// Parsea una única expresión (REPL, interpolaciones).
pub fn parse_expr_source(file: u32, src: &str) -> (Option<Expr>, Vec<Diagnostic>) {
    let (toks, mut diags) = lexer::lex(file, src, false);
    let mut p = Parser { toks, pos: 0, file, src, diags: vec![], no_struct: false, no_union: false };
    let r = p.expr();
    let r = match r {
        Ok(e) if matches!(p.tok(), Tok::Eof) => Some(e),
        Ok(_) => {
            let d = p.unexpected("fin de la expresión");
            p.diags.push(d);
            None
        }
        Err(d) => {
            p.diags.push(d);
            None
        }
    };
    diags.extend(p.diags);
    (r, diags)
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("el identificador `{s}`"),
        Tok::Kw(k) => format!("la palabra reservada `{k}`"),
        Tok::Prim(p) => format!("el tipo `${p}`"),
        Tok::MacroVar(n) => format!("`${n}`"),
        Tok::Int(_) | Tok::Float(_) => "un número".into(),
        Tok::Char(_) => "un carácter".into(),
        Tok::Str(_) => "un string".into(),
        Tok::Sym(s) => format!("`{s}`"),
        Tok::Comment(_) => "un comentario".into(),
        Tok::Eof => "el fin del fichero".into(),
    }
}

impl<'a> Parser<'a> {
    // ---------- utilidades ----------
    fn tok(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn nth(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }
    fn advance(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn at_sym(&self, s: &str) -> bool {
        matches!(self.tok(), Tok::Sym(x) if *x == s)
    }
    fn at_kw(&self, k: &str) -> bool {
        matches!(self.tok(), Tok::Kw(x) if *x == k)
    }
    fn at_ident(&self, s: &str) -> bool {
        matches!(self.tok(), Tok::Ident(x) if x == s)
    }
    fn nth_sym(&self, n: usize, s: &str) -> bool {
        matches!(self.nth(n), Tok::Sym(x) if *x == s)
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if self.at_sym(s) { self.advance(); true } else { false }
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.at_kw(k) { self.advance(); true } else { false }
    }
    fn unexpected(&self, expected: &str) -> Diagnostic {
        if let Tok::MacroVar(n) = self.tok() {
            return Diagnostic::error(
                self.span(),
                format!("tipo primitivo desconocido `${n}`; los primitivos son $int, $float, $bool, $char, $string y $nothing"),
            )
            .with_label("`$` solo se usa en tipos primitivos (y en parámetros de `macro`)");
        }
        Diagnostic::error(self.span(), format!("se esperaba {expected}, pero se encontró {}", describe(self.tok())))
            .with_label(format!("se esperaba {expected}"))
    }
    fn expect_sym(&mut self, s: &str) -> PResult<Span> {
        if self.at_sym(s) {
            Ok(self.advance().span)
        } else if s == ";" && self.pos > 0 {
            Err(Diagnostic::error(self.prev_span(), "falta `;` al final de la sentencia")
                .with_label("se esperaba `;` justo después de esto"))
        } else {
            Err(self.unexpected(&format!("`{s}`")))
        }
    }
    fn expect_kw(&mut self, k: &str) -> PResult<Span> {
        if self.at_kw(k) {
            Ok(self.advance().span)
        } else {
            Err(self.unexpected(&format!("`{k}`")))
        }
    }
    fn ident(&mut self) -> PResult<Ident> {
        match self.tok().clone() {
            Tok::Ident(name) => {
                let span = self.advance().span;
                if name.chars().all(|c| c == '_') {
                    return Err(Diagnostic::error(span, format!("`{name}` es un wildcard reservado y no puede usarse como identificador")));
                }
                Ok(Ident { name, span })
            }
            _ => Err(self.unexpected("un identificador")),
        }
    }
    /// Cierra un `>` de genéricos partiendo `>>` o `>=` si hace falta.
    fn expect_gt(&mut self) -> PResult<()> {
        match self.tok().clone() {
            Tok::Sym(">") => { self.advance(); Ok(()) }
            Tok::Sym(">>") => { self.split_sym(">"); Ok(()) }
            Tok::Sym(">=") => { self.split_sym("="); Ok(()) }
            _ => Err(self.unexpected("`>`")),
        }
    }
    fn split_sym(&mut self, rest: &'static str) {
        let t = &mut self.toks[self.pos];
        t.tok = Tok::Sym(rest);
        t.span.lo += 1;
    }

    fn sync_stmt(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.tok() {
                Tok::Eof => return,
                Tok::Sym("(") | Tok::Sym("[") | Tok::Sym("{") => depth += 1,
                Tok::Sym(")") | Tok::Sym("]") => depth -= 1,
                Tok::Sym("}") => {
                    if depth <= 0 { return; }
                    depth -= 1;
                }
                Tok::Sym(";") if depth <= 0 => { self.advance(); return; }
                _ => {}
            }
            self.advance();
        }
    }

    fn starts_item(&self) -> bool {
        match self.tok() {
            Tok::Kw(k) => matches!(*k, "fn" | "struct" | "enum" | "trait" | "impl" | "import" | "pub" | "const" | "static" | "type" | "async")
                || (*k == "test" && matches!(self.nth(1), Tok::Str(_)))
                || (*k == "unsafe" && matches!(self.nth(1), Tok::Kw("fn"))),
            Tok::Sym("@") => true,
            Tok::Ident(s) => (s == "extern" && matches!(self.nth(1), Tok::Str(_))) || (s == "macro" && matches!(self.nth(1), Tok::Ident(_))) || (s == "bench" && matches!(self.nth(1), Tok::Str(_))),
            _ => false,
        }
    }

    fn sync_item(&mut self) {
        let mut depth = 0i32;
        while !matches!(self.tok(), Tok::Eof) {
            if depth <= 0 && self.starts_item() { return; }
            match self.tok() {
                Tok::Sym("{") | Tok::Sym("(") | Tok::Sym("[") => depth += 1,
                Tok::Sym("}") | Tok::Sym(")") | Tok::Sym("]") => depth -= 1,
                _ => {}
            }
            self.advance();
            if depth == 0 && matches!(self.toks[self.pos.saturating_sub(1)].tok, Tok::Sym("}") | Tok::Sym(";")) { return; }
        }
    }

    // ---------- módulo e items ----------
    fn module(&mut self) -> Module {
        let mut items = Vec::new();
        while !matches!(self.tok(), Tok::Eof) {
            let before = self.pos;
            match self.item() {
                Ok(it) => items.push(it),
                Err(d) => {
                    self.diags.push(d);
                    if self.pos == before { self.advance(); }
                    self.sync_item();
                }
            }
        }
        Module { items }
    }

    fn attributes(&mut self) -> PResult<Vec<Attribute>> {
        let mut attrs = vec![];
        while self.at_sym("@") {
            self.advance();
            let name = self.ident()?;
            let mut args = vec![];
            if self.eat_sym("(") {
                while !self.at_sym(")") {
                    args.push(self.ident()?);
                    if !self.eat_sym(",") { break; }
                }
                self.expect_sym(")")?;
            }
            attrs.push(Attribute { name, args });
        }
        Ok(attrs)
    }

    fn item(&mut self) -> PResult<Item> {
        let start = self.span();
        let doc = self.toks[self.pos].doc.clone();
        let attrs = self.attributes()?;
        let doc = doc.or_else(|| self.toks[self.pos].doc.clone());
        let is_pub = self.eat_kw("pub");
        let kind = match self.tok().clone() {
            Tok::Kw("fn") | Tok::Kw("async") => ItemKind::Fn(self.fn_decl(is_pub, doc, attrs, false)?),
            Tok::Kw("unsafe") if matches!(self.nth(1), Tok::Kw("fn") | Tok::Kw("async")) => {
                ItemKind::Fn(self.fn_decl(is_pub, doc, attrs, false)?)
            }
            Tok::Kw("struct") => ItemKind::Struct(self.struct_decl(is_pub, doc, attrs)?),
            Tok::Kw("enum") => ItemKind::Enum(self.enum_decl(is_pub, doc, attrs)?),
            Tok::Kw("trait") => ItemKind::Trait(self.trait_decl(is_pub, doc)?),
            Tok::Kw("impl") => ItemKind::Impl(self.impl_decl()?),
            Tok::Kw("import") => ItemKind::Import(self.import_decl()?),
            Tok::Kw("const") => ItemKind::Const(self.const_decl(is_pub, doc)?),
            Tok::Kw("static") => ItemKind::Static(self.const_decl(is_pub, doc)?),
            Tok::Kw("type") => ItemKind::Alias(self.alias_decl(is_pub, doc)?),
            Tok::Kw("test") if matches!(self.nth(1), Tok::Str(_)) => {
                self.advance();
                let name = self.plain_string()?;
                ItemKind::Test { name, body: self.block()? }
            }
            Tok::Ident(s) if s == "bench" && matches!(self.nth(1), Tok::Str(_)) => {
                self.advance();
                let name = self.plain_string()?;
                ItemKind::Bench { name, body: self.block()? }
            }
            Tok::Ident(s) if s == "extern" && matches!(self.nth(1), Tok::Str(_)) => self.extern_decl(is_pub, doc, attrs)?,
            Tok::Ident(s) if s == "macro" && matches!(self.nth(1), Tok::Ident(_)) => {
                self.advance();
                let name = self.ident()?;
                // Se consume la firma y el cuerpo; la semántica de macros está reservada en v0.1.
                self.expect_sym("(")?;
                let mut depth = 1;
                while depth > 0 && !matches!(self.tok(), Tok::Eof) {
                    if self.at_sym("(") { depth += 1 } else if self.at_sym(")") { depth -= 1 }
                    self.advance();
                }
                self.block()?;
                ItemKind::Macro { name }
            }
            _ => {
                if is_pub || !attrs.is_empty() {
                    return Err(self.unexpected("una declaración (fn, struct, enum, trait, ...)"));
                }
                ItemKind::Stmt(self.stmt_or_tail(false)?.0)
            }
        };
        Ok(Item { kind, span: start.to(self.prev_span()) })
    }

    fn plain_string(&mut self) -> PResult<String> {
        match self.tok().clone() {
            Tok::Str(parts) => {
                let sp = self.advance().span;
                let mut s = String::new();
                for p in parts {
                    match p {
                        StrPart::Lit(l) => s.push_str(&l),
                        StrPart::Expr(..) => return Err(Diagnostic::error(sp, "aquí se requiere un string sin interpolación")),
                    }
                }
                Ok(s)
            }
            _ => Err(self.unexpected("un string")),
        }
    }

    fn generics(&mut self) -> PResult<Vec<GenericParam>> {
        let mut out = vec![];
        if !self.at_sym("<") { return Ok(out); }
        self.advance();
        loop {
            let name = self.ident()?;
            let mut bounds = vec![];
            if self.eat_sym(":") {
                loop {
                    bounds.push(self.type_atom()?);
                    if !self.eat_sym("+") { break; }
                }
            }
            out.push(GenericParam { name, bounds });
            if !self.eat_sym(",") { break; }
        }
        self.expect_gt()?;
        Ok(out)
    }

    fn fn_decl(&mut self, is_pub: bool, doc: Option<String>, attrs: Vec<Attribute>, in_extern: bool) -> PResult<FnDecl> {
        let start = self.span();
        let mut is_unsafe = false;
        let mut is_async = false;
        let mut is_extern = in_extern;
        loop {
            if self.eat_kw("unsafe") { is_unsafe = true; }
            else if self.eat_kw("async") { is_async = true; }
            else if self.at_ident("extern") && matches!(self.nth(1), Tok::Str(_)) {
                self.advance();
                let abi = self.plain_string()?;
                if abi != "C" {
                    return Err(Diagnostic::error(self.prev_span(), format!("ABI `{abi}` no soportada; solo existe \"C\"")));
                }
                is_extern = true;
            } else { break; }
        }
        self.expect_kw("fn")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_sym("(")?;
        let mut params = vec![];
        let mut variadic = false;
        while !self.at_sym(")") {
            if self.at_sym(".") && self.nth_sym(1, ".") && self.nth_sym(2, ".") || self.at_sym("..") && self.nth_sym(1, ".") {
                if !is_extern {
                    return Err(Diagnostic::error(self.span(), "los parámetros variádicos no existen en N# v0.1 (solo en declaraciones `extern \"C\"`)"));
                }
                while self.eat_sym(".") || self.eat_sym("..") {}
                variadic = true;
                break;
            }
            let pname = self.ident()?;
            let ty = self.type_()?;
            params.push(Param { name: pname, ty });
            if !self.eat_sym(",") { break; }
        }
        self.expect_sym(")")?;
        let ret = if self.eat_sym("->") {
            self.type_()?
        } else if in_extern || is_extern {
            TypeExpr { kind: TypeKind::Prim(Prim::Int), span: self.prev_span() }
        } else {
            return Err(Diagnostic::error(self.span(), "falta el tipo de retorno; en N# es obligatorio (`-> tipo`) y no se infiere")
                .with_label("se esperaba `->`")
                .with_note("si la función no devuelve nada, escribe `-> nothing`"));
        };
        let body = if self.at_sym("{") {
            Some(self.block()?)
        } else {
            self.expect_sym(";")?;
            None
        };
        Ok(FnDecl { name, generics, params, variadic, ret, body, is_pub, is_unsafe, is_async, is_extern, doc, attrs, span: start.to(self.prev_span()) })
    }

    fn extern_decl(&mut self, is_pub: bool, doc: Option<String>, attrs: Vec<Attribute>) -> PResult<ItemKind> {
        // `extern "C" { ... }` o `extern "C" fn ...`
        if matches!(self.nth(2), Tok::Sym("{")) {
            self.advance();
            let abi = self.plain_string()?;
            if abi != "C" {
                return Err(Diagnostic::error(self.prev_span(), format!("ABI `{abi}` no soportada; solo existe \"C\"")));
            }
            self.expect_sym("{")?;
            let mut fns = vec![];
            while !self.at_sym("}") {
                let d = self.toks[self.pos].doc.clone();
                let p = self.eat_kw("pub");
                fns.push(self.fn_decl(p, d, vec![], true)?);
            }
            self.expect_sym("}")?;
            Ok(ItemKind::Extern { abi, fns })
        } else {
            Ok(ItemKind::Fn(self.fn_decl(is_pub, doc, attrs, false)?))
        }
    }

    fn struct_decl(&mut self, is_pub: bool, doc: Option<String>, attrs: Vec<Attribute>) -> PResult<StructDecl> {
        self.expect_kw("struct")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_sym("{")?;
        let mut fields = vec![];
        while !self.at_sym("}") {
            let fp = self.eat_kw("pub");
            let fname = self.ident()?;
            let ty = self.type_()?;
            if !self.eat_sym(";") && !self.eat_sym(",") {
                return Err(self.unexpected("`;` tras el campo"));
            }
            fields.push(FieldDecl { name: fname, ty, is_pub: fp });
        }
        self.expect_sym("}")?;
        Ok(StructDecl { name, generics, fields, is_pub, doc, attrs })
    }

    fn enum_decl(&mut self, is_pub: bool, doc: Option<String>, attrs: Vec<Attribute>) -> PResult<EnumDecl> {
        self.expect_kw("enum")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_sym("{")?;
        let mut variants = vec![];
        while !self.at_sym("}") {
            let vname = self.ident()?;
            let mut fields = vec![];
            if self.eat_sym("(") {
                while !self.at_sym(")") {
                    fields.push(self.type_()?);
                    if !self.eat_sym(",") { break; }
                }
                self.expect_sym(")")?;
            }
            variants.push(VariantDecl { name: vname, fields });
            if !self.eat_sym(",") { break; }
        }
        self.expect_sym("}")?;
        Ok(EnumDecl { name, generics, variants, is_pub, doc, attrs })
    }

    fn trait_decl(&mut self, is_pub: bool, doc: Option<String>) -> PResult<TraitDecl> {
        self.expect_kw("trait")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_sym("{")?;
        let methods = self.methods_until_brace()?;
        Ok(TraitDecl { name, generics, methods, is_pub, doc })
    }

    fn methods_until_brace(&mut self) -> PResult<Vec<FnDecl>> {
        let mut methods = vec![];
        while !self.at_sym("}") {
            if matches!(self.tok(), Tok::Eof) { return Err(self.unexpected("`}`")); }
            let doc = self.toks[self.pos].doc.clone();
            let attrs = self.attributes()?;
            let p = self.eat_kw("pub");
            methods.push(self.fn_decl(p, doc, attrs, false)?);
        }
        self.expect_sym("}")?;
        Ok(methods)
    }

    fn impl_decl(&mut self) -> PResult<ImplDecl> {
        let start = self.span();
        self.expect_kw("impl")?;
        let generics = self.generics()?;
        let first = self.type_atom()?;
        let (trait_, target) = if self.eat_kw_for() { (Some(first), self.type_atom()?) } else { (None, first) };
        self.expect_sym("{")?;
        let methods = self.methods_until_brace()?;
        Ok(ImplDecl { key: String::new(), trait_key: None, generics, trait_, target, methods, span: start.to(self.prev_span()) })
    }

    fn eat_kw_for(&mut self) -> bool {
        if self.at_ident("for") { self.advance(); true } else { false }
    }

    fn import_decl(&mut self) -> PResult<Vec<ImportPath>> {
        self.expect_kw("import")?;
        let mut out = vec![];
        loop {
            let mut path = vec![self.ident()?];
            let mut wildcard = false;
            while self.eat_sym(".") {
                if self.eat_sym("*") { wildcard = true; break; }
                path.push(self.ident()?);
            }
            let alias = if self.eat_kw("as") { Some(self.ident()?) } else { None };
            out.push(ImportPath { path, wildcard, alias });
            if !self.eat_sym(",") { break; }
        }
        self.expect_sym(";")?;
        Ok(out)
    }

    fn const_decl(&mut self, is_pub: bool, doc: Option<String>) -> PResult<ConstDecl> {
        self.advance(); // const | static
        let name = self.ident()?;
        let ty = self.type_()?;
        self.expect_sym("=")?;
        let value = self.expr()?;
        self.expect_sym(";")?;
        Ok(ConstDecl { name, ty, value, is_pub, doc })
    }

    fn alias_decl(&mut self, is_pub: bool, doc: Option<String>) -> PResult<AliasDecl> {
        self.expect_kw("type")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_sym("=")?;
        let ty = self.type_()?;
        self.expect_sym(";")?;
        Ok(AliasDecl { name, generics, ty, is_pub, doc })
    }

    // ---------- tipos ----------
    pub fn type_(&mut self) -> PResult<TypeExpr> {
        let first = self.type_postfix()?;
        if self.at_sym("|") && !self.no_union {
            let start = first.span;
            let mut alts = vec![first];
            while self.eat_sym("|") {
                alts.push(self.type_postfix()?);
            }
            return Ok(TypeExpr { span: start.to(self.prev_span()), kind: TypeKind::Union(alts) });
        }
        Ok(first)
    }

    fn type_postfix(&mut self) -> PResult<TypeExpr> {
        let mut t = self.type_atom()?;
        loop {
            if self.at_sym("?") {
                let sp = self.advance().span;
                t = TypeExpr { span: t.span.to(sp), kind: TypeKind::Optional(Box::new(t)) };
            } else if self.at_sym("[") && matches!(self.nth(1), Tok::Int(_)) && self.nth_sym(2, "]") {
                self.advance();
                let Tok::Int(n) = self.tok().clone() else { unreachable!() };
                self.advance();
                let end = self.expect_sym("]")?;
                if n < 0 {
                    return Err(Diagnostic::error(end, "el tamaño de un array no puede ser negativo"));
                }
                t = TypeExpr { span: t.span.to(end), kind: TypeKind::Array { elem: Box::new(t), len: n as usize } };
            } else {
                break;
            }
        }
        Ok(t)
    }

    /// Tipo sin sufijos `?`, `[N]` ni uniones (útil tras `as` y en cotas).
    fn type_atom(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        match self.tok().clone() {
            Tok::Prim(p) => {
                self.advance();
                let prim = match p {
                    "int" => Prim::Int, "float" => Prim::Float, "bool" => Prim::Bool,
                    "char" => Prim::Char, "string" => Prim::Str, _ => Prim::Nothing,
                };
                Ok(TypeExpr { kind: TypeKind::Prim(prim), span: start })
            }
            Tok::Kw("nothing") => {
                self.advance();
                Ok(TypeExpr { kind: TypeKind::Prim(Prim::Nothing), span: start })
            }
            Tok::Sym("&") => {
                self.advance();
                let inner = self.type_postfix_no_union()?;
                Ok(TypeExpr { span: start.to(inner.span), kind: TypeKind::Ref(Box::new(inner)) })
            }
            Tok::Sym("*") => {
                self.advance();
                let inner = self.type_postfix_no_union()?;
                Ok(TypeExpr { span: start.to(inner.span), kind: TypeKind::Ptr(Box::new(inner)) })
            }
            Tok::Sym("(") => {
                self.advance();
                let mut items = vec![];
                let saved = self.no_union;
                self.no_union = false;
                while !self.at_sym(")") {
                    items.push(self.type_()?);
                    if !self.eat_sym(",") { break; }
                }
                self.no_union = saved;
                let end = self.expect_sym(")")?;
                if self.eat_sym("->") {
                    let ret = self.type_()?;
                    return Ok(TypeExpr { span: start.to(ret.span), kind: TypeKind::Func { params: items, ret: Box::new(ret) } });
                }
                if items.len() == 1 {
                    return Ok(items.pop().unwrap());
                }
                if items.is_empty() {
                    return Err(Diagnostic::error(start.to(end), "tipo vacío `()`; para «sin valor» usa `nothing`"));
                }
                Ok(TypeExpr { span: start.to(end), kind: TypeKind::Tuple(items) })
            }
            Tok::Kw("Self") => {
                let sp = self.advance().span;
                Ok(TypeExpr { kind: TypeKind::Named { path: vec![Ident { name: "Self".into(), span: sp }], args: vec![] }, span: sp })
            }
            Tok::Ident(_) => {
                let mut path = vec![self.ident()?];
                while self.at_sym(".") && matches!(self.nth(1), Tok::Ident(_)) {
                    self.advance();
                    path.push(self.ident()?);
                }
                let mut args = vec![];
                if self.at_sym("<") {
                    self.advance();
                    let saved = self.no_union;
                    self.no_union = false;
                    loop {
                        args.push(self.type_()?);
                        if !self.eat_sym(",") { break; }
                    }
                    self.no_union = saved;
                    self.expect_gt()?;
                }
                Ok(TypeExpr { span: start.to(self.prev_span()), kind: TypeKind::Named { path, args } })
            }
            _ => Err(self.unexpected("un tipo")),
        }
    }

    fn type_postfix_no_union(&mut self) -> PResult<TypeExpr> {
        let saved = self.no_union;
        self.no_union = true;
        let r = self.type_postfix();
        self.no_union = saved;
        r
    }

    // ---------- bloques y sentencias ----------
    fn block(&mut self) -> PResult<Block> {
        let start = self.expect_sym("{")?;
        let saved = (self.no_struct, self.no_union);
        self.no_struct = false;
        self.no_union = false;
        let mut stmts = Vec::new();
        let mut tail = None;
        loop {
            if self.at_sym("}") || matches!(self.tok(), Tok::Eof) { break; }
            let before = self.pos;
            match self.stmt_or_tail(true) {
                Ok((s, is_tail)) => {
                    if is_tail {
                        if let StmtKind::Expr(e) = s.kind { tail = Some(Box::new(e)); }
                        break;
                    }
                    stmts.push(s);
                }
                Err(d) => {
                    self.diags.push(d);
                    if self.pos == before { self.advance(); }
                    self.sync_stmt();
                }
            }
        }
        self.no_struct = saved.0;
        self.no_union = saved.1;
        let end = self.expect_sym("}")?;
        Ok(Block { stmts, tail, span: start.to(end) })
    }

    fn is_block_like_start(&self) -> bool {
        match self.tok() {
            Tok::Kw("if") | Tok::Kw("match") | Tok::Kw("while") | Tok::Kw("loop") | Tok::Sym("{") => true,
            Tok::Kw("unsafe") => self.nth_sym(1, "{"),
            _ => false,
        }
    }

    fn type_start_after_ident(&self) -> bool {
        matches!(self.nth(1), Tok::Prim(_) | Tok::Kw("nothing"))
            || (matches!(self.nth(1), Tok::Ident(_)) && !matches!(self.nth(2), Tok::Sym(".") | Tok::Sym("(")) )
            || (matches!(self.nth(1), Tok::Sym("&") | Tok::Sym("*")) && matches!(self.nth(2), Tok::Prim(_) | Tok::Ident(_)) && self.looks_like_typed_decl())
    }

    /// `x &$int = ...` / `x *$int;`: distingue una declaración tipada de `x & y`.
    fn looks_like_typed_decl(&self) -> bool {
        matches!(self.nth(2), Tok::Prim(_))
    }

    /// Devuelve (sentencia, es_cola). `in_block` permite expresión final sin `;`.
    fn stmt_or_tail(&mut self, in_block: bool) -> PResult<(Stmt, bool)> {
        let start = self.span();
        if self.at_kw("let") {
            self.advance();
            let name = self.ident()?;
            let ty = if !self.at_sym("=") && !self.at_sym(";") { Some(self.type_()?) } else { None };
            let init = if self.eat_sym("=") { Some(self.expr()?) } else { None };
            if ty.is_none() && init.is_none() {
                return Err(Diagnostic::error(name.span, "una declaración `let` sin tipo requiere inicializador"));
            }
            self.expect_sym(";")?;
            return Ok((Stmt { span: start.to(self.prev_span()), kind: StmtKind::Let { name, ty, init } }, false));
        }
        if matches!(self.tok(), Tok::Ident(n) if !n.chars().all(|c| c == '_')) && self.nth_sym(1, ":=") {
            let name = self.ident()?;
            self.advance();
            let init = self.expr()?;
            self.expect_sym(";")?;
            return Ok((Stmt { span: start.to(self.prev_span()), kind: StmtKind::Let { name, ty: None, init: Some(init) } }, false));
        }
        if self.at_sym("(") {
            if let Some(s) = self.try_destructure()? {
                return Ok((s, false));
            }
        }
        if self.is_block_like_start() {
            let e = self.block_like()?;
            if self.eat_sym(";") {
                return Ok((Stmt { span: start.to(self.prev_span()), kind: StmtKind::Expr(e) }, false));
            }
            if in_block && self.at_sym("}") {
                return Ok((Stmt { span: start.to(self.prev_span()), kind: StmtKind::Expr(e) }, true));
            }
            // continuación posfija: `match x { .. }.foo()` o `if .. {..} else {..} + 1`
            if self.at_sym(".") || self.at_sym("?") {
                let e = self.postfix_from(e)?;
                return self.finish_expr_stmt(e, start, in_block);
            }
            return Ok((Stmt { span: start.to(self.prev_span()), kind: StmtKind::Expr(e) }, false));
        }
        let e = self.expr()?;
        self.finish_expr_stmt(e, start, in_block)
    }

    fn finish_expr_stmt(&mut self, e: Expr, start: Span, in_block: bool) -> PResult<(Stmt, bool)> {
        if self.eat_sym(";") {
            return Ok((Stmt { span: start.to(self.prev_span()), kind: StmtKind::Expr(e) }, false));
        }
        if in_block && self.at_sym("}") {
            return Ok((Stmt { span: start.to(self.prev_span()), kind: StmtKind::Expr(e) }, true));
        }
        Err(Diagnostic::error(self.prev_span(), "falta `;` al final de la sentencia").with_label("se esperaba `;` después de esto"))
    }

    /// `(x $int, y $int) = expr;` / `(a, b) := expr;`. Devuelve None si no encaja (sin consumir).
    fn try_destructure(&mut self) -> PResult<Option<Stmt>> {
        let save = self.pos;
        let start = self.span();
        self.advance();
        let mut targets = vec![];
        let ok = loop {
            if self.at_sym(")") { break !targets.is_empty(); }
            let Ok(name) = self.ident() else { break false };
            let ty = if self.at_sym(",") || self.at_sym(")") { None } else {
                match self.type_() { Ok(t) => Some(t), Err(_) => break false }
            };
            targets.push(DestructTarget { name, ty });
            if !self.eat_sym(",") {
                break self.at_sym(")");
            }
        };
        if !ok || !self.eat_sym(")") || !(self.at_sym("=") || self.at_sym(":=")) || targets.len() < 2 {
            self.pos = save;
            return Ok(None);
        }
        self.advance();
        let init = self.expr()?;
        self.expect_sym(";")?;
        Ok(Some(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Destructure { targets, init } }))
    }

    // ---------- expresiones ----------
    pub fn expr(&mut self) -> PResult<Expr> {
        self.assign_expr()
    }

    fn assign_expr(&mut self) -> PResult<Expr> {
        let lhs = self.ternary_expr()?;
        let op = match self.tok() {
            Tok::Sym("=") => Some(None),
            Tok::Sym("+=") => Some(Some(BinOp::Add)),
            Tok::Sym("-=") => Some(Some(BinOp::Sub)),
            Tok::Sym("*=") => Some(Some(BinOp::Mul)),
            Tok::Sym("/=") => Some(Some(BinOp::Div)),
            Tok::Sym("%=") => Some(Some(BinOp::Rem)),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let rhs = self.assign_expr()?;
            let sp = lhs.span.to(rhs.span);
            return Ok(Expr { kind: ExprKind::Assign(op, Box::new(lhs), Box::new(rhs)), span: sp });
        }
        Ok(lhs)
    }

    fn ternary_expr(&mut self) -> PResult<Expr> {
        let cond = self.binary_expr(1)?;
        if self.at_sym("?") {
            self.advance();
            let saved = self.no_struct;
            self.no_struct = false;
            let a = self.assign_expr()?;
            self.no_struct = saved;
            self.expect_sym(":")?;
            let b = self.ternary_expr()?;
            let sp = cond.span.to(b.span);
            return Ok(Expr { kind: ExprKind::Ternary(Box::new(cond), Box::new(a), Box::new(b)), span: sp });
        }
        Ok(cond)
    }

    fn binop_info(&self) -> Option<(u8, BinOp)> {
        Some(match self.tok() {
            Tok::Sym("||") => (1, BinOp::Or),
            Tok::Sym("&&") => (2, BinOp::And),
            Tok::Sym("|") => (3, BinOp::BitOr),
            Tok::Sym("^^") => (4, BinOp::BitXor),
            Tok::Sym("&") => (5, BinOp::BitAnd),
            Tok::Sym("==") => (6, BinOp::Eq),
            Tok::Sym("!=") => (6, BinOp::Ne),
            Tok::Sym("===") => (6, BinOp::RefEq),
            Tok::Sym("<") => (7, BinOp::Lt),
            Tok::Sym(">") => (7, BinOp::Gt),
            Tok::Sym("<=") => (7, BinOp::Le),
            Tok::Sym(">=") => (7, BinOp::Ge),
            Tok::Sym("<<") => (8, BinOp::Shl),
            Tok::Sym(">>") => (8, BinOp::Shr),
            Tok::Sym("+") => (9, BinOp::Add),
            Tok::Sym("-") => (9, BinOp::Sub),
            Tok::Sym("*") => (10, BinOp::Mul),
            Tok::Sym("/") => (10, BinOp::Div),
            Tok::Sym("%") => (10, BinOp::Rem),
            _ => return None,
        })
    }

    fn binary_expr(&mut self, min: u8) -> PResult<Expr> {
        let mut lhs = self.power_expr()?;
        while let Some((prec, op)) = self.binop_info() {
            if prec < min { break; }
            self.advance();
            let rhs = self.binary_expr(prec + 1)?;
            let sp = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span: sp };
        }
        Ok(lhs)
    }

    fn power_expr(&mut self) -> PResult<Expr> {
        let base = self.cast_expr()?;
        if self.at_sym("^") {
            self.advance();
            let exp = self.power_expr()?;
            let sp = base.span.to(exp.span);
            return Ok(Expr { kind: ExprKind::Binary(BinOp::Pow, Box::new(base), Box::new(exp)), span: sp });
        }
        Ok(base)
    }

    fn cast_expr(&mut self) -> PResult<Expr> {
        let mut e = self.unary_expr()?;
        while self.at_kw("as") || self.at_kw("as!") {
            let lossy = self.at_kw("as!");
            self.advance();
            let ty = self.type_atom()?;
            let sp = e.span.to(ty.span);
            e = Expr { kind: ExprKind::Cast { expr: Box::new(e), ty, lossy }, span: sp };
        }
        Ok(e)
    }

    fn unary_expr(&mut self) -> PResult<Expr> {
        let start = self.span();
        let op = match self.tok() {
            Tok::Sym("!") => Some(UnOp::Not),
            Tok::Sym("~") => Some(UnOp::BitNot),
            Tok::Sym("-") => Some(UnOp::Neg),
            Tok::Sym("&") => Some(UnOp::Ref),
            Tok::Sym("*") => Some(UnOp::Deref),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let inner = self.unary_expr()?;
            let sp = start.to(inner.span);
            return Ok(Expr { kind: ExprKind::Unary(op, Box::new(inner)), span: sp });
        }
        if self.at_sym("++") || self.at_sym("--") {
            let inc = self.at_sym("++");
            self.advance();
            let inner = self.unary_expr()?;
            let sp = start.to(inner.span);
            return Ok(Expr { kind: ExprKind::IncDec { target: Box::new(inner), inc, prefix: true }, span: sp });
        }
        if self.at_kw("await") {
            self.advance();
            let inner = self.unary_expr()?;
            let sp = start.to(inner.span);
            return Ok(Expr { kind: ExprKind::Await(Box::new(inner)), span: sp });
        }
        let prim = self.primary()?;
        self.postfix_from(prim)
    }

    fn is_upper_name(e: &Expr) -> Option<Vec<Ident>> {
        match &e.kind {
            ExprKind::Ident(n) if n.chars().next().is_some_and(|c| c.is_uppercase()) => {
                Some(vec![Ident { name: n.clone(), span: e.span }])
            }
            ExprKind::Field(b, f) if f.name.chars().next().is_some_and(|c| c.is_uppercase()) => {
                let mut p = Self::path_of(b)?;
                p.push(f.clone());
                Some(p)
            }
            _ => None,
        }
    }

    fn path_of(e: &Expr) -> Option<Vec<Ident>> {
        match &e.kind {
            ExprKind::Ident(n) => Some(vec![Ident { name: n.clone(), span: e.span }]),
            ExprKind::Field(b, f) => {
                let mut p = Self::path_of(b)?;
                p.push(f.clone());
                Some(p)
            }
            _ => None,
        }
    }

    fn looks_like_struct_lit(&self) -> bool {
        self.at_sym("{")
            && (self.nth_sym(1, "}") || (matches!(self.nth(1), Tok::Ident(_)) && self.nth_sym(2, ":")))
    }

    fn postfix_from(&mut self, mut e: Expr) -> PResult<Expr> {
        loop {
            match self.tok() {
                Tok::Sym("(") => {
                    self.advance();
                    let saved = self.no_struct;
                    self.no_struct = false;
                    let mut args = vec![];
                    while !self.at_sym(")") {
                        args.push(self.expr()?);
                        if !self.eat_sym(",") { break; }
                    }
                    self.no_struct = saved;
                    let end = self.expect_sym(")")?;
                    let sp = e.span.to(end);
                    e = Expr { kind: ExprKind::Call { callee: Box::new(e), args }, span: sp };
                }
                Tok::Sym("[") => {
                    self.advance();
                    let saved = self.no_struct;
                    self.no_struct = false;
                    let r = self.index_or_slice(e);
                    self.no_struct = saved;
                    e = r?;
                }
                Tok::Sym(".") => {
                    self.advance();
                    let name = match self.tok().clone() {
                        Tok::Ident(n) => { let sp = self.advance().span; Ident { name: n, span: sp } }
                        Tok::Int(_) => return Err(Diagnostic::error(self.span(), "el acceso `x.0` no existe en v0.1; usa `x[0]` para tuplas")),
                        _ => return Err(self.unexpected("un nombre de campo o método")),
                    };
                    let sp = e.span.to(name.span);
                    e = Expr { kind: ExprKind::Field(Box::new(e), name), span: sp };
                }
                Tok::Sym("++") | Tok::Sym("--") => {
                    let inc = self.at_sym("++");
                    let sp = self.advance().span;
                    let full = e.span.to(sp);
                    e = Expr { kind: ExprKind::IncDec { target: Box::new(e), inc, prefix: false }, span: full };
                }
                Tok::Sym("?") if self.toks[self.pos].span.lo == self.toks[self.pos - 1].span.hi => {
                    let sp = self.advance().span;
                    let full = e.span.to(sp);
                    e = Expr { kind: ExprKind::Try(Box::new(e)), span: full };
                }
                Tok::Sym("{") if !self.no_struct && self.looks_like_struct_lit() && Self::is_upper_name(&e).is_some() => {
                    let path = Self::is_upper_name(&e).unwrap();
                    self.advance();
                    let mut fields = vec![];
                    while !self.at_sym("}") {
                        let f = self.ident()?;
                        self.expect_sym(":")?;
                        let v = self.expr()?;
                        fields.push((f, v));
                        if !self.eat_sym(",") { break; }
                    }
                    let end = self.expect_sym("}")?;
                    let sp = e.span.to(end);
                    e = Expr { kind: ExprKind::StructLit { path, fields, key: String::new() }, span: sp };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn index_or_slice(&mut self, base: Expr) -> PResult<Expr> {
        let lo = if self.at_sym("..") { None } else { Some(Box::new(self.expr()?)) };
        if self.eat_sym("..") {
            let hi = if self.at_sym("]") { None } else { Some(Box::new(self.expr()?)) };
            let end = self.expect_sym("]")?;
            let sp = base.span.to(end);
            return Ok(Expr { kind: ExprKind::Slice { base: Box::new(base), lo, hi }, span: sp });
        }
        let end = self.expect_sym("]")?;
        let sp = base.span.to(end);
        Ok(Expr { kind: ExprKind::Index(Box::new(base), lo.unwrap()), span: sp })
    }

    fn no_struct_expr(&mut self) -> PResult<Expr> {
        let saved = self.no_struct;
        self.no_struct = true;
        let r = self.expr();
        self.no_struct = saved;
        r
    }

    fn block_like(&mut self) -> PResult<Expr> {
        let start = self.span();
        match self.tok().clone() {
            Tok::Sym("{") => {
                let b = self.block()?;
                let sp = b.span;
                Ok(Expr { kind: ExprKind::Block(b), span: sp })
            }
            Tok::Kw("unsafe") => {
                self.advance();
                let b = self.block()?;
                Ok(Expr { span: start.to(b.span), kind: ExprKind::Unsafe(b) })
            }
            Tok::Kw("if") => self.if_expr(),
            Tok::Kw("match") => {
                self.advance();
                let scrut = self.no_struct_expr()?;
                self.expect_sym("{")?;
                let saved = (self.no_struct, self.no_union);
                self.no_struct = false;
                self.no_union = false;
                let mut arms = vec![];
                while !self.at_sym("}") {
                    if matches!(self.tok(), Tok::Eof) { return Err(self.unexpected("`}`")); }
                    arms.push(self.match_arm()?);
                }
                self.no_struct = saved.0;
                self.no_union = saved.1;
                let end = self.expect_sym("}")?;
                Ok(Expr { kind: ExprKind::Match { scrut: Box::new(scrut), arms }, span: start.to(end) })
            }
            Tok::Kw("while") => {
                self.advance();
                let cond = self.no_struct_expr()?;
                let body = self.block()?;
                Ok(Expr { span: start.to(body.span), kind: ExprKind::While { cond: Box::new(cond), body } })
            }
            Tok::Kw("loop") => self.loop_expr(),
            _ => Err(self.unexpected("una expresión")),
        }
    }

    fn if_expr(&mut self) -> PResult<Expr> {
        let start = self.expect_kw("if")?;
        let cond = self.no_struct_expr()?;
        let then = self.block()?;
        let mut end = then.span;
        let els = if self.eat_kw("else") {
            let e = if self.at_kw("if") { self.if_expr()? } else {
                let b = self.block()?;
                let sp = b.span;
                Expr { kind: ExprKind::Block(b), span: sp }
            };
            end = e.span;
            Some(Box::new(e))
        } else { None };
        Ok(Expr { kind: ExprKind::If { cond: Box::new(cond), then, els }, span: start.to(end) })
    }

    fn loop_expr(&mut self) -> PResult<Expr> {
        let start = self.expect_kw("loop")?;
        self.expect_sym("(")?;
        // ¿hay `;` al nivel 0 de los paréntesis? → forma C
        let mut depth = 0i32;
        let mut c_form = false;
        let mut i = self.pos;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::Sym("(") | Tok::Sym("[") | Tok::Sym("{") => depth += 1,
                Tok::Sym(")") | Tok::Sym("]") | Tok::Sym("}") => {
                    if depth == 0 { break; }
                    depth -= 1;
                }
                Tok::Sym(";") if depth == 0 => { c_form = true; break; }
                Tok::Eof => break,
                _ => {}
            }
            i += 1;
        }
        let kind = if c_form {
            let init = if self.at_sym(";") { None } else { Some(Box::new(self.loop_init()?)) };
            self.expect_sym(";")?;
            let cond = if self.at_sym(";") { None } else { Some(Box::new(self.expr()?)) };
            self.expect_sym(";")?;
            let update = if self.at_sym(")") { None } else { Some(Box::new(self.expr()?)) };
            LoopKind::C { init, cond, update }
        } else {
            let var = self.ident()?;
            let ty = if self.at_sym(":") { None } else { Some(self.type_()?) };
            self.expect_sym(":")?;
            let iter = self.expr()?;
            LoopKind::Each { var, ty, iter: Box::new(iter) }
        };
        self.expect_sym(")")?;
        let body = self.block()?;
        Ok(Expr { span: start.to(body.span), kind: ExprKind::Loop { kind, body } })
    }

    fn loop_init(&mut self) -> PResult<Stmt> {
        let start = self.span();
        if self.at_kw("let") {
            self.advance();
            let name = self.ident()?;
            let ty = if self.at_sym("=") { None } else { Some(self.type_()?) };
            self.expect_sym("=")?;
            let init = Some(self.expr()?);
            return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Let { name, ty, init } });
        }
        if matches!(self.tok(), Tok::Ident(_)) && self.nth_sym(1, ":=") {
            let name = self.ident()?;
            self.advance();
            let init = Some(self.expr()?);
            return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Let { name, ty: None, init } });
        }
        if matches!(self.tok(), Tok::Ident(_)) && self.type_start_after_ident() {
            let name = self.ident()?;
            let ty = Some(self.type_()?);
            self.expect_sym("=")?;
            let init = Some(self.expr()?);
            return Ok(Stmt { span: start.to(self.prev_span()), kind: StmtKind::Let { name, ty, init } });
        }
        let e = self.expr()?;
        Ok(Stmt { span: e.span, kind: StmtKind::Expr(e) })
    }

    fn match_arm(&mut self) -> PResult<MatchArm> {
        let start = self.span();
        let pattern = self.pattern()?;
        let guard = if self.eat_kw("if") { Some(self.expr()?) } else { None };
        self.expect_sym("=>")?;
        let (body, was_block) = if self.at_sym("{") || (self.at_kw("unsafe") && self.nth_sym(1, "{")) {
            (self.block_like()?, true)
        } else {
            (self.expr()?, false)
        };
        let sep = self.eat_sym(";") || self.eat_sym(",");
        if !sep && !was_block && !self.at_sym("}") {
            return Err(Diagnostic::error(self.prev_span(), "falta `;` o `,` al final del brazo del match"));
        }
        Ok(MatchArm { span: start.to(body.span), pattern, guard, body })
    }

    fn pattern(&mut self) -> PResult<Pattern> {
        let start = self.span();
        match self.tok().clone() {
            Tok::Ident(n) if n == "_" => {
                self.advance();
                Ok(Pattern::Wild(start))
            }
            Tok::Ident(n) if n.chars().all(|c| c == '_') => Err(Diagnostic::error(start, "`_` es el único wildcard; `__` o más no son válidos")),
            Tok::Ident(_) => {
                let mut path = vec![self.ident()?];
                while self.at_sym(".") {
                    self.advance();
                    path.push(self.ident()?);
                }
                if self.at_sym("(") {
                    self.advance();
                    let mut args = vec![];
                    while !self.at_sym(")") {
                        args.push(self.pattern()?);
                        if !self.eat_sym(",") { break; }
                    }
                    let end = self.expect_sym(")")?;
                    return Ok(Pattern::Variant { path, args: Some(args), span: start.to(end), res: None });
                }
                if path.len() > 1 || path[0].name == "None" {
                    return Ok(Pattern::Variant { path, args: None, span: start.to(self.prev_span()), res: None });
                }
                let name = path.pop().unwrap();
                let ty = if matches!(self.tok(), Tok::Prim(_) | Tok::Ident(_) | Tok::Sym("&")) && !self.at_ident("if") {
                    Some(self.type_postfix()?)
                } else { None };
                Ok(Pattern::Bind { name, ty })
            }
            Tok::Sym("(") => {
                self.advance();
                let mut items = vec![];
                while !self.at_sym(")") {
                    items.push(self.pattern()?);
                    if !self.eat_sym(",") { break; }
                }
                let end = self.expect_sym(")")?;
                Ok(Pattern::Tuple(items, start.to(end)))
            }
            Tok::Int(_) | Tok::Float(_) | Tok::Char(_) | Tok::Str(_) | Tok::Kw("true") | Tok::Kw("false") | Tok::Kw("null") | Tok::Sym("-") => {
                let e = self.unary_expr()?;
                Ok(Pattern::Lit(Box::new(e)))
            }
            _ => Err(self.unexpected("un patrón")),
        }
    }

    fn string_expr(&mut self, parts: Vec<StrPart>, span: Span) -> PResult<Expr> {
        let mut out = vec![];
        for p in parts {
            match p {
                StrPart::Lit(s) => out.push(StrPiece::Lit(s)),
                StrPart::Expr(lo, hi) => {
                    let (toks, diags) = lexer::lex_range(self.file, self.src, lo, hi, false);
                    if let Some(d) = diags.into_iter().next() { return Err(d); }
                    let mut sub = Parser { toks, pos: 0, file: self.file, src: self.src, diags: vec![], no_struct: false, no_union: false };
                    let e = sub.expr()?;
                    if !matches!(sub.tok(), Tok::Eof) {
                        return Err(sub.unexpected("el fin de la interpolación `}`"));
                    }
                    out.push(StrPiece::Expr(e));
                }
            }
        }
        Ok(Expr { kind: ExprKind::Str(out), span })
    }

    /// Intenta `(params) -> tipo { cuerpo }`; None si no es una función anónima.
    fn try_closure(&mut self) -> Option<Expr> {
        let save = self.pos;
        let start = self.span();
        let r = (|| -> PResult<Expr> {
            self.expect_sym("(")?;
            let mut params = vec![];
            while !self.at_sym(")") {
                let name = self.ident()?;
                let ty = self.type_()?;
                params.push(ClosureParam { name, ty: Some(ty) });
                if !self.eat_sym(",") { break; }
            }
            self.expect_sym(")")?;
            self.expect_sym("->")?;
            let ret = self.type_()?;
            let body = self.block()?;
            let sp = start.to(body.span);
            Ok(Expr { kind: ExprKind::Closure { params, ret: Some(ret), body: Box::new(Expr { span: body.span, kind: ExprKind::Block(body) }) }, span: sp })
        })();
        match r {
            Ok(e) => Some(e),
            Err(_) => {
                self.pos = save;
                None
            }
        }
    }

    /// `|| cuerpo` / `|x $int, y| cuerpo` (forma corta usada en `thread.spawn(|| { .. })`).
    fn short_closure(&mut self) -> PResult<Expr> {
        let start = self.span();
        let mut params = vec![];
        if self.eat_sym("||") {
        } else {
            self.expect_sym("|")?;
            let saved = self.no_union;
            self.no_union = true;
            while !self.at_sym("|") {
                let name = self.ident()?;
                let ty = if self.at_sym(",") || self.at_sym("|") { None } else { Some(self.type_()?) };
                params.push(ClosureParam { name, ty });
                if !self.eat_sym(",") { break; }
            }
            self.no_union = saved;
            self.expect_sym("|")?;
        }
        let ret = if self.eat_sym("->") { Some(self.type_()?) } else { None };
        let body = if self.at_sym("{") { self.block_like()? } else { self.expr()? };
        let sp = start.to(body.span);
        Ok(Expr { kind: ExprKind::Closure { params, ret, body: Box::new(body) }, span: sp })
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        match self.tok().clone() {
            Tok::Int(v) => { self.advance(); Ok(Expr { kind: ExprKind::Int(v), span: start }) }
            Tok::Float(v) => { self.advance(); Ok(Expr { kind: ExprKind::Float(v), span: start }) }
            Tok::Char(c) => { self.advance(); Ok(Expr { kind: ExprKind::Char(c), span: start }) }
            Tok::Str(parts) => { self.advance(); self.string_expr(parts, start) }
            Tok::Kw("true") => { self.advance(); Ok(Expr { kind: ExprKind::Bool(true), span: start }) }
            Tok::Kw("false") => { self.advance(); Ok(Expr { kind: ExprKind::Bool(false), span: start }) }
            Tok::Kw("null") => { self.advance(); Ok(Expr { kind: ExprKind::Null, span: start }) }
            Tok::Kw("nothing") => Err(Diagnostic::error(start, "`nothing` no es un valor; solo existe como tipo de retorno")
                .with_note("para terminar una función `-> nothing` usa `return;`")),
            Tok::Kw(k @ ("self" | "Self" | "assert" | "panic" | "exit")) => {
                self.advance();
                Ok(Expr { kind: ExprKind::Ident(k.to_string()), span: start })
            }
            Tok::Ident(n) if (n == "map" || n == "set") && self.nth_sym(1, "{") => {
                self.advance();
                self.advance();
                let saved = (self.no_struct, self.no_union);
                self.no_struct = false;
                let mut entries = vec![];
                let mut items = vec![];
                while !self.at_sym("}") {
                    let k = self.expr()?;
                    if n == "map" {
                        self.expect_sym(":")?;
                        let v = self.expr()?;
                        entries.push((k, v));
                    } else {
                        items.push(k);
                    }
                    if !self.eat_sym(",") { break; }
                }
                self.no_struct = saved.0;
                self.no_union = saved.1;
                let end = self.expect_sym("}")?;
                let kind = if n == "map" { ExprKind::Map(entries) } else { ExprKind::Set(items) };
                Ok(Expr { kind, span: start.to(end) })
            }
            Tok::Ident(n) if n == "asm" && self.nth_sym(1, "!") => {
                self.advance();
                self.advance();
                self.expect_sym("(")?;
                let text = self.plain_string()?;
                let end = self.expect_sym(")")?;
                Ok(Expr { kind: ExprKind::Asm(text), span: start.to(end) })
            }
            Tok::Ident(n) => {
                if n.chars().all(|c| c == '_') {
                    return Err(Diagnostic::error(start, "`_` es un wildcard reservado y no puede usarse como valor"));
                }
                self.advance();
                Ok(Expr { kind: ExprKind::Ident(n), span: start })
            }
            Tok::Sym("(") => {
                if let Some(c) = self.try_closure() { return Ok(c); }
                self.advance();
                let saved = (self.no_struct, self.no_union);
                self.no_struct = false;
                self.no_union = false;
                let mut items = vec![];
                let mut trailing_comma = false;
                while !self.at_sym(")") {
                    items.push(self.expr()?);
                    trailing_comma = false;
                    if !self.eat_sym(",") { break; }
                    trailing_comma = true;
                }
                self.no_struct = saved.0;
                self.no_union = saved.1;
                let end = self.expect_sym(")")?;
                if items.len() == 1 && !trailing_comma {
                    let mut e = items.pop().unwrap();
                    e.span = start.to(end);
                    return Ok(e);
                }
                if items.len() < 2 {
                    return Err(Diagnostic::error(start.to(end), "una tupla necesita al menos dos elementos"));
                }
                Ok(Expr { kind: ExprKind::Tuple(items), span: start.to(end) })
            }
            Tok::Sym("[") => {
                self.advance();
                let saved = (self.no_struct, self.no_union);
                self.no_struct = false;
                self.no_union = false;
                let mut items = vec![];
                while !self.at_sym("]") {
                    items.push(self.expr()?);
                    if !self.eat_sym(",") { break; }
                }
                self.no_struct = saved.0;
                self.no_union = saved.1;
                let end = self.expect_sym("]")?;
                Ok(Expr { kind: ExprKind::Array(items), span: start.to(end) })
            }
            Tok::Sym("|") | Tok::Sym("||") => self.short_closure(),
            Tok::Kw("return") => {
                self.advance();
                let value = if self.at_sym(";") || self.at_sym("}") || self.at_sym(",") { None } else { Some(Box::new(self.expr()?)) };
                Ok(Expr { span: start.to(self.prev_span()), kind: ExprKind::Return(value) })
            }
            Tok::Kw("break") => {
                self.advance();
                Ok(Expr { kind: ExprKind::Break, span: start })
            }
            Tok::Kw("if") | Tok::Kw("match") | Tok::Kw("while") | Tok::Kw("loop") | Tok::Sym("{") => self.block_like(),
            Tok::Kw("unsafe") if self.nth_sym(1, "{") => self.block_like(),
            _ => Err(self.unexpected("una expresión")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> Module {
        let (m, d) = parse_module(0, src);
        assert!(d.is_empty(), "{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
        m
    }

    #[test]
    fn parses_function_and_statements() {
        let m = ok("fn add(a $int, b $int) -> $int { return a + b; }\nfn main() -> nothing { x := add(1, 2); println(x); }");
        assert_eq!(m.items.len(), 2);
    }

    #[test]
    fn parses_types() {
        ok("let a List<User?>; let b List<$int>?; let c Map<$string, List<$int>>; let d $int | $string; let e $int[2][3]; let f ($int, $int) -> $int;");
    }

    #[test]
    fn parses_try_vs_ternary() {
        ok("fn f() -> Result<$int, $string> { let v = g()?; let w = v > 1 ? 2 : 3; return Ok(w); }");
    }

    #[test]
    fn parses_loops_and_match() {
        ok("fn f() -> nothing { loop (i := 0; i < 3; i++) { } loop (x : xs) { } loop (;;) { break; } while a { } match s { Status.A => 1; Some(v) if v > 1 => 2; _ => 3, } }");
    }

    #[test]
    fn requires_return_type() {
        let (_, d) = parse_module(0, "fn f() { }");
        assert!(!d.is_empty());
    }

    #[test]
    fn struct_literal_not_in_condition() {
        ok("fn f() -> nothing { if x == y { } let u = User { name: \"a\", age: 1 }; }");
    }
}
