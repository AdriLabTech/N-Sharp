//! Llamadas a funciones y métodos, rutas `modulo.item` / `Tipo.item`.

use super::*;

#[derive(Clone)]
pub(super) struct Cand {
    pub sig: usize,
    pub map: HashMap<Rc<str>, Ty>,
}

enum Head {
    Value,
    Module(usize),
    Type(String),
}

enum Callee {
    Cands { cands: Vec<Cand>, name: String, static_call: bool },
    Value(Ty),
    Error,
}

impl Checker {
    fn head_of(&mut self, e: &Expr) -> Head {
        match &e.kind {
            ExprKind::Ident(n) => {
                if self.lookup_var(n).is_some() {
                    return Head::Value;
                }
                if n == "Self" {
                    return match self.body_scope().self_ty.and_then(|t| t.head_key()) {
                        Some(k) => Head::Type(k),
                        None => Head::Value,
                    };
                }
                if let Some(m) = self.find_module(self.cur_module, n) {
                    return Head::Module(m);
                }
                if let Some(k) = self.lookup_type(self.cur_module, n) {
                    return Head::Type(k);
                }
                // los módulos de la biblioteca estándar se pueden usar sin `import`
                if let Some(&m) = self.mod_index.get(n.as_str()) {
                    if self.mod_std[m] && m != 0 {
                        return Head::Module(m);
                    }
                }
                Head::Value
            }
            ExprKind::Field(b, n) => match self.head_of(b) {
                Head::Module(m) => match self.mods[m].types.get(&n.name) {
                    Some(k) => Head::Type(k.clone()),
                    None => Head::Value,
                },
                _ => Head::Value,
            },
            _ => Head::Value,
        }
    }

    fn enum_variants(&self, key: &str) -> Option<(Vec<GenericInfo>, Vec<VariantInfo>)> {
        match self.types.get(key) {
            Some(TypeDef { generics, kind: DefKind::Enum(v), .. }) => Some((generics.clone(), v.clone())),
            _ => None,
        }
    }

    fn variant_type(&mut self, key: &str, vname: &str) -> Option<Ty> {
        let (generics, variants) = self.enum_variants(key)?;
        let v = variants.iter().find(|v| v.name == vname)?;
        let targs: Vec<Ty> = generics.iter().map(|_| self.fresh()).collect();
        let map: HashMap<Rc<str>, Ty> = generics.iter().map(|g| g.name.clone()).zip(targs.iter().cloned()).collect();
        let ety = Ty::Named(Rc::from(key), targs);
        Some(if v.fields.is_empty() { ety } else { Ty::Func(v.fields.iter().map(|t| t.subst(&map)).collect(), Box::new(ety)) })
    }

    /// `modulo.item` o `Tipo.item` usado como valor (no como callee). Reescribe el nodo si resuelve.
    pub(super) fn check_path_member(&mut self, e: &mut Expr) -> Option<Ty> {
        let ExprKind::Field(base, name) = &e.kind else { return None };
        let name = name.clone();
        let span = e.span;
        let head = self.head_of(base);
        match head {
            Head::Value => None,
            Head::Module(m) => {
                if let Some(t) = self.global_member_value(e, m, &name.name, None) {
                    return Some(t);
                }
                if self.mods[m].types.contains_key(&name.name) {
                    self.err_label(name.span, format!("`{}` es un tipo, no un valor", name.name), "se esperaba un valor");
                } else {
                    let mut d = Diagnostic::error(name.span, format!("el módulo `{}` no tiene el elemento `{}`", self.mod_names[m], name.name)).with_label("no existe");
                    let cands: Vec<String> = self.mods[m].fns.keys().chain(self.mods[m].consts.keys()).cloned().collect();
                    if let Some(s) = self.suggest(&name.name, cands.into_iter()) {
                        d = d.with_note(format!("¿quisiste decir `{s}`?"));
                    }
                    self.diags.push(d);
                }
                Some(Ty::Error)
            }
            Head::Type(key) => {
                if let Some(t) = self.variant_type(&key, &name.name) {
                    e.kind = ExprKind::VariantRef { enum_key: key, variant: name.name };
                    return Some(t);
                }
                let cands = self.static_cands(&key, &name.name, span);
                if cands.len() == 1 {
                    let c = cands[0].clone();
                    let sig = self.sigs[c.sig].clone();
                    e.kind = ExprKind::TypeStatic { key, name: name.name };
                    let map = self.instantiate(&sig.generics, span);
                    let mut full = c.map.clone();
                    full.extend(map);
                    return Some(Ty::Func(sig.params.iter().map(|p| p.subst(&full)).collect(), Box::new(sig.ret.subst(&full))));
                }
                let short = key.rsplit("::").next().unwrap_or(&key).to_string();
                let mut d = Diagnostic::error(name.span, format!("`{short}` no tiene el elemento `{}`", name.name)).with_label("no existe");
                let mut names: Vec<String> = vec![];
                if let Some((_, vs)) = self.enum_variants(&key) {
                    names.extend(vs.iter().map(|v| v.name.clone()));
                }
                for i in self.impls_by_head.get(&key).cloned().unwrap_or_default() {
                    names.extend(self.impls[i].methods.iter().map(|(n, _)| n.clone()));
                }
                if let Some(s) = self.suggest(&name.name, names.into_iter()) {
                    d = d.with_note(format!("¿quisiste decir `{s}`?"));
                }
                self.diags.push(d);
                Some(Ty::Error)
            }
        }
    }

