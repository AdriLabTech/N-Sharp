//! Tipado de expresiones.

use super::*;

impl Checker {
    pub(super) fn cur_ctx(&mut self) -> &mut FnCtx {
        self.fcx.last_mut().expect("contexto de función")
    }

    pub(super) fn in_unsafe(&self) -> bool {
        self.fcx.last().is_some_and(|c| c.unsafe_depth > 0)
    }

    /// Comprueba `e` esperando `expected` y aplica la coerción; informa del desajuste.
    pub(super) fn check_coerce(&mut self, e: &mut Expr, expected: &Ty) -> Ty {
        let t = self.check_expr(e, Some(expected));
        if !self.coerce(e, &t, expected) {
            self.mismatch(e.span, expected, &t);
        }
        t
    }

    pub(super) fn check_expr(&mut self, e: &mut Expr, exp: Option<&Ty>) -> Ty {
        let discard = std::mem::take(&mut self.discard);
        let span = e.span;
        match &mut e.kind {
            ExprKind::Int(_) => Ty::Int,
            ExprKind::Float(_) => Ty::Float,
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Char(_) => Ty::Char,
            ExprKind::Null => Ty::Null,
            ExprKind::Str(parts) => {
                for p in parts {
                    if let StrPiece::Expr(x) = p {
                        let t = self.check_expr(x, None);
                        if self.shallow(&t) == Ty::Nothing {
                            self.err_label(x.span, "no se puede interpolar un valor `nothing`", "no produce ningún valor");
                        }
                    }
                }
                Ty::Str
            }
            ExprKind::Ident(_) => self.check_ident(e, exp),
            ExprKind::Unary(..) => self.check_unary(e, exp),
            ExprKind::Binary(..) => self.check_binary(e),
            ExprKind::Assign(..) => self.check_assign(e),
            ExprKind::IncDec { .. } => self.check_incdec(e),
            ExprKind::Ternary(c, a, b) => {
                let ct = self.check_expr(c, Some(&Ty::Bool));
                if !self.coerce(c, &ct, &Ty::Bool) {
                    self.mismatch(c.span, &Ty::Bool, &ct);
                }
                let before = self.maybe_uninit.clone();
                let ta = self.check_expr(a, exp);
                let after_a = std::mem::replace(&mut self.maybe_uninit, before);
                let tb = self.check_expr(b, exp.or(Some(&ta)));
                self.maybe_uninit.extend(after_a);
                self.join_branch_types(a, ta, b, tb, exp, span)
            }
            ExprKind::Call { .. } => self.check_call(e, exp),
            ExprKind::Field(..) => self.check_field(e),
            ExprKind::Index(..) => self.check_index(e),
            ExprKind::Slice { base, lo, hi } => {
                let bt = self.check_expr(base, None);
                for x in [lo, hi].into_iter().flatten() {
                    let t = self.check_expr(x, Some(&Ty::Int));
                    if !self.coerce(x, &t, &Ty::Int) {
                        self.mismatch(x.span, &Ty::Int, &t);
                    }
                }
                let mut r = self.shallow(&bt);
                while let Ty::Ref(i) = r.clone() { r = self.shallow(&i); }
                match r {
                    Ty::Str => Ty::Str,
                    Ty::Array(t, _) => Ty::list(*t),
                    Ty::Named(n, a) if &*n == "List" => Ty::list(a[0].clone()),
                    Ty::Error => Ty::Error,
                    other => {
                        let o = self.resolve(&other);
                        self.err(span, format!("no se puede tomar un slice de `{o}`"));
                        Ty::Error
                    }
                }
            }
            ExprKind::Array(items) => {
                let (elem_exp, len_exp) = match exp.map(|t| self.shallow(t)) {
                    Some(Ty::Array(t, n)) => (Some(*t), Some(n)),
                    _ => (None, None),
                };
                let elem = elem_exp.unwrap_or_else(|| self.fresh());
                for it in items.iter_mut() {
                    let t = self.check_expr(it, Some(&elem));
                    if !self.coerce(it, &t, &elem) {
                        self.mismatch(it.span, &elem, &t);
                    }
                }
                let _ = len_exp;
                Ty::Array(Box::new(elem), items.len())
            }
            ExprKind::Tuple(items) => {
                let exps: Option<Vec<Ty>> = match exp.map(|t| self.shallow(t)) {
                    Some(Ty::Tuple(v)) if v.len() == items.len() => Some(v),
                    _ => None,
                };
                let mut tys = Vec::new();
                for (i, it) in items.iter_mut().enumerate() {
                    let t = self.check_expr(it, exps.as_ref().map(|v| &v[i]));
                    tys.push(t);
                }
                Ty::Tuple(tys)
            }
            ExprKind::Map(entries) => {
                let (ke, ve) = match exp.map(|t| self.shallow(t)) {
                    Some(Ty::Named(n, a)) if &*n == "Map" => (a[0].clone(), a[1].clone()),
                    _ => (self.fresh(), self.fresh()),
                };
                for (k, v) in entries.iter_mut() {
                    let kt = self.check_expr(k, Some(&ke));
                    if !self.coerce(k, &kt, &ke) { self.mismatch(k.span, &ke, &kt); }
                    let vt = self.check_expr(v, Some(&ve));
                    if !self.coerce(v, &vt, &ve) { self.mismatch(v.span, &ve, &vt); }
                }
                Ty::named("Map", vec![ke, ve])
            }
            ExprKind::Set(items) => {
                let te = match exp.map(|t| self.shallow(t)) {
                    Some(Ty::Named(n, a)) if &*n == "Set" => a[0].clone(),
                    _ => self.fresh(),
                };
                for it in items.iter_mut() {
                    let t = self.check_expr(it, Some(&te));
                    if !self.coerce(it, &t, &te) { self.mismatch(it.span, &te, &t); }
                }
                Ty::named("Set", vec![te])
            }
            ExprKind::StructLit { .. } => self.check_struct_lit(e, exp),
            ExprKind::If { .. } => self.check_if(e, exp, discard),
            ExprKind::Match { .. } => self.check_match(e, exp, discard),
            ExprKind::Closure { .. } => self.check_closure(e, exp),
            ExprKind::Block(b) => self.check_block(b, exp, discard),
            ExprKind::Unsafe(b) => {
                self.cur_ctx().unsafe_depth += 1;
                let t = self.check_block(b, exp, discard);
                self.cur_ctx().unsafe_depth -= 1;
                t
            }
            ExprKind::Cast { .. } => self.check_cast(e),
            ExprKind::Try(_) => self.check_try(e),
            ExprKind::Await(inner) => {
                if !self.fcx.last().is_some_and(|c| c.is_async) {
                    self.diags.push(
                        Diagnostic::error(span, "`await` solo puede usarse dentro de una función `async`")
                            .with_label("`await` fuera de `async`")
                            .with_note("declara la función como `async fn ...`"),
                    );
                }
                let t = self.check_expr(inner, None);
                match self.shallow(&t) {
                    Ty::Named(n, a) if &*n == "Future" => a[0].clone(),
                    Ty::Error => Ty::Error,
                    other => {
                        let o = self.resolve(&other);
                        self.err_label(inner.span, format!("no se puede hacer `await` sobre `{o}`"), "se esperaba un `Future<T>`");
                        Ty::Error
                    }
                }
            }
            ExprKind::Return(val) => {
                let ret = self.fcx.last().map(|c| c.ret.clone()).unwrap_or(Ty::Nothing);
                match val {
                    Some(v) => {
                        if self.shallow(&ret) == Ty::Nothing {
                            let t = self.check_expr(v, None);
                            let _ = t;
                            self.diags.push(
                                Diagnostic::error(v.span, "esta función devuelve `nothing`; no puede devolver un valor")
                                    .with_label("valor inesperado")
                                    .with_note("`return nothing;` no es válido: usa `return;`"),
                            );
                        } else {
                            let t = self.check_expr(v, Some(&ret));
                            if !self.coerce(v, &t, &ret) {
                                self.mismatch(v.span, &ret, &t);
                            }
                        }
                    }
                    None => {
                        let r = self.shallow(&ret);
                        if let Ty::Var(_) = r {
                            self.unify(&r, &Ty::Nothing);
                        } else if r != Ty::Nothing && !r.is_error() {
                            self.err_label(span, format!("se esperaba devolver un valor de tipo `{}`", self.resolve(&ret)), "`return;` sin valor");
                        }
                    }
                }
                Ty::Never
            }
            ExprKind::Break => {
                if self.fcx.last().is_none_or(|c| c.loop_depth == 0) {
                    self.err_label(span, "`break` fuera de un bucle", "no hay bucle que interrumpir");
                } else if let Some(b) = self.cur_ctx().breaks.last_mut() {
                    *b = true;
                }
                Ty::Never
            }
            ExprKind::While { cond, body } => {
                let ct = self.check_expr(cond, Some(&Ty::Bool));
                if !self.coerce(cond, &ct, &Ty::Bool) {
                    self.mismatch(cond.span, &Ty::Bool, &ct);
                }
                let before = self.maybe_uninit.clone();
                let c = self.cur_ctx();
                c.loop_depth += 1;
                c.breaks.push(false);
                self.check_block(body, None, true);
                let c = self.cur_ctx();
                c.loop_depth -= 1;
                c.breaks.pop();
                self.maybe_uninit = before;
                Ty::Nothing
            }
            ExprKind::Loop { .. } => self.check_loop(e),
            ExprKind::Asm(_) => {
                if !self.in_unsafe() {
                    self.err_label(span, "el ensamblador en línea solo puede usarse dentro de `unsafe`", "operación peligrosa fuera de `unsafe`");
                }
                Ty::Nothing
            }
            ExprKind::SomeWrap(_) | ExprKind::Global { .. } | ExprKind::TypeStatic { .. } | ExprKind::VariantRef { .. } => Ty::Error,
        }
    }

