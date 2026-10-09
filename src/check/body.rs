//! Verificación de items, cuerpos de función, ámbitos, bloques y sentencias.

use super::decls::TyScope;
use super::*;

impl Checker {
    pub(super) fn check_module(&mut self, mi: usize, ast: &mut Module, main_idx: usize) {
        self.cur_module = mi;
        let is_std = self.mod_std[mi];
        let mut has_main = false;
        let mut main_span = Span::default();
        let mut first_stmt: Option<Span> = None;
        for (ii, it) in ast.items.iter_mut().enumerate() {
            let sigs = self.item_sigs[mi][ii].clone();
            match &mut it.kind {
                ItemKind::Fn(f) => {
                    if f.name.name == "main" {
                        has_main = true;
                        main_span = f.name.span;
                        self.check_main_sig(f, sigs[0]);
                    }
                    if f.body.is_some() {
                        self.check_fn(f, sigs[0], None, vec![], mi);
                    } else if !is_std && !f.is_extern {
                        self.err_label(f.span, format!("la función `{}` no tiene cuerpo", f.name.name), "se esperaba `{ ... }`");
                    }
                }
                ItemKind::Impl(imp) => {
                    let ii_idx = self.impl_of_item[&(mi, ii)];
                    let info = self.impls[ii_idx].clone();
                    for (m, sid) in imp.methods.iter_mut().zip(sigs) {
                        if m.body.is_some() {
                            self.check_fn(m, sid, Some(info.target.clone()), info.generics.clone(), mi);
                        }
                    }
                }
                ItemKind::Trait(t) => {
                    let key = crate::types::qualify(&self.mod_names[mi], &t.name.name);
                    let ti = self.traits[&key].clone();
                    let mut gens = ti.generics.clone();
                    gens.push(GenericInfo {
                        name: Rc::from("Self"),
                        bounds: vec![TraitRef { key: key.clone(), args: ti.generics.iter().map(|g| Ty::Param(g.name.clone())).collect() }],
                    });
                    for (m, sid) in t.methods.iter_mut().zip(sigs) {
                        if m.body.is_some() {
                            self.check_fn(m, sid, Some(Ty::Param(Rc::from("Self"))), gens.clone(), mi);
                        }
                    }
                }
                ItemKind::Const(c) | ItemKind::Static(c) => {
                    let (ty, _) = self.mods[mi].consts.get(&c.name.name).cloned().unwrap_or((Ty::Error, false));
                    self.fcx.push(self.plain_ctx(mi, Ty::Nothing));
                    self.push_scope();
                    let t = self.check_expr(&mut c.value, Some(&ty));
                    if !self.coerce(&mut c.value, &t, &ty) {
                        self.mismatch(c.value.span, &ty, &t);
                    }
                    if !self.is_const_expr(&c.value) {
                        self.err_label(c.value.span, format!("el valor de `{}` no es evaluable en tiempo de compilación", c.name.name), "no es una expresión constante");
                    }
                    self.pop_scope();
                    self.fcx.pop();
                    self.check_pending_bounds();
                }
                ItemKind::Test { body, .. } | ItemKind::Bench { body, .. } => {
                    self.fcx.push(self.plain_ctx(mi, Ty::Nothing));
                    self.check_block(body, Some(&Ty::Nothing), true);
                    self.fcx.pop();
                    self.check_pending_bounds();
                }
                ItemKind::Macro { name } => {
                    self.err_label(name.span, "las macros están reservadas para una versión futura de N# (semántica UNSPECIFIED en v0.1)", "macro no soportada");
                }
                ItemKind::Stmt(s) => {
                    if first_stmt.is_none() {
                        first_stmt = Some(s.span);
                    }
                }
                ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Import(_) | ItemKind::Alias(_) | ItemKind::Extern { .. } => {}
            }
        }
        // código global (script mode)
        if let Some(sp) = first_stmt {
            if has_main {
                self.diags.push(
                    Diagnostic::error(sp, "no puede existir a la vez código global y `fn main`")
                        .with_label("código global")
                        .with_note("usa solo código global (script) o solo `fn main`"),
                );
            } else if self.opts.mode == Mode::Project || mi != main_idx {
                self.err_label(sp, "el código global solo se permite en script mode (`ncom archivo.ncs`)", "código global no permitido aquí");
            }
            self.fcx.push(self.plain_ctx(mi, Ty::Nothing));
            self.push_scope();
            let mut diverged = false;
            for it in ast.items.iter_mut() {
                if let ItemKind::Stmt(s) = &mut it.kind {
                    let t = self.check_stmt(s);
                    if t == Ty::Never { diverged = true; }
                }
            }
            let _ = diverged;
            self.pop_scope();
            self.fcx.pop();
            self.check_pending_bounds();
        } else if mi == main_idx && !has_main && self.opts.require_main && !is_std {
            self.diags.push(
                Diagnostic::error(Span::new(self.files_of(mi), 0, 0), "no se encontró la función `main`")
                    .with_note("define `fn main() -> nothing { ... }` o `fn main() -> $int { ... }`"),
            );
        }
        let _ = main_span;
    }