    // ---------- candidatos ----------

    fn static_cands(&mut self, key: &str, name: &str, span: Span) -> Vec<Cand> {
        let mut out = Vec::new();
        for i in self.impls_by_head.get(key).cloned().unwrap_or_default() {
            let imp = self.impls[i].clone();
            if !imp.methods.iter().any(|(n, _)| n == name) {
                continue;
            }
            let mut map = self.instantiate(&imp.generics, span);
            let self_ty = imp.target.subst(&map);
            self.add_trait_map(&imp, &mut map, &self_ty);
            for (n, sid) in &imp.methods {
                if n == name {
                    out.push(Cand { sig: *sid, map: map.clone() });
                }
            }
        }
        out
    }

    fn add_trait_map(&mut self, imp: &ImplInfo, map: &mut HashMap<Rc<str>, Ty>, self_ty: &Ty) {
        if let Some(tr) = &imp.trait_ref {
            map.insert(Rc::from("Self"), self_ty.clone());
            if let Some(ti) = self.traits.get(&tr.key) {
                for (g, a) in ti.generics.iter().zip(&tr.args) {
                    let sub = a.subst(map);
                    map.insert(g.name.clone(), sub);
                }
            }
        }
    }

    pub(super) fn method_cands(&mut self, ty: &Ty, name: &str, span: Span) -> Vec<Cand> {
        let mut cur = self.shallow(ty);
        for _ in 0..4 {
            let found = self.cands_for(&cur, name, span);
            if !found.is_empty() {
                return found;
            }
            match cur.clone() {
                Ty::Ref(i) => cur = self.shallow(&i),
                Ty::Named(n, a) if matches!(&*n, "Box" | "Rc") => cur = self.shallow(&a[0]),
                _ => break,
            }
        }
        vec![]
    }

    fn cands_for(&mut self, t: &Ty, name: &str, span: Span) -> Vec<Cand> {
        let mut out = Vec::new();
        let as_array;
        let t = if let Ty::Array(elem, _) = t {
            as_array = Ty::named("Array", vec![(**elem).clone()]);
            &as_array
        } else {
            t
        };
        match t {
            Ty::Param(p) => {
                let mut bounds: Vec<TraitRef> = vec![];
                for c in self.fcx.iter().rev() {
                    for g in &c.generics {
                        if g.name == *p {
                            bounds.extend(g.bounds.iter().cloned());
                        }
                    }
                }
                for b in bounds {
                    if let Some(ti) = self.traits.get(&b.key).cloned() {
                        for (n, sid) in &ti.methods {
                            if n == name {
                                let mut map = HashMap::new();
                                map.insert(Rc::from("Self"), t.clone());
                                for (g, a) in ti.generics.iter().zip(&b.args) {
                                    map.insert(g.name.clone(), a.clone());
                                }
                                out.push(Cand { sig: *sid, map });
                            }
                        }
                    }
                }
                return out;
            }
            Ty::Var(_) | Ty::Error | Ty::Never | Ty::Null => return out,
            _ => {}
        }
        let Some(head) = t.head_key() else { return out };
        for i in self.impls_by_head.get(&head).cloned().unwrap_or_default() {
            let imp = self.impls[i].clone();
            if !imp.methods.iter().any(|(n, _)| n == name) {
                continue;
            }
            let mut map: HashMap<Rc<str>, Ty> = HashMap::new();
            for g in &imp.generics {
                map.insert(g.name.clone(), self.fresh());
            }
            let target = imp.target.subst(&map);
            if !self.unify(&target, t) {
                continue;
            }
            for g in &imp.generics {
                for b in &g.bounds {
                    let args = b.args.iter().map(|a| a.subst(&map)).collect();
                    self.pending_bounds.push((map[&g.name].clone(), TraitRef { key: b.key.clone(), args }, span));
                }
            }
            self.add_trait_map(&imp, &mut map, t);
            for (n, sid) in &imp.methods {
                if n == name {
                    out.push(Cand { sig: *sid, map: map.clone() });
                }
            }
        }
        out
    }