    fn join_branch_types(&mut self, a: &mut Expr, ta: Ty, b: &mut Expr, tb: Ty, exp: Option<&Ty>, span: Span) -> Ty {
        let (sa, sb) = (self.shallow(&ta), self.shallow(&tb));
        if sa == Ty::Never { return tb; }
        if sb == Ty::Never { return ta; }
        let target = match exp {
            Some(t) if !matches!(self.shallow(t), Ty::Var(_)) => t.clone(),
            _ => ta.clone(),
        };
        let okb = self.coerce(b, &tb, &target);
        let oka = self.coerce(a, &ta, &target);
        if !oka || !okb {
            let (ra, rb) = (self.resolve(&ta), self.resolve(&tb));
            if !ra.is_error() && !rb.is_error() {
                self.err_label(span, format!("las ramas tienen tipos distintos: `{ra}` y `{rb}`"), "las dos ramas deben tener el mismo tipo");
            }
            return Ty::Error;
        }
        target
    }

    // ---------- identificadores ----------

    fn check_ident(&mut self, e: &mut Expr, exp: Option<&Ty>) -> Ty {
        let ExprKind::Ident(name) = &e.kind else { unreachable!() };
        let name = name.clone();
        let span = e.span;
        if let Some(id) = self.lookup_var(&name) {
            self.vars[id].used = true;
            if self.maybe_uninit.contains(&id) {
                self.diags.push(
                    Diagnostic::error(span, format!("la variable `{name}` se usa antes de ser inicializada"))
                        .with_label("variable no inicializada")
                        .with_note(format!("asígnale un valor antes: `{name} = ...;`")),
                );
            }
            return self.vars[id].ty.clone();
        }
        if name == "Self" || name == "self" {
            self.err(span, if name == "self" { "`self` solo puede usarse dentro de un método de `impl`" } else { "`Self` es un tipo, no un valor" });
            return Ty::Error;
        }
        if let Some(t) = self.check_global_value(e, &name, exp) {
            return t;
        }
        match name.as_str() {
            "None" => {
                e.kind = ExprKind::VariantRef { enum_key: "Option".into(), variant: "None".into() };
                let t = self.fresh();
                return Ty::option(t);
            }
            "Some" => {
                e.kind = ExprKind::VariantRef { enum_key: "Option".into(), variant: "Some".into() };
                let t = self.fresh();
                return Ty::Func(vec![t.clone()], Box::new(Ty::option(t)));
            }
            "Ok" => {
                e.kind = ExprKind::VariantRef { enum_key: "Result".into(), variant: "Ok".into() };
                let (t, er) = (self.fresh(), self.fresh());
                return Ty::Func(vec![t.clone()], Box::new(Ty::result(t, er)));
            }
            "Error" => {
                e.kind = ExprKind::VariantRef { enum_key: "Result".into(), variant: "Error".into() };
                let (t, er) = (self.fresh(), self.fresh());
                return Ty::Func(vec![er.clone()], Box::new(Ty::result(t, er)));
            }
            _ => {}
        }
        if self.lookup_type(self.cur_module, &name).is_some() {
            self.err_label(span, format!("`{name}` es un tipo, no un valor"), "se esperaba un valor");
            return Ty::Error;
        }
        let mut cands: Vec<String> = self.scopes.iter().flat_map(|s| s.keys().cloned()).collect();
        cands.extend(self.mods[self.cur_module].fns.keys().cloned());
        cands.extend(self.mods[self.cur_module].consts.keys().cloned());
        cands.extend(self.mods[0].fns.keys().cloned());
        let mut d = Diagnostic::error(span, format!("no se encuentra el valor `{name}` en este ámbito")).with_label("no declarado");
        if let Some(s) = self.suggest(&name, cands.into_iter()) {
            d = d.with_note(format!("¿quisiste decir `{s}`?"));
        }
        if matches!(name.as_str(), "continue" | "switch" | "class" | "interface" | "extends" | "mut" | "throw" | "catch" | "try" | "this") {
            d = d.with_note(format!("`{name}` no existe en N# v0.1 (ver «Reserved/future features» de la especificación)"));
        }
        self.diags.push(d);
        Ty::Error
    }