    fn files_of(&self, _mi: usize) -> u32 {
        // los spans de módulo vacío apuntan al primer archivo del usuario
        0
    }

    fn check_main_sig(&mut self, f: &FnDecl, sid: usize) {
        let sig = self.sigs[sid].clone();
        if !sig.params.is_empty() {
            self.err_label(f.name.span, "`main` no admite parámetros; usa `args()` para leer los argumentos", "parámetros no permitidos");
        }
        if !matches!(sig.ret, Ty::Nothing | Ty::Int | Ty::Error) {
            self.err_label(f.ret.span, format!("`main` debe devolver `nothing` o `$int`, no `{}`", sig.ret), "tipo de retorno no válido");
        }
        if sig.is_async {
            self.err(f.name.span, "`main` no puede ser `async`");
        }
    }

    pub(super) fn plain_ctx(&self, mi: usize, ret: Ty) -> FnCtx {
        FnCtx {
            ret,
            is_async: false,
            unsafe_depth: 0,
            loop_depth: 0,
            breaks: vec![],
            generics: vec![],
            self_ty: None,
            module: mi,
            is_main: false,
            scope_base: self.scopes.len(),
        }
    }

    pub(super) fn body_scope(&self) -> TyScope {
        let mut generics: Vec<Rc<str>> = Vec::new();
        let mut self_ty = None;
        for c in &self.fcx {
            for g in &c.generics {
                generics.push(g.name.clone());
            }
            if c.self_ty.is_some() {
                self_ty = c.self_ty.clone();
            }
        }
        TyScope { module: self.cur_module, generics, self_ty }
    }

    fn check_fn(&mut self, f: &mut FnDecl, sid: usize, self_ty: Option<Ty>, outer: Vec<GenericInfo>, mi: usize) {
        self.cur_module = mi;
        let sig = self.sigs[sid].clone();
        let mut generics = outer;
        generics.extend(sig.generics.clone());
        let is_main = f.name.name == "main" && self_ty.is_none();
        self.fcx.push(FnCtx {
            ret: sig.ret.clone(),
            is_async: sig.is_async,
            unsafe_depth: u32::from(sig.is_unsafe),
            loop_depth: 0,
            breaks: vec![],
            generics,
            self_ty: self_ty.clone(),
            module: mi,
            is_main,
            scope_base: self.scopes.len(),
        });
        self.push_scope();
        if let Some(st) = &self_ty {
            self.declare("self", st.clone(), f.name.span, true, false);
        }
        for (p, ty) in f.params.iter().zip(sig.params.iter()) {
            self.declare(&p.name.name, ty.clone(), p.name.span, true, false);
        }
        let ret = sig.ret.clone();
        let body = f.body.as_mut().unwrap();
        let discard = ret == Ty::Nothing;
        let t = self.check_block(body, Some(&ret), discard);
        self.finish_body(body, t, &ret, f.name.span, &f.name.name);
        self.pop_scope();
        self.fcx.pop();
        self.check_pending_bounds();
    }

    /// Comprueba que el cuerpo devuelve `ret` en todos los caminos.
    pub(super) fn finish_body(&mut self, body: &mut Block, t: Ty, ret: &Ty, name_span: Span, name: &str) {
        if t == Ty::Never || *ret == Ty::Nothing || ret.is_error() {
            return;
        }
        let tail_ty = self.last_tail_ty.take().unwrap_or(t);
        let missing = match &mut body.tail {
            Some(tail) => {
                if self.shallow(&tail_ty) == Ty::Nothing {
                    true
                } else {
                    if !self.coerce(tail, &tail_ty, ret) {
                        self.mismatch(tail.span, ret, &tail_ty);
                    }
                    false
                }
            }
            None => true,
        };
        if missing {
            self.diags.push(
                Diagnostic::error(name_span, format!("la función `{name}` debe devolver `{}` en todos los caminos", self.resolve(ret)))
                    .with_label("falta un `return` en algún camino")
                    .with_note("termina cada camino con `return <valor>;`"),
            );
        }
    }