    // ---------- llamadas ----------

    pub(super) fn check_call(&mut self, e: &mut Expr, _exp: Option<&Ty>) -> Ty {
        let span = e.span;
        let ExprKind::Call { callee, args } = &mut e.kind else { unreachable!() };
        if let ExprKind::Ident(n) = &callee.kind {
            if matches!(n.as_str(), "panic" | "exit" | "assert") && self.lookup_var(n).is_none() {
                let n = n.clone();
                return self.check_special_call(&n, args, span);
            }
        }
        match self.resolve_callee(callee, span) {
            Callee::Cands { cands, name, static_call } => match self.call_resolved(cands, args, span, &name, static_call) {
                Some((_, ret)) => ret,
                None => Ty::Error,
            },
            Callee::Value(fty) => self.call_value(&fty, args, span, callee.span),
            Callee::Error => {
                for a in args.iter_mut() {
                    self.check_expr(a, None);
                }
                Ty::Error
            }
        }
    }

    fn call_value(&mut self, fty: &Ty, args: &mut Vec<Expr>, span: Span, callee_span: Span) -> Ty {
        let f = self.shallow(fty);
        match f {
            Ty::Func(ps, r) => {
                if ps.len() != args.len() {
                    self.err_label(span, format!("la función espera {} argumento(s), pero se dieron {}", ps.len(), args.len()), "número de argumentos incorrecto");
                    for a in args.iter_mut() { self.check_expr(a, None); }
                } else {
                    for (a, p) in args.iter_mut().zip(&ps) {
                        self.check_coerce(a, p);
                    }
                }
                *r
            }
            Ty::Var(_) => {
                let tys: Vec<Ty> = args.iter_mut().map(|a| self.check_expr(a, None)).collect();
                let r = self.fresh();
                self.unify(&f, &Ty::Func(tys, Box::new(r.clone())));
                r
            }
            Ty::Error => {
                for a in args.iter_mut() { self.check_expr(a, None); }
                Ty::Error
            }
            other => {
                let o = self.resolve(&other);
                self.err_label(callee_span, format!("`{o}` no es una función y no se puede llamar"), "no es invocable");
                for a in args.iter_mut() { self.check_expr(a, None); }
                Ty::Error
            }
        }
    }

    fn global_cands(&mut self, m: usize, name: &str, span: Span) -> Vec<Cand> {
        let ids = self.mods[m].fns.get(name).cloned().unwrap_or_default();
        let mut out = Vec::new();
        for id in ids {
            let sig = self.sigs[id].clone();
            self.check_visible(m, sig.is_pub, name, span);
            out.push(Cand { sig: id, map: HashMap::new() });
        }
        out
    }