    /// Busca funciones, constantes y statics visibles por nombre.
    fn lookup_global(&self, name: &str) -> Option<(usize, String)> {
        let m = self.cur_module;
        if self.mods[m].fns.contains_key(name) || self.mods[m].consts.contains_key(name) {
            return Some((m, name.to_string()));
        }
        if let Some(ImportTarget::Item(im, item)) = self.mods[m].imports.get(name) {
            if self.mods[*im].fns.contains_key(item) || self.mods[*im].consts.contains_key(item) {
                return Some((*im, item.clone()));
            }
        }
        for w in &self.mods[m].wildcards {
            if self.mods[*w].fns.contains_key(name) || self.mods[*w].consts.contains_key(name) {
                return Some((*w, name.to_string()));
            }
        }
        if m != 0 && (self.mods[0].fns.contains_key(name) || self.mods[0].consts.contains_key(name)) {
            return Some((0, name.to_string()));
        }
        None
    }

    fn check_global_value(&mut self, e: &mut Expr, name: &str, exp: Option<&Ty>) -> Option<Ty> {
        let (m, item) = self.lookup_global(name)?;
        self.global_member_value(e, m, &item, exp)
    }

    /// Tipo de una función/constante de módulo usada como valor; reescribe el nodo a `Global`.
    pub(super) fn global_member_value(&mut self, e: &mut Expr, m: usize, item: &str, exp: Option<&Ty>) -> Option<Ty> {
        if let Some((ty, _)) = self.mods[m].consts.get(item).cloned() {
            let is_pub = self.pub_consts.contains(&(m, item.to_string()));
            self.check_visible(m, is_pub, item, e.span);
            e.kind = ExprKind::Global { module: m, name: item.to_string() };
            return Some(ty);
        }
        let ids = self.mods[m].fns.get(item)?.clone();
        let chosen = if ids.len() == 1 {
            ids[0]
        } else if let Some(Ty::Func(ps, _)) = exp.map(|t| self.shallow(t)) {
            match ids.iter().copied().find(|&i| {
                let s = &self.sigs[i];
                s.params.len() == ps.len() && s.generics.is_empty() && s.params.iter().zip(&ps).all(|(a, b)| a == b || matches!(self.shallow(b), Ty::Var(_)))
            }) {
                Some(i) => i,
                None => {
                    self.err(e.span, format!("ninguna sobrecarga de `{item}` encaja con el tipo de función esperado"));
                    return Some(Ty::Error);
                }
            }
        } else {
            self.diags.push(
                Diagnostic::error(e.span, format!("`{item}` está sobrecargada; no se puede usar como valor sin contexto"))
                    .with_note("llama a la función o indica el tipo de función esperado"),
            );
            return Some(Ty::Error);
        };
        let sig = self.sigs[chosen].clone();
        self.check_visible(m, sig.is_pub, item, e.span);
        e.kind = ExprKind::Global { module: m, name: item.to_string() };
        let map = self.instantiate(&sig.generics, e.span);
        let ret = if sig.is_async { Ty::named("Future", vec![sig.ret.subst(&map)]) } else { sig.ret.subst(&map) };
        Some(Ty::Func(sig.params.iter().map(|p| p.subst(&map)).collect(), Box::new(ret)))
    }

    pub(super) fn check_visible(&mut self, module: usize, is_pub: bool, name: &str, span: Span) {
        if module != self.cur_module && !is_pub && !self.mod_std[module] {
            self.err_label(span, format!("`{name}` es privado en el módulo `{}`", self.mod_names[module]), "no es `pub`");
        }
    }

    pub(super) fn instantiate(&mut self, generics: &[GenericInfo], span: Span) -> HashMap<Rc<str>, Ty> {
        let mut map = HashMap::new();
        for g in generics {
            map.insert(g.name.clone(), self.fresh());
        }
        for g in generics {
            for b in &g.bounds {
                let args = b.args.iter().map(|a| a.subst(&map)).collect();
                self.pending_bounds.push((map[&g.name].clone(), TraitRef { key: b.key.clone(), args }, span));
            }
        }
        map
    }

    // ---------- operadores ----------

    fn is_numeric_like(&self, t: &Ty) -> bool {
        match self.shallow(t) {
            Ty::Int | Ty::Float => true,
            Ty::Param(p) => self.fcx.iter().any(|c| c.generics.iter().any(|g| g.name == p && g.bounds.iter().any(|b| b.key == "Numeric"))),
            _ => false,
        }
    }

    fn is_comparable(&self, t: &Ty) -> bool {
        match self.shallow(t) {
            Ty::Int | Ty::Float | Ty::Char | Ty::Str => true,
            Ty::Param(_) | Ty::Named(..) => self.implements(t, &TraitRef { key: "Comparable".into(), args: vec![] }),
            _ => false,
        }
    }

    fn check_unary(&mut self, e: &mut Expr, exp: Option<&Ty>) -> Ty {
        let span = e.span;
        let ExprKind::Unary(op, inner) = &mut e.kind else { unreachable!() };
        let op = *op;
        match op {
            UnOp::Ref => {
                let hint = match exp.map(|t| self.shallow(t)) { Some(Ty::Ref(t)) => Some(*t), _ => None };
                let t = self.check_expr(inner, hint.as_ref());
                Ty::Ref(Box::new(t))
            }
            UnOp::Deref => {
                let t = self.check_expr(inner, None);
                match self.shallow(&t) {
                    Ty::Ref(x) => *x,
                    Ty::Ptr(x) => {
                        if !self.in_unsafe() {
                            self.diags.push(
                                Diagnostic::error(span, "desreferenciar un puntero crudo requiere `unsafe`")
                                    .with_label("operación peligrosa fuera de `unsafe`")
                                    .with_note("envuelve la operación en un bloque `unsafe { ... }`"),
                            );
                        }
                        *x
                    }
                    Ty::Named(n, a) if matches!(&*n, "Box" | "Rc") => a[0].clone(),
                    Ty::Error => Ty::Error,
                    other => {
                        let o = self.resolve(&other);
                        self.err_label(span, format!("no se puede desreferenciar `{o}`"), "se esperaba una referencia, puntero, `Box` o `Rc`");
                        Ty::Error
                    }
                }
            }
            UnOp::Neg => {
                let t = self.check_expr(inner, exp);
                if self.is_numeric_like(&t) || matches!(self.shallow(&t), Ty::Var(_) | Ty::Error) { t } else {
                    let r = self.resolve(&t);
                    self.err_label(span, format!("no se puede aplicar `-` a `{r}`"), "se esperaba un número");
                    Ty::Error
                }
            }
            UnOp::Not => {
                let t = self.check_expr(inner, Some(&Ty::Bool));
                if !self.coerce(inner, &t, &Ty::Bool) {
                    let r = self.resolve(&t);
                    self.err_label(span, format!("no se puede aplicar `!` a `{r}`"), "se esperaba `$bool`");
                }
                Ty::Bool
            }
            UnOp::BitNot => {
                let t = self.check_expr(inner, Some(&Ty::Int));
                if !self.coerce(inner, &t, &Ty::Int) {
                    let r = self.resolve(&t);
                    self.err_label(span, format!("no se puede aplicar `~` a `{r}`"), "se esperaba `$int`");
                }
                Ty::Int
            }
        }
    }