    // ---------- ámbitos ----------

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(super) fn pop_scope(&mut self) {
        if let Some(sc) = self.scopes.pop() {
            let mut ids: Vec<usize> = sc.values().copied().collect();
            ids.sort();
            for id in ids {
                let v = self.vars[id].clone();
                if v.warn_unused && !v.used && !v.name.starts_with('_') {
                    self.warn("unused-variable", v.span, format!("la variable `{}` nunca se usa", v.name));
                    if let Some(d) = self.diags.last_mut() {
                        if d.severity == crate::diag::Severity::Warning {
                            d.notes.insert(0, "si es intencional, prefija el nombre con `_`".into());
                        }
                    }
                }
            }
        }
    }

    pub(super) fn declare(&mut self, name: &str, ty: Ty, span: Span, init: bool, warn: bool) -> usize {
        let id = self.vars.len();
        self.vars.push(VarInfo { name: name.to_string(), ty, span, used: !warn, warn_unused: warn });
        if let Some(sc) = self.scopes.last_mut() {
            if sc.contains_key(name) {
                self.diags.push(Diagnostic::error(span, format!("la variable `{name}` ya está declarada en este ámbito")).with_label("declaración duplicada"));
            }
            sc.insert(name.to_string(), id);
        }
        if !init {
            self.maybe_uninit.insert(id);
        }
        id
    }