    fn resolve_callee(&mut self, callee: &mut Expr, span: Span) -> Callee {
        let cspan = callee.span;
        match &callee.kind {
            ExprKind::Ident(name) => {
                let name = name.clone();
                if self.lookup_var(&name).is_some() {
                    let t = self.check_expr(callee, None);
                    return Callee::Value(t);
                }
                if name == "self" {
                    self.err(cspan, "`self` no es una función");
                    return Callee::Error;
                }
                // función global
                if let Some((m, item)) = self.lookup_global_pub(&name) {
                    if self.mods[m].fns.contains_key(&item) {
                        let cands = self.global_cands(m, &item, cspan);
                        callee.kind = ExprKind::Global { module: m, name: item.clone() };
                        return Callee::Cands { cands, name: item, static_call: false };
                    }
                    let t = self.check_expr(callee, None);
                    return Callee::Value(t);
                }
                if matches!(name.as_str(), "Some" | "Ok" | "Error") {
                    let t = self.check_expr(callee, None);
                    return Callee::Value(t);
                }
                if self.lookup_type(self.cur_module, &name).is_some() {
                    self.diags.push(
                        Diagnostic::error(cspan, format!("`{name}` es un tipo y no se puede llamar como una función"))
                            .with_label("no es invocable")
                            .with_note(format!("para construirlo usa `{name} {{ campo: valor }}` o `{name}.new(...)`")),
                    );
                    return Callee::Error;
                }
                let t = self.check_expr(callee, None); // emite "no se encuentra el valor"
                Callee::Value(t)
            }
            ExprKind::Field(base, fname) => {
                let fname = fname.clone();
                let head = self.head_of(base);
                match head {
                    Head::Module(m) => {
                        if self.mods[m].fns.contains_key(&fname.name) {
                            let cands = self.global_cands(m, &fname.name, fname.span);
                            callee.kind = ExprKind::Global { module: m, name: fname.name.clone() };
                            return Callee::Cands { cands, name: fname.name, static_call: false };
                        }
                        match self.check_path_member(callee) {
                            Some(t) => Callee::Value(t),
                            None => Callee::Error,
                        }
                    }
                    Head::Type(key) => {
                        if self.enum_variants(&key).is_some_and(|(_, vs)| vs.iter().any(|v| v.name == fname.name)) {
                            let t = self.check_path_member(callee).unwrap_or(Ty::Error);
                            return Callee::Value(t);
                        }
                        let cands = self.static_cands(&key, &fname.name, span);
                        if cands.is_empty() {
                            let _ = self.check_path_member(callee);
                            return Callee::Error;
                        }
                        callee.kind = ExprKind::TypeStatic { key, name: fname.name.clone() };
                        Callee::Cands { cands, name: fname.name, static_call: true }
                    }
                    Head::Value => {
                        let ExprKind::Field(base, _) = &mut callee.kind else { unreachable!() };
                        let bt = self.check_expr(base, None);
                        let rbt = self.deref_for_member(&bt);
                        if rbt.is_error() {
                            return Callee::Error;
                        }
                        let cands = self.method_cands(&bt, &fname.name, span);
                        if !cands.is_empty() {
                            return Callee::Cands { cands, name: fname.name, static_call: false };
                        }
                        if let Some(ft) = self.field_type(&bt, &fname.name, fname.span) {
                            return Callee::Value(ft);
                        }
                        let rs = self.resolve(&rbt);
                        if matches!(rs, Ty::Var(_)) {
                            self.err_label(fname.span, format!("no se conoce el tipo del receptor para llamar a `{}`", fname.name), "tipo aún no inferido");
                            return Callee::Error;
                        }
                        let mut names: Vec<String> = vec![];
                        if let Some(h) = rs.head_key() {
                            for i in self.impls_by_head.get(&h).cloned().unwrap_or_default() {
                                names.extend(self.impls[i].methods.iter().map(|(n, _)| n.clone()));
                            }
                        }
                        let mut d = Diagnostic::error(fname.span, format!("el tipo `{rs}` no tiene un método `{}`", fname.name)).with_label("método inexistente");
                        if matches!(rs, Ty::Union(_)) {
                            d = d.with_note("para usar un valor de tipo unión primero hay que acotarlo con `match`");
                        } else if let Some(s) = self.suggest(&fname.name, names.into_iter()) {
                            d = d.with_note(format!("¿quisiste decir `{s}`?"));
                        }
                        self.diags.push(d);
                        Callee::Error
                    }
                }
            }
            _ => {
                let t = self.check_expr(callee, None);
                Callee::Value(t)
            }
        }
    }