    fn check_binary(&mut self, e: &mut Expr) -> Ty {
        let span = e.span;
        let ExprKind::Binary(op, l, r) = &mut e.kind else { unreachable!() };
        let op = *op;
        use BinOp::*;
        match op {
            And | Or => {
                for x in [&mut **l, &mut **r] {
                    let t = self.check_expr(x, Some(&Ty::Bool));
                    if !self.coerce(x, &t, &Ty::Bool) {
                        self.mismatch(x.span, &Ty::Bool, &t);
                    }
                }
                Ty::Bool
            }
            Eq | Ne => {
                let lt = self.check_expr(l, None);
                let rt = self.check_expr(r, Some(&lt));
                let (sl, sr) = (self.shallow(&lt), self.shallow(&rt));
                let ok = match (&sl, &sr) {
                    (Ty::Null, t) | (t, Ty::Null) => t.is_option() || matches!(t, Ty::Ref(_) | Ty::Null | Ty::Var(_)),
                    _ => self.unify(&lt, &rt) || {
                        // T con T? : se envuelve el lado no opcional en `Some` para comparar valores del mismo tipo
                        if sl.is_option() { self.coerce(r, &rt, &lt) } else if sr.is_option() { self.coerce(l, &lt, &rt) } else { false }
                    },
                };
                if !ok {
                    let (a, b) = (self.resolve(&lt), self.resolve(&rt));
                    if !a.is_error() && !b.is_error() {
                        self.err_label(span, format!("no se pueden comparar `{a}` y `{b}`"), "tipos distintos");
                    }
                }
                Ty::Bool
            }
            RefEq => {
                let lt = self.check_expr(l, None);
                let rt = self.check_expr(r, Some(&lt));
                let a = self.resolve(&lt);
                let is_ref = |t: &Ty| matches!(t, Ty::Ref(_) | Ty::Ptr(_)) || matches!(t, Ty::Named(n, _) if matches!(&**n, "Rc" | "Box" | "Future"));
                if !a.is_error() && !is_ref(&a) {
                    self.diags.push(
                        Diagnostic::error(span, format!("`===` solo se puede usar con tipos referencia, no con `{a}`"))
                            .with_label("tipo por valor")
                            .with_note("usa `==` para comparar valores"),
                    );
                } else if !self.unify(&lt, &rt) {
                    let b = self.resolve(&rt);
                    self.err_label(span, format!("no se pueden comparar `{a}` y `{b}` con `===`"), "tipos distintos");
                }
                Ty::Bool
            }
            Lt | Gt | Le | Ge => {
                let lt = self.check_expr(l, None);
                let rt = self.check_expr(r, Some(&lt));
                if !self.unify(&lt, &rt) {
                    self.binop_mismatch(op, &lt, &rt, span);
                    return Ty::Bool;
                }
                if !self.is_comparable(&lt) && !matches!(self.shallow(&lt), Ty::Var(_) | Ty::Error) {
                    let a = self.resolve(&lt);
                    self.err_label(span, format!("el tipo `{a}` no admite comparación de orden"), "no implementa `Comparable`");
                }
                Ty::Bool
            }
            Add | Sub | Mul | Div | Rem | Pow => {
                let lt = self.check_expr(l, None);
                let rt = self.check_expr(r, Some(&lt));
                let (sl, sr) = (self.shallow(&lt), self.shallow(&rt));
                if sl.is_error() || sr.is_error() { return Ty::Error; }
                if matches!(op, Add | Sub) && matches!(sl, Ty::Ptr(_)) && self.can_coerce(&rt, &Ty::Int) {
                    if !self.in_unsafe() {
                        self.diags.push(
                            Diagnostic::error(span, "la aritmética de punteros requiere `unsafe`")
                                .with_label("operación peligrosa fuera de `unsafe`"),
                        );
                    }
                    return lt;
                }
                if op == Add && sl == Ty::Str && matches!(sr, Ty::Str | Ty::Char) {
                    return Ty::Str;
                }
                if !self.unify(&lt, &rt) {
                    self.binop_mismatch(op, &lt, &rt, span);
                    return Ty::Error;
                }
                if self.is_numeric_like(&lt) || matches!(self.shallow(&lt), Ty::Var(_)) {
                    return lt;
                }
                if op == Add && self.shallow(&lt) == Ty::Str { return Ty::Str; }
                let a = self.resolve(&lt);
                self.err_label(span, format!("el operador `{}` no se puede aplicar a `{a}`", op.symbol()), "operandos no válidos");
                Ty::Error
            }
            BitAnd | BitOr | BitXor | Shl | Shr => {
                for x in [&mut **l, &mut **r] {
                    let t = self.check_expr(x, Some(&Ty::Int));
                    if !self.coerce(x, &t, &Ty::Int) {
                        let rr = self.resolve(&t);
                        self.err_label(x.span, format!("el operador `{}` requiere `$int`, no `{rr}`", op.symbol()), "se esperaba `$int`");
                    }
                }
                Ty::Int
            }
        }
    }

    fn binop_mismatch(&mut self, op: BinOp, lt: &Ty, rt: &Ty, span: Span) {
        let (a, b) = (self.resolve(lt), self.resolve(rt));
        if a.is_error() || b.is_error() {
            return;
        }
        let mut d = Diagnostic::error(span, format!("el operador `{}` no admite `{a}` y `{b}`", op.symbol())).with_label("tipos incompatibles");
        if matches!((&a, &b), (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int)) {
            d = d.with_note("N# no convierte entre `$int` y `$float` implícitamente: usa `x as $float`");
        }
        self.diags.push(d);
    }

    fn check_place(&mut self, e: &mut Expr, initializing: bool) -> Ty {
        let span = e.span;
        match &mut e.kind {
            ExprKind::Ident(name) => {
                let name = name.clone();
                match self.lookup_var(&name) {
                    Some(id) => {
                        if initializing {
                            self.maybe_uninit.remove(&id);
                        } else {
                            self.vars[id].used = true;
                            if self.maybe_uninit.contains(&id) {
                                self.err_label(span, format!("la variable `{name}` se usa antes de ser inicializada"), "variable no inicializada");
                            }
                        }
                        self.vars[id].ty.clone()
                    }
                    None => {
                        if let Some((m, item)) = self.lookup_global(&name) {
                            if let Some((ty, is_static)) = self.mods[m].consts.get(&item).cloned() {
                                if !is_static {
                                    self.err_label(span, format!("`{name}` es una constante y no se puede modificar"), "`const` es inmutable");
                                }
                                e.kind = ExprKind::Global { module: m, name: item };
                                return ty;
                            }
                        }
                        self.check_expr(e, None)
                    }
                }
            }
            ExprKind::Field(..) | ExprKind::Index(..) => {
                let t = self.check_expr(e, None);
                if !self.rooted_in_place(e) {
                    self.err_label(span, "la parte izquierda de la asignación no es modificable", "es un valor temporal, no una variable, campo ni elemento");
                } else if let Some(c) = self.const_root(e) {
                    self.err_label(span, format!("`{c}` es una constante y no se puede modificar"), "`const` es inmutable");
                }
                if matches!(e.kind, ExprKind::Index(..)) && self.last_index_base.take().is_some_and(|b| self.shallow(&b) == Ty::Str) {
                    self.err_label(span, "los strings son inmutables: no se puede asignar a un carácter", "asignación a un elemento de `$string`");
                }
                t
            }
            ExprKind::Unary(UnOp::Deref, _) => self.check_expr(e, None),
            _ => {
                self.err_label(span, "la parte izquierda de la asignación no es modificable", "no es una variable, campo ni elemento");
                self.check_expr(e, None);
                Ty::Error
            }
        }
    }

