//! Variables de inferencia, unificación (con deshacer), coerciones y comprobación de traits.

use super::*;

impl Checker {
    pub(super) fn fresh(&mut self) -> Ty {
        self.infer.push(None);
        Ty::Var((self.infer.len() - 1) as u32)
    }

    pub(super) fn shallow(&self, t: &Ty) -> Ty {
        let mut cur = t.clone();
        while let Ty::Var(id) = cur {
            match &self.infer[id as usize] {
                Some(next) => cur = next.clone(),
                None => break,
            }
        }
        cur
    }

    /// Resuelve las variables de inferencia en profundidad.
    pub(super) fn resolve(&self, t: &Ty) -> Ty {
        match self.shallow(t) {
            Ty::Named(n, a) => Ty::Named(n, a.iter().map(|x| self.resolve(x)).collect()),
            Ty::Array(t, n) => Ty::Array(Box::new(self.resolve(&t)), n),
            Ty::Tuple(v) => Ty::Tuple(v.iter().map(|x| self.resolve(x)).collect()),
            Ty::Ref(t) => Ty::Ref(Box::new(self.resolve(&t))),
            Ty::Ptr(t) => Ty::Ptr(Box::new(self.resolve(&t))),
            Ty::Func(p, r) => Ty::Func(p.iter().map(|x| self.resolve(x)).collect(), Box::new(self.resolve(&r))),
            Ty::Union(v) => Ty::Union(v.iter().map(|x| self.resolve(x)).collect()),
            other => other,
        }
    }

    fn occurs(&self, id: u32, t: &Ty) -> bool {
        match self.shallow(t) {
            Ty::Var(v) => v == id,
            Ty::Named(_, a) | Ty::Tuple(a) | Ty::Union(a) => a.iter().any(|x| self.occurs(id, x)),
            Ty::Array(t, _) | Ty::Ref(t) | Ty::Ptr(t) => self.occurs(id, &t),
            Ty::Func(p, r) => p.iter().any(|x| self.occurs(id, x)) || self.occurs(id, &r),
            _ => false,
        }
    }

    pub(super) fn snapshot(&self) -> usize {
        self.trail.len()
    }

    pub(super) fn rollback(&mut self, snap: usize) {
        while self.trail.len() > snap {
            let id = self.trail.pop().unwrap();
            self.infer[id as usize] = None;
        }
    }