    fn lookup_global_pub(&self, name: &str) -> Option<(usize, String)> {
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

    /// Resuelve sobrecargas, comprueba argumentos y devuelve (firma elegida, tipo de retorno).
    fn call_resolved(&mut self, cands: Vec<Cand>, args: &mut Vec<Expr>, span: Span, name: &str, static_call: bool) -> Option<(usize, Ty)> {
        let n = args.len();
        let fit: Vec<usize> = (0..cands.len())
            .filter(|&i| {
                let s = &self.sigs[cands[i].sig];
                n >= s.required() && (n <= s.params.len() || s.variadic)
            })
            .collect();
        if fit.is_empty() {
            let s = self.sigs[cands[0].sig].clone();
            let expected = if s.required() == s.params.len() { format!("{}", s.params.len()) } else { format!("entre {} y {}", s.required(), s.params.len()) };
            self.diags.push(
                Diagnostic::error(span, format!("`{name}` espera {expected} argumento(s), pero se dieron {n}"))
                    .with_label("número de argumentos incorrecto")
                    .with_note(format!("firma: {}", self.sig_text(&s))),
            );
            for a in args.iter_mut() {
                self.check_expr(a, None);
            }
            return None;
        }
        let mut prechecked: Option<Vec<Ty>> = None;
        let chosen = if fit.len() == 1 {
            fit[0]
        } else {
            let tys: Vec<Ty> = args.iter_mut().map(|a| self.check_expr(a, None)).collect();
            let mut pick = None;
            // primero sobrecargas que encajan sin coerciones; después las que las necesitan
            'passes: for exact in [true, false] {
                for &i in &fit {
                    let s = self.sigs[cands[i].sig].clone();
                    let mut map = cands[i].map.clone();
                    for g in &s.generics {
                        map.insert(g.name.clone(), self.fresh());
                    }
                    let snap = self.snapshot();
                    let ok = tys.iter().zip(&s.params).all(|(t, p)| {
                        let p = p.subst(&map);
                        let good = if exact { self.unify(t, &p) } else { self.can_coerce(t, &p) && self.unify(t, &p) || self.can_coerce(t, &p) };
                        good
                    });
                    self.rollback(snap);
                    if ok {
                        pick = Some(i);
                        break 'passes;
                    }
                }
            }
            prechecked = Some(tys.clone());
            match pick {
                Some(i) => i,
                None => {
                    let shown: Vec<String> = tys.iter().map(|t| self.resolve(t).to_string()).collect();
                    let mut d = Diagnostic::error(span, format!("ninguna sobrecarga de `{name}` acepta los argumentos ({})", shown.join(", ")))
                        .with_label("sobrecarga no encontrada");
                    for &i in &fit {
                        d = d.with_note(format!("candidata: {}", self.sig_text(&self.sigs[cands[i].sig].clone())));
                    }
                    self.diags.push(d);
                    return None;
                }
            }
        };
        let sig = self.sigs[cands[chosen].sig].clone();
        if static_call && !sig.is_static {
            self.diags.push(
                Diagnostic::error(span, format!("`{name}` es un método de instancia: llámalo sobre un valor, no sobre el tipo"))
                    .with_label("requiere `self`"),
            );
        }
        if sig.is_unsafe && !self.in_unsafe() {
            self.diags.push(
                Diagnostic::error(span, format!("llamar a `{name}` requiere un bloque `unsafe`"))
                    .with_label("operación peligrosa fuera de `unsafe`")
                    .with_note("envuelve la llamada en `unsafe { ... }`"),
            );
        }
        if sig.module != self.cur_module && !sig.is_pub && !self.mod_std[sig.module] {
            self.err_label(span, format!("`{name}` es privado en el módulo `{}`", self.mod_names[sig.module]), "no es `pub`");
        }
        let mut map = cands[chosen].map.clone();
        let own = self.instantiate(&sig.generics, span);
        map.extend(own);
        let ptys: Vec<Ty> = sig.params.iter().map(|p| p.subst(&map)).collect();
        for (k, a) in args.iter_mut().enumerate() {
            if k < ptys.len() {
                let t = match &prechecked {
                    Some(v) => v[k].clone(),
                    None => self.check_expr(a, Some(&ptys[k])),
                };
                if !self.coerce(a, &t, &ptys[k]) {
                    self.mismatch(a.span, &ptys[k], &t);
                }
            } else if prechecked.is_none() {
                self.check_expr(a, None);
            }
        }
        for _ in n..ptys.len() {
            args.push(Expr { kind: ExprKind::Null, span });
        }
        let ret = sig.ret.subst(&map);
        let ret = if sig.is_async { Ty::named("Future", vec![ret]) } else { ret };
        Some((cands[chosen].sig, ret))
    }

    fn sig_text(&self, s: &FnSig) -> String {
        let ps: Vec<String> = s.params.iter().map(|p| self.resolve(p).to_string()).collect();
        format!("{}({}) -> {}", s.name, ps.join(", "), self.resolve(&s.ret))
    }

    fn check_special_call(&mut self, name: &str, args: &mut Vec<Expr>, span: Span) -> Ty {
        match name {
            "panic" => {
                if args.len() > 1 {
                    self.err_label(span, "`panic` admite como máximo un argumento (el mensaje)", "demasiados argumentos");
                }
                for a in args.iter_mut() { self.check_expr(a, None); }
                Ty::Never
            }
            "exit" => {
                if args.len() != 1 {
                    self.err_label(span, "`exit` espera un argumento: el código de salida", "número de argumentos incorrecto");
                    for a in args.iter_mut() { self.check_expr(a, None); }
                } else {
                    self.check_coerce(&mut args[0], &Ty::Int);
                }
                Ty::Never
            }
            _ => {
                if args.is_empty() || args.len() > 2 {
                    self.err_label(span, "`assert` espera una condición y, opcionalmente, un mensaje", "número de argumentos incorrecto");
                    for a in args.iter_mut() { self.check_expr(a, None); }
                } else {
                    self.check_coerce(&mut args[0], &Ty::Bool);
                    if args.len() == 2 {
                        self.check_coerce(&mut args[1], &Ty::Str);
                    }
                }
                Ty::Nothing
            }
        }
    }
}