    /// ¿La expresión nace de una variable, `static`, desreferencia o campo/índice de alguna de ellas?
    fn rooted_in_place(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Ident(_) | ExprKind::Global { .. } | ExprKind::Unary(UnOp::Deref, _) => true,
            ExprKind::Field(b, _) | ExprKind::Index(b, _) => self.rooted_in_place(b),
            _ => false,
        }
    }

    fn const_root(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Global { module, name } => match self.mods[*module].consts.get(name) {
                Some((_, false)) => Some(name.clone()),
                _ => None,
            },
            ExprKind::Field(b, _) | ExprKind::Index(b, _) => self.const_root(b),
            _ => None,
        }
    }

    fn check_assign(&mut self, e: &mut Expr) -> Ty {
        let span = e.span;
        let ExprKind::Assign(op, l, r) = &mut e.kind else { unreachable!() };
        let op = *op;
        let lt = self.check_place(l, op.is_none());
        match op {
            None => {
                let rt = self.check_expr(r, Some(&lt));
                if !self.coerce(r, &rt, &lt) {
                    // mensaje específico para el cambio de tipo de una variable
                    let (a, b) = (self.resolve(&lt), self.resolve(&rt));
                    if !a.is_error() && !b.is_error() {
                        self.diags.push(
                            Diagnostic::error(r.span, format!("tipos incompatibles: la variable es `{a}` y se le asigna `{b}`"))
                                .with_label(format!("se esperaba `{a}`"))
                                .with_note("el tipo de una variable no puede cambiar; usa una conversión explícita con `as`"),
                        );
                    }
                }
            }
            Some(bop) => {
                let rt = self.check_expr(r, Some(&lt));
                let ok = self.unify(&lt, &rt)
                    && (self.is_numeric_like(&lt) || matches!(self.shallow(&lt), Ty::Var(_)) || (bop == BinOp::Add && self.shallow(&lt) == Ty::Str));
                let str_char = bop == BinOp::Add && self.shallow(&lt) == Ty::Str && self.shallow(&rt) == Ty::Char;
                if !ok && !str_char {
                    self.binop_mismatch(bop, &lt, &rt, span);
                }
            }
        }
        Ty::Nothing
    }

    fn check_incdec(&mut self, e: &mut Expr) -> Ty {
        let span = e.span;
        let ExprKind::IncDec { target, .. } = &mut e.kind else { unreachable!() };
        let t = self.check_place(target, false);
        if !self.is_numeric_like(&t) && !matches!(self.shallow(&t), Ty::Var(_) | Ty::Error) {
            let r = self.resolve(&t);
            self.err_label(span, format!("`++`/`--` requieren un número, no `{r}`"), "operando no válido");
        }
        t
    }

    // ---------- cast / try ----------

    fn check_cast(&mut self, e: &mut Expr) -> Ty {
        let span = e.span;
        let ExprKind::Cast { expr, ty, lossy } = &mut e.kind else { unreachable!() };
        let lossy = *lossy;
        let sc = self.body_scope();
        let target = self.resolve_type(ty, &sc);
        let from = self.check_expr(expr, None);
        let f = self.resolve(&from);
        if f.is_error() || target.is_error() {
            return target;
        }
        let same = self.unify(&from, &target);
        if same {
            return target;
        }
        use Ty::*;
        let safe = matches!((&f, &target), (Int, Float) | (Char, Int) | (Bool, Int) | (Ptr(_), Ptr(_)));
        let lossy_only = matches!((&f, &target), (Float, Int) | (Int, Char) | (Int, Bool) | (Ptr(_), Int) | (Int, Ptr(_)));
        let needs_unsafe = matches!((&f, &target), (Int, Ptr(_)) | (Ptr(_), Int));
        if safe && !needs_unsafe {
            if lossy {
                // `as!` sobre una conversión segura es válido (redundante)
            }
        } else if lossy_only || needs_unsafe {
            if !lossy {
                self.diags.push(
                    Diagnostic::error(span, format!("la conversión de `{f}` a `{target}` puede perder información"))
                        .with_label("conversión con pérdida")
                        .with_note("usa `as!` para aceptar la pérdida de información"),
                );
            }
            if needs_unsafe && !self.in_unsafe() {
                self.err_label(span, "convertir entre enteros y punteros requiere `unsafe`", "operación peligrosa fuera de `unsafe`");
            }
        } else {
            self.err_label(span, format!("no se puede convertir `{f}` a `{target}`"), "conversión no soportada");
        }
        target
    }

    fn check_try(&mut self, e: &mut Expr) -> Ty {
        let span = e.span;
        let ExprKind::Try(inner) = &mut e.kind else { unreachable!() };
        let t = self.check_expr(inner, None);
        let ret = self.fcx.last().map(|c| c.ret.clone()).unwrap_or(Ty::Nothing);
        let is_main = self.fcx.last().is_some_and(|c| c.is_main);
        let sret = self.shallow(&ret);
        match self.shallow(&t) {
            Ty::Named(n, a) if &*n == "Result" => {
                match &sret {
                    Ty::Named(rn, ra) if &**rn == "Result" => {
                        if !self.unify(&a[1], &ra[1]) {
                            let (x, y) = (self.resolve(&a[1]), self.resolve(&ra[1]));
                            self.err_label(span, format!("el tipo de error `{x}` no coincide con el de la función (`{y}`)"), "error incompatible");
                        }
                    }
                    Ty::Var(_) => {
                        let ok = self.fresh();
                        self.unify(&sret, &Ty::result(ok, a[1].clone()));
                    }
                    Ty::Nothing | Ty::Int if is_main => {}
                    Ty::Error => {}
                    other => {
                        let o = self.resolve(other);
                        self.diags.push(
                            Diagnostic::error(span, format!("`?` requiere que la función devuelva un `Result`, pero devuelve `{o}`"))
                                .with_label("no se puede propagar el error")
                                .with_note("cambia el tipo de retorno a `Result<T, E>`"),
                        );
                    }
                }
                a[0].clone()
            }
            Ty::Named(n, a) if &*n == "Option" => {
                match &sret {
                    Ty::Named(rn, _) if &**rn == "Option" => {}
                    Ty::Var(_) => {
                        let v = self.fresh();
                        self.unify(&sret, &Ty::option(v));
                    }
                    Ty::Error => {}
                    other => {
                        let o = self.resolve(other);
                        self.diags.push(
                            Diagnostic::error(span, format!("`?` sobre un `Option` requiere que la función devuelva un `Option`, pero devuelve `{o}`"))
                                .with_label("no se puede propagar `None`"),
                        );
                    }
                }
                a[0].clone()
            }
            Ty::Error => Ty::Error,
            other => {
                let o = self.resolve(&other);
                self.err_label(span, format!("`?` solo se puede usar con `Result` u `Option`, no con `{o}`"), "operando no válido");
                Ty::Error
            }
        }
    }

    // ---------- if / match / loop / closure ----------

    fn check_if(&mut self, e: &mut Expr, exp: Option<&Ty>, discard: bool) -> Ty {
        let span = e.span;
        let ExprKind::If { cond, then, els } = &mut e.kind else { unreachable!() };
        let ct = self.check_expr(cond, Some(&Ty::Bool));
        if !self.coerce(cond, &ct, &Ty::Bool) {
            self.mismatch(cond.span, &Ty::Bool, &ct);
        }
        let start = self.maybe_uninit.clone();
        let Some(els) = els else {
            self.check_block(then, None, true);
            self.maybe_uninit = start;
            return Ty::Nothing;
        };
        let tt = self.check_block(then, exp, discard);
        let after_then = std::mem::replace(&mut self.maybe_uninit, start);
        self.discard = discard;
        let et = self.check_expr(els, exp.or(if discard { None } else { Some(&tt) }));
        let after_else = std::mem::take(&mut self.maybe_uninit);
        // asignación definida: una rama que no termina no aporta variables sin inicializar
        let then_div = self.shallow(&tt) == Ty::Never;
        let else_div = self.shallow(&et) == Ty::Never;
        self.maybe_uninit = match (then_div, else_div) {
            (true, false) => after_else,
            (false, true) => after_then,
            _ => after_then.union(&after_else).copied().collect(),
        };
        if discard {
            return if then_div && else_div { Ty::Never } else { Ty::Nothing };
        }
        // `then` ya está comprobado: hay que coercionar su cola si hace falta
        let (st, se) = (self.shallow(&tt), self.shallow(&et));
        if st == Ty::Never { return et; }
        if se == Ty::Never { return tt; }
        let target = match exp {
            Some(t) if !matches!(self.shallow(t), Ty::Var(_)) => t.clone(),
            _ => tt.clone(),
        };
        let mut ok = true;
        if let Some(tail) = &mut then.tail {
            ok &= self.coerce(tail, &tt, &target);
        } else {
            ok &= self.can_unify(&tt, &target);
        }
        if let ExprKind::Block(b) = &mut els.kind {
            match &mut b.tail {
                Some(tail) => ok &= self.coerce(tail, &et, &target),
                None => ok &= self.can_unify(&et, &target),
            }
        } else {
            ok &= self.can_unify(&et, &target);
        }
        if !ok {
            let (a, b) = (self.resolve(&tt), self.resolve(&et));
            if !a.is_error() && !b.is_error() {
                self.err_label(span, format!("las ramas del `if` tienen tipos distintos: `{a}` y `{b}`"), "las dos ramas deben tener el mismo tipo");
            }
            return Ty::Error;
        }
        target
    }

    fn check_match(&mut self, e: &mut Expr, exp: Option<&Ty>, discard: bool) -> Ty {
        let span = e.span;
        let ExprKind::Match { scrut, arms } = &mut e.kind else { unreachable!() };
        let st = self.check_expr(scrut, None);
        let start = self.maybe_uninit.clone();
        let mut result: Option<Ty> = if discard { Some(Ty::Nothing) } else { exp.filter(|t| !matches!(self.shallow(t), Ty::Var(_))).cloned() };
        let mut merged: Option<HashSet<usize>> = None;
        let mut all_never = !arms.is_empty();
        for arm in arms.iter_mut() {
            self.maybe_uninit = start.clone();
            self.push_scope();
            self.check_pattern(&mut arm.pattern, &st);
            if let Some(g) = &mut arm.guard {
                let gt = self.check_expr(g, Some(&Ty::Bool));
                if !self.coerce(g, &gt, &Ty::Bool) {
                    self.mismatch(g.span, &Ty::Bool, &gt);
                }
            }
            self.discard = discard;
            let bt = self.check_expr(&mut arm.body, result.as_ref().or(exp));
            self.pop_scope();
            let diverges = self.shallow(&bt) == Ty::Never;
            if !diverges {
                all_never = false;
                merged = Some(match merged {
                    None => self.maybe_uninit.clone(),
                    Some(m) => m.union(&self.maybe_uninit).copied().collect(),
                });
            }
            if discard || diverges {
                continue;
            }
            match &result {
                None => result = Some(bt),
                Some(r) => {
                    let r = r.clone();
                    if !self.coerce(&mut arm.body, &bt, &r) {
                        let (a, b) = (self.resolve(&r), self.resolve(&bt));
                        if !a.is_error() && !b.is_error() {
                            self.err_label(arm.body.span, format!("los brazos del `match` tienen tipos distintos: `{a}` y `{b}`"), format!("se esperaba `{a}`"));
                        }
                    }
                }
            }
        }
        self.maybe_uninit = merged.unwrap_or(start);
        self.check_match_coverage(arms, &st, span);
        if all_never {
            return Ty::Never;
        }
        if discard { Ty::Nothing } else { result.unwrap_or(Ty::Nothing) }
    }

    fn check_loop(&mut self, e: &mut Expr) -> Ty {
        let ExprKind::Loop { kind, body } = &mut e.kind else { unreachable!() };
        self.push_scope();
        let before = self.maybe_uninit.clone();
        let mut infinite = false;
        match kind {
            LoopKind::C { init, cond, update } => {
                if let Some(s) = init {
                    self.check_stmt(s);
                }
                match cond {
                    Some(c) => {
                        let ct = self.check_expr(c, Some(&Ty::Bool));
                        if !self.coerce(c, &ct, &Ty::Bool) {
                            self.mismatch(c.span, &Ty::Bool, &ct);
                        }
                    }
                    None => infinite = true,
                }
                let ctx = self.cur_ctx();
                ctx.loop_depth += 1;
                ctx.breaks.push(false);
                self.check_block(body, None, true);
                if let Some(u) = update {
                    self.check_expr_stmt(u);
                }
            }
            LoopKind::Each { var, ty, iter } => {
                let it = self.check_expr(iter, None);
                let item = self.iter_item_type(&it, iter.span);
                let vty = match ty {
                    Some(te) => {
                        let sc = self.body_scope();
                        let dt = self.resolve_type(te, &sc);
                        if !self.unify(&dt, &item) {
                            self.mismatch(te.span, &item, &dt);
                        }
                        dt
                    }
                    None => item,
                };
                self.push_scope();
                self.declare(&var.name, vty, var.span, true, true);
                let ctx = self.cur_ctx();
                ctx.loop_depth += 1;
                ctx.breaks.push(false);
                self.check_block(body, None, true);
                self.pop_scope();
            }
        }
        let ctx = self.cur_ctx();
        ctx.loop_depth -= 1;
        let had_break = ctx.breaks.pop().unwrap_or(false);
        self.maybe_uninit = before;
        self.pop_scope();
        if infinite && !had_break { Ty::Never } else { Ty::Nothing }
    }

    fn iter_item_type(&mut self, ty: &Ty, span: Span) -> Ty {
        if let Some(t) = self.iter_item_type_static(ty) {
            return t;
        }
        let r = self.shallow(ty);
        match &r {
            Ty::Error => return Ty::Error,
            Ty::Var(_) => {
                let item = self.fresh();
                self.unify(&r, &Ty::list(item.clone()));
                return item;
            }
            Ty::Param(p) => {
                for c in self.fcx.iter().rev() {
                    for g in &c.generics {
                        if g.name == *p {
                            if let Some(b) = g.bounds.iter().find(|b| b.key == "Iterable") {
                                if let Some(a) = b.args.first() {
                                    return a.clone();
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(head) = r.head_key() {
                    if let Some(idxs) = self.impls_by_head.get(&head).cloned() {
                        for i in idxs {
                            let imp = self.impls[i].clone();
                            if imp.trait_ref.as_ref().is_some_and(|t| t.key == "Iterable") {
                                let map = self.instantiate(&imp.generics, span);
                                if self.unify(&imp.target.subst(&map), &r) {
                                    return imp.trait_ref.unwrap().args[0].subst(&map);
                                }
                            }
                        }
                    }
                }
            }
        }
        let o = self.resolve(&r);
        self.diags.push(
            Diagnostic::error(span, format!("el tipo `{o}` no se puede recorrer con `loop (x : ...)`"))
                .with_label("no implementa `Iterable`")
                .with_note("se pueden recorrer arrays, `List`, `Set`, `Map`, strings y tipos que implementen `Iterable<T>`"),
        );
        Ty::Error
    }

    fn check_closure(&mut self, e: &mut Expr, exp: Option<&Ty>) -> Ty {
        let ExprKind::Closure { params, ret, body } = &mut e.kind else { unreachable!() };
        let expected = match exp.map(|t| self.shallow(t)) {
            Some(Ty::Func(p, r)) if p.len() == params.len() => Some((p, *r)),
            _ => None,
        };
        let mut ptys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let t = match &p.ty {
                Some(te) => {
                    let sc = self.body_scope();
                    let t = self.resolve_type(te, &sc);
                    if let Some((ep, _)) = &expected {
                        self.unify(&t, &ep[i]);
                    }
                    t
                }
                None => match &expected {
                    Some((ep, _)) => ep[i].clone(),
                    None => self.fresh(),
                },
            };
            ptys.push(t);
        }
        let rty = match ret {
            Some(te) => {
                let sc = self.body_scope();
                let t = self.resolve_type(te, &sc);
                if let Some((_, er)) = &expected {
                    self.unify(&t, er);
                }
                t
            }
            None => match &expected {
                Some((_, er)) => er.clone(),
                None => self.fresh(),
            },
        };
        let outer = self.fcx.last().map(|c| (c.generics.clone(), c.self_ty.clone(), c.module, c.unsafe_depth, c.is_main)).unwrap();
        self.fcx.push(FnCtx {
            ret: rty.clone(),
            is_async: false,
            unsafe_depth: outer.3,
            loop_depth: 0,
            breaks: vec![],
            generics: vec![],
            self_ty: None,
            module: outer.2,
            is_main: false,
            scope_base: self.scopes.len(),
        });
        self.push_scope();
        for (p, t) in params.iter().zip(&ptys) {
            self.declare(&p.name.name, t.clone(), p.name.span, true, false);
        }
        let saved_uninit = self.maybe_uninit.clone();
        let rshallow = self.shallow(&rty);
        let discard = rshallow == Ty::Nothing;
        let bt = match &mut body.kind {
            ExprKind::Block(b) => {
                let t = self.check_block(b, Some(&rty), discard);
                if t != Ty::Never && !discard {
                    match &mut b.tail {
                        Some(tail) => {
                            if !self.coerce(tail, &t, &rty) {
                                self.mismatch(tail.span, &rty, &t);
                            }
                        }
                        None => {
                            if matches!(rshallow, Ty::Var(_)) {
                                self.unify(&rshallow, &Ty::Nothing);
                            } else if !rshallow.is_error() {
                                let r = self.resolve(&rty);
                                self.err_label(b.span, format!("la función anónima debe devolver `{r}` en todos los caminos"), "falta un `return`");
                            }
                        }
                    }
                }
                t
            }
            _ => {
                let t = self.check_expr(body, Some(&rty));
                if !self.coerce(body, &t, &rty) {
                    self.mismatch(body.span, &rty, &t);
                }
                t
            }
        };
        let _ = bt;
        self.maybe_uninit = saved_uninit;
        self.pop_scope();
        self.fcx.pop();
        let _ = outer.0;
        Ty::Func(ptys, Box::new(rty))
    }

    // ---------- índices, campos, struct literal ----------

    fn check_index(&mut self, e: &mut Expr) -> Ty {
        let span = e.span;
        let ExprKind::Index(base, idx) = &mut e.kind else { unreachable!() };
        let bt = self.check_expr(base, None);
        let mut r = self.shallow(&bt);
        while let Ty::Ref(i) = r.clone() { r = self.shallow(&i); }
        self.last_index_base = Some(r.clone());
        match r {
            Ty::Tuple(tys) => {
                let it = self.check_expr(idx, Some(&Ty::Int));
                let _ = it;
                match &idx.kind {
                    ExprKind::Int(n) if *n >= 0 && (*n as usize) < tys.len() => tys[*n as usize].clone(),
                    ExprKind::Int(n) => {
                        self.err_label(idx.span, format!("índice {n} fuera de rango para una tupla de {} elementos", tys.len()), "fuera de rango");
                        Ty::Error
                    }
                    _ => {
                        self.err_label(idx.span, "los índices de una tupla deben ser literales enteros", "se esperaba un literal");
                        Ty::Error
                    }
                }
            }
            Ty::Named(n, a) if &*n == "Map" => {
                let t = self.check_expr(idx, Some(&a[0]));
                if !self.coerce(idx, &t, &a[0]) { self.mismatch(idx.span, &a[0], &t); }
                a[1].clone()
            }
            other => {
                let t = self.check_expr(idx, Some(&Ty::Int));
                if !self.coerce(idx, &t, &Ty::Int) { self.mismatch(idx.span, &Ty::Int, &t); }
                match other {
                    Ty::Array(t, _) => *t,
                    Ty::Named(n, a) if &*n == "List" => a[0].clone(),
                    Ty::Str => Ty::Char,
                    Ty::Ptr(t) => {
                        if !self.in_unsafe() {
                            self.err_label(span, "indexar un puntero crudo requiere `unsafe`", "operación peligrosa fuera de `unsafe`");
                        }
                        *t
                    }
                    Ty::Error => Ty::Error,
                    o => {
                        let o = self.resolve(&o);
                        self.err_label(span, format!("no se puede indexar `{o}`"), "no admite `[...]`");
                        Ty::Error
                    }
                }
            }
        }
    }

    pub(super) fn deref_for_member(&mut self, t: &Ty) -> Ty {
        let mut r = self.shallow(t);
        for _ in 0..4 {
            match r.clone() {
                Ty::Ref(i) => r = self.shallow(&i),
                Ty::Named(n, a) if matches!(&*n, "Box" | "Rc") => r = self.shallow(&a[0]),
                _ => break,
            }
        }
        r
    }

    pub(super) fn field_type(&mut self, base_ty: &Ty, name: &str, span: Span) -> Option<Ty> {
        let r = self.deref_for_member(base_ty);
        let Ty::Named(k, args) = &r else { return None };
        let def = self.types.get(&**k)?.clone();
        let DefKind::Struct(fields) = &def.kind else { return None };
        let f = fields.iter().find(|f| f.name == name)?;
        if !f.is_pub && def.module != self.cur_module && !self.mod_std[def.module] {
            self.err_label(span, format!("el campo `{name}` de `{}` es privado", def.name), "campo privado (añade `pub` en su declaración)");
        }
        let map: HashMap<Rc<str>, Ty> = def.generics.iter().map(|g| g.name.clone()).zip(args.iter().cloned()).collect();
        Some(f.ty.subst(&map))
    }

    fn check_field(&mut self, e: &mut Expr) -> Ty {
        let span = e.span;
        // ¿ruta de módulo / tipo?
        if let Some(t) = self.check_path_member(e) {
            return t;
        }
        let ExprKind::Field(base, name) = &mut e.kind else { unreachable!() };
        let name = name.clone();
        let bt = self.check_expr(base, None);
        let r = self.deref_for_member(&bt);
        if r.is_error() { return Ty::Error; }
        if let Some(t) = self.field_type(&bt, &name.name, name.span) {
            return t;
        }
        if matches!(r, Ty::Var(_)) {
            self.err_label(name.span, format!("no se conoce el tipo de la expresión para acceder a `{}`", name.name), "tipo aún no inferido");
            return Ty::Error;
        }
        // ¿es un método usado sin llamarlo?
        if !self.method_cands(&r, &name.name, span).is_empty() {
            self.diags.push(
                Diagnostic::error(name.span, format!("`{}` es un método; falta llamarlo con `()`", name.name))
                    .with_label("método sin llamar"),
            );
            return Ty::Error;
        }
        let rs = self.resolve(&r);
        let mut cands: Vec<String> = vec![];
        if let Ty::Named(k, _) = &rs {
            if let Some(TypeDef { kind: DefKind::Struct(fs), .. }) = self.types.get(&**k) {
                cands.extend(fs.iter().map(|f| f.name.clone()));
            }
        }
        let mut d = Diagnostic::error(name.span, format!("el tipo `{rs}` no tiene el campo `{}`", name.name)).with_label("campo inexistente");
        if let Some(s) = self.suggest(&name.name, cands.into_iter()) {
            d = d.with_note(format!("¿quisiste decir `{s}`?"));
        }
        self.diags.push(d);
        Ty::Error
    }

    fn check_struct_lit(&mut self, e: &mut Expr, exp: Option<&Ty>) -> Ty {
        let span = e.span;
        let ExprKind::StructLit { path, fields, key } = &mut e.kind else { unreachable!() };
        let last = path.last().unwrap().clone();
        let k = if path.len() == 1 {
            if last.name == "Self" {
                self.body_scope().self_ty.and_then(|t| match self.shallow(&t) { Ty::Named(k, _) => Some(k.to_string()), _ => None })
            } else {
                self.lookup_type(self.cur_module, &last.name)
            }
        } else {
            let mname = path[..path.len() - 1].iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
            self.find_module(self.cur_module, &mname).and_then(|m| self.mods[m].types.get(&last.name).cloned())
        };
        let Some(k) = k else {
            let cands: Vec<String> = self.mods[self.cur_module].types.keys().cloned().collect();
            let mut d = Diagnostic::error(last.span, format!("no se encuentra el tipo `{}`", last.name)).with_label("tipo desconocido");
            if let Some(s) = self.suggest(&last.name, cands.into_iter()) {
                d = d.with_note(format!("¿quisiste decir `{s}`?"));
            }
            self.diags.push(d);
            for (_, v) in fields.iter_mut() { self.check_expr(v, None); }
            return Ty::Error;
        };
        let def = self.types[&k].clone();
        let DefKind::Struct(finfos) = &def.kind else {
            self.err_label(last.span, format!("`{}` no es un struct", last.name), "no se puede construir con `{ ... }`");
            for (_, v) in fields.iter_mut() { self.check_expr(v, None); }
            return Ty::Error;
        };
        if def.opaque {
            self.err_label(span, format!("`{}` es un tipo del runtime y no se construye con un literal", def.name), format!("usa `{}.new(...)`", def.name));
            return Ty::Error;
        }
        if def.module != self.cur_module && !def.is_pub && !self.mod_std[def.module] {
            self.err_label(last.span, format!("el tipo `{}` es privado en su módulo", def.name), "no es `pub`");
        }
        *key = k.clone();
        let targs: Vec<Ty> = def.generics.iter().map(|_| self.fresh()).collect();
        let ty = Ty::Named(Rc::from(k.as_str()), targs.clone());
        if let Some(x) = exp {
            if matches!(self.shallow(x), Ty::Named(..)) {
                self.unify(&ty, x);
            }
        }
        let map: HashMap<Rc<str>, Ty> = def.generics.iter().map(|g| g.name.clone()).zip(targs).collect();
        let mut seen = HashSet::new();
        for (fname, val) in fields.iter_mut() {
            if !seen.insert(fname.name.clone()) {
                self.err(fname.span, format!("el campo `{}` está duplicado", fname.name));
            }
            match finfos.iter().find(|f| f.name == fname.name) {
                Some(f) => {
                    if !f.is_pub && def.module != self.cur_module && !self.mod_std[def.module] {
                        self.err_label(fname.span, format!("el campo `{}` es privado", f.name), "campo privado");
                    }
                    let fty = f.ty.subst(&map);
                    self.check_coerce(val, &fty);
                }
                None => {
                    let mut d = Diagnostic::error(fname.span, format!("`{}` no tiene el campo `{}`", def.name, fname.name)).with_label("campo inexistente");
                    if let Some(s) = self.suggest(&fname.name, finfos.iter().map(|f| f.name.clone())) {
                        d = d.with_note(format!("¿quisiste decir `{s}`?"));
                    }
                    self.diags.push(d);
                    self.check_expr(val, None);
                }
            }
        }
        let missing: Vec<&str> = finfos.iter().filter(|f| !seen.contains(&f.name)).map(|f| f.name.as_str()).collect();
        if !missing.is_empty() {
            self.err_label(span, format!("faltan campos en `{}`: {}", def.name, missing.join(", ")), "inicialización incompleta");
        }
        ty
    }
}