    pub(super) fn lookup_var(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    // ---------- bloques y sentencias ----------

    pub(super) fn check_block(&mut self, b: &mut Block, exp: Option<&Ty>, discard: bool) -> Ty {
        self.push_scope();
        let t = self.check_block_inner(b, exp, discard);
        self.pop_scope();
        t
    }

    fn check_block_inner(&mut self, b: &mut Block, exp: Option<&Ty>, discard: bool) -> Ty {
        let mut diverged = false;
        let mut warned = false;
        for s in &mut b.stmts {
            if diverged && !warned {
                self.warn("unreachable-code", s.span, "código inalcanzable");
                warned = true;
            }
            if self.check_stmt(s) == Ty::Never {
                diverged = true;
            }
        }
        let Some(tail) = &mut b.tail else {
            return if diverged { Ty::Never } else { Ty::Nothing };
        };
        if diverged && !warned {
            self.warn("unreachable-code", tail.span, "código inalcanzable");
        }
        if discard {
            let t = self.check_expr_stmt(tail);
            return if diverged || t == Ty::Never { Ty::Never } else { Ty::Nothing };
        }
        let t = self.check_expr(tail, exp);
        self.last_tail_ty = Some(t.clone());
        if diverged { Ty::Never } else { t }
    }

    pub(super) fn check_expr_stmt(&mut self, e: &mut Expr) -> Ty {
        self.discard = true;
        let t = self.check_expr(e, None);
        self.discard = false;
        t
    }

    /// Devuelve `Never` si la sentencia no termina normalmente.
    pub(super) fn check_stmt(&mut self, s: &mut Stmt) -> Ty {
        match &mut s.kind {
            StmtKind::Expr(e) => {
                let t = self.check_expr_stmt(e);
                if self.shallow(&t) == Ty::Never { Ty::Never } else { Ty::Nothing }
            }
            StmtKind::Let { name, ty, init } => {
                let declared = ty.as_ref().map(|te| {
                    let sc = self.body_scope();
                    self.resolve_type(te, &sc)
                });
                match (declared, init.as_mut()) {
                    (Some(t), Some(e)) => {
                        let et = self.check_expr(e, Some(&t));
                        if !self.coerce(e, &et, &t) {
                            self.mismatch(e.span, &t, &et);
                        }
                        let diverges = self.shallow(&et) == Ty::Never;
                        self.declare(&name.name, t, name.span, true, true);
                        if diverges { return Ty::Never; }
                    }
                    (Some(t), None) => {
                        let rt = self.shallow(&t);
                        let coll = match &rt {
                            Ty::Named(k, _) if matches!(&**k, "List" | "Map" | "Set") => Some(k.to_string()),
                            _ => None,
                        };
                        if let Some(key) = coll {
                            let sp = name.span;
                            *init = Some(Expr {
                                kind: ExprKind::Call { callee: Box::new(Expr { kind: ExprKind::TypeStatic { key, name: "new".into() }, span: sp }), args: vec![] },
                                span: sp,
                            });
                            self.declare(&name.name, t, name.span, true, true);
                        } else {
                            self.declare(&name.name, t, name.span, false, true);
                        }
                    }
                    (None, Some(e)) => {
                        let et = self.check_expr(e, None);
                        let r = self.shallow(&et);
                        let vty = match r {
                            Ty::Null => {
                                self.diags.push(
                                    Diagnostic::error(e.span, "no se puede inferir el tipo de `null`")
                                        .with_label("tipo desconocido")
                                        .with_note("anota el tipo: `let x $User? = null;`"),
                                );
                                Ty::Error
                            }
                            Ty::Nothing => {
                                self.err_label(e.span, "la expresión no produce ningún valor y no se puede guardar en una variable", "es de tipo `nothing`");
                                Ty::Error
                            }
                            _ => et,
                        };
                        if vty.contains_var() || matches!(self.shallow(&vty), Ty::Var(_)) {
                            self.pending_infer.push((vty.clone(), name.span, name.name.clone()));
                        }
                        let diverges = self.shallow(&vty) == Ty::Never;
                        self.declare(&name.name, if diverges { Ty::Error } else { vty }, name.span, true, true);
                        if diverges { return Ty::Never; }
                    }
                    (None, None) => {}
                }
                Ty::Nothing
            }
            StmtKind::Destructure { targets, init } => {
                let it = self.check_expr(init, None);
                let r = self.shallow(&it);
                let elem_tys: Vec<Ty> = match r {
                    Ty::Tuple(v) if v.len() == targets.len() => v,
                    Ty::Var(_) => {
                        let vars: Vec<Ty> = targets.iter().map(|_| self.fresh()).collect();
                        self.unify(&r, &Ty::Tuple(vars.clone()));
                        vars
                    }
                    Ty::Error => targets.iter().map(|_| Ty::Error).collect(),
                    other => {
                        let o = self.resolve(&other);
                        self.err_label(init.span, format!("no se puede desestructurar `{o}` en {} variables", targets.len()), "se esperaba una tupla con el mismo número de elementos");
                        targets.iter().map(|_| Ty::Error).collect()
                    }
                };
                for (t, et) in targets.iter().zip(elem_tys) {
                    let vty = match &t.ty {
                        Some(te) => {
                            let sc = self.body_scope();
                            let dt = self.resolve_type(te, &sc);
                            if !self.unify(&dt, &et) {
                                self.mismatch(te.span, &dt, &et);
                            }
                            dt
                        }
                        None => et,
                    };
                    self.declare(&t.name.name, vty, t.name.span, true, true);
                }
                Ty::Nothing
            }
        }
    }

    pub(super) fn is_const_expr(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Char(_) | ExprKind::Null => true,
            ExprKind::Str(parts) => parts.iter().all(|p| match p {
                StrPiece::Lit(_) => true,
                StrPiece::Expr(e) => self.is_const_expr(e),
            }),
            ExprKind::Global { module, name } => self.mods[*module].consts.get(name).is_some_and(|(_, st)| !*st),
            ExprKind::Unary(op, x) => !matches!(op, UnOp::Ref | UnOp::Deref) && self.is_const_expr(x),
            ExprKind::Binary(_, a, b) => self.is_const_expr(a) && self.is_const_expr(b),
            ExprKind::Ternary(a, b, c) => self.is_const_expr(a) && self.is_const_expr(b) && self.is_const_expr(c),
            ExprKind::Cast { expr, .. } => self.is_const_expr(expr),
            ExprKind::Array(v) | ExprKind::Tuple(v) => v.iter().all(|x| self.is_const_expr(x)),
            ExprKind::SomeWrap(x) => self.is_const_expr(x),
            ExprKind::VariantRef { .. } => true,
            ExprKind::Call { callee, args } => matches!(callee.kind, ExprKind::VariantRef { .. }) && args.iter().all(|a| self.is_const_expr(a)),
            _ => false,
        }
    }
}