    /// Unificación atómica: si falla no deja ligaduras parciales.
    pub(super) fn unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let snap = self.snapshot();
        if self.unify_inner(a, b) {
            true
        } else {
            self.rollback(snap);
            false
        }
    }

    /// ¿Unificarían? No deja ligaduras.
    pub(super) fn can_unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let snap = self.snapshot();
        let ok = self.unify_inner(a, b);
        self.rollback(snap);
        ok
    }

    fn unify_inner(&mut self, a: &Ty, b: &Ty) -> bool {
        let a = self.shallow(a);
        let b = self.shallow(b);
        match (&a, &b) {
            (Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Never, _) | (_, Ty::Never) => true,
            (Ty::Var(x), Ty::Var(y)) if x == y => true,
            (Ty::Var(x), t) | (t, Ty::Var(x)) => {
                if self.occurs(*x, t) {
                    return false;
                }
                self.infer[*x as usize] = Some(t.clone());
                self.trail.push(*x);
                true
            }
            (Ty::Named(n1, a1), Ty::Named(n2, a2)) => n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| self.unify_inner(x, y)),
            (Ty::Array(t1, n1), Ty::Array(t2, n2)) => n1 == n2 && self.unify_inner(t1, t2),
            (Ty::Tuple(v1), Ty::Tuple(v2)) => v1.len() == v2.len() && v1.iter().zip(v2).all(|(x, y)| self.unify_inner(x, y)),
            (Ty::Ref(x), Ty::Ref(y)) | (Ty::Ptr(x), Ty::Ptr(y)) => self.unify_inner(x, y),
            (Ty::Func(p1, r1), Ty::Func(p2, r2)) => {
                p1.len() == p2.len() && p1.iter().zip(p2).all(|(x, y)| self.unify_inner(x, y)) && self.unify_inner(r1, r2)
            }
            (Ty::Union(v1), Ty::Union(v2)) => {
                v1.len() == v2.len() && v1.iter().all(|x| v2.iter().any(|y| self.can_unify(x, y)))
            }
            (Ty::Param(x), Ty::Param(y)) => x == y,
            _ => a == b,
        }
    }

    /// Coerción de un valor de tipo `found` hacia `expected` (puede envolver en `Some`).
    /// Devuelve false si no es posible; no emite diagnósticos.
    pub(super) fn coerce(&mut self, e: &mut Expr, found: &Ty, expected: &Ty) -> bool {
        let found = self.shallow(found);
        let expected = self.shallow(expected);
        match (&found, &expected) {
            (Ty::Error, _) | (_, Ty::Error) | (Ty::Never, _) => return true,
            (Ty::Null, Ty::Var(_)) => return false,
            (Ty::Null, _) => {
                return expected.is_option() || matches!(expected, Ty::Ref(_));
            }
            _ => {}
        }
        if let Ty::Union(members) = &expected {
            if let Ty::Union(fm) = &found {
                return fm.iter().all(|f| members.iter().any(|m| self.can_unify(f, m)));
            }
            for m in members.clone() {
                if self.unify(&found, &m) {
                    return true;
                }
            }
            return false;
        }
        if self.unify(&found, &expected) {
            return true;
        }
        // T -> Option<T>
        if let Ty::Named(n, args) = &expected {
            if &**n == "Option" && !found.is_option() {
                let inner = args[0].clone();
                if self.coerce(e, &found, &inner) {
                    let span = e.span;
                    let old = std::mem::replace(&mut e.kind, ExprKind::Null);
                    e.kind = ExprKind::SomeWrap(Box::new(Expr { kind: old, span }));
                    return true;
                }
            }
        }
        false
    }

    pub(super) fn can_coerce(&mut self, found: &Ty, expected: &Ty) -> bool {
        let mut dummy = Expr { kind: ExprKind::Null, span: Span::default() };
        let snap = self.snapshot();
        let ok = self.coerce(&mut dummy, found, expected);
        self.rollback(snap);
        ok
    }

    pub(super) fn mismatch(&mut self, span: Span, expected: &Ty, found: &Ty) {
        let (ex, fo) = (self.resolve(expected), self.resolve(found));
        if ex.is_error() || fo.is_error() {
            return;
        }
        let mut d = Diagnostic::error(span, format!("tipos incompatibles: se esperaba `{ex}`, pero se encontró `{fo}`"))
            .with_label(format!("se esperaba `{ex}`"));
        match (&ex, &fo) {
            (Ty::Float, Ty::Int) => d = d.with_note("no hay conversión implícita: usa `as $float`"),
            (Ty::Int, Ty::Float) => d = d.with_note("no hay conversión implícita: usa `as! $int` (puede perder precisión)"),
            (_, Ty::Null) => d = d.with_note("`null` solo es válido en tipos opcionales (`T?`) y referencias"),
            (_, Ty::Nothing) => d = d.with_note("la expresión no produce ningún valor (`nothing`); un `if` usado como expresión necesita `else`"),
            (Ty::Str, Ty::Int | Ty::Float | Ty::Bool | Ty::Char) => d = d.with_note("para convertir a texto usa la interpolación `\"{valor}\"` o `.to_string()`"),
            _ => {}
        }
        self.diags.push(d);
    }

    /// ¿El tipo implementa el trait? Las variables sin resolver se aceptan (se reevalúan después).
    pub(super) fn implements(&self, ty: &Ty, tr: &TraitRef) -> bool {
        let ty = self.shallow(ty);
        match &ty {
            Ty::Var(_) | Ty::Error | Ty::Never => true,
            Ty::Param(p) => {
                self.fcx.iter().rev().any(|c| c.generics.iter().any(|g| g.name == *p && g.bounds.iter().any(|b| b.key == tr.key)))
            }
            _ => {
                if tr.key == "Iterable" {
                    if self.iter_item_type_static(&ty).is_some() {
                        return true;
                    }
                }
                match ty.head_key() {
                    Some(h) => self.trait_impls.contains(&(tr.key.clone(), h)),
                    None => false,
                }
            }
        }
    }

    pub(super) fn iter_item_type_static(&self, ty: &Ty) -> Option<Ty> {
        match self.shallow(ty) {
            Ty::Array(t, _) => Some(*t),
            Ty::Str => Some(Ty::Char),
            Ty::Named(n, a) => match &*n {
                "List" | "Set" => Some(a[0].clone()),
                "Map" => Some(Ty::Tuple(vec![a[0].clone(), a[1].clone()])),
                _ => None,
            },
            Ty::Ref(t) => self.iter_item_type_static(&t),
            _ => None,
        }
    }

    pub(super) fn check_pending_bounds(&mut self) {
        let pend = std::mem::take(&mut self.pending_bounds);
        for (ty, tr, span) in pend {
            let r = self.resolve(&ty);
            if r.contains_var() || r.is_error() {
                continue;
            }
            if !self.implements(&r, &tr) {
                let tname = tr.key.rsplit("::").next().unwrap_or(&tr.key).to_string();
                self.err_label(span, format!("el tipo `{r}` no implementa el trait `{tname}`"), format!("`{r}` no cumple la cota `{tname}`"));
            }
        }
        let pend = std::mem::take(&mut self.pending_infer);
        for (ty, span, name) in pend {
            let r = self.resolve(&ty);
            if r.contains_var() {
                self.diags.push(
                    Diagnostic::error(span, format!("no se puede inferir el tipo de `{name}`"))
                        .with_label("tipo no determinado")
                        .with_note("añade una anotación de tipo, p. ej. `let x List<$int> = ...`"),
                );
            }
        }
    }
}
