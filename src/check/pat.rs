//! Patrones: tipado, enlaces, exhaustividad y patrones inalcanzables.

use super::*;

#[derive(Clone, Debug)]
pub enum NPat {
    Wild,
    Ctor(String, Vec<NPat>),
    Lit,
}

impl Checker {
    fn enum_def(&self, key: &str) -> Option<(Vec<GenericInfo>, Vec<VariantInfo>)> {
        match self.types.get(key) {
            Some(TypeDef { generics, kind: DefKind::Enum(v), .. }) => Some((generics.clone(), v.clone())),
            _ => None,
        }
    }

    /// Resuelve `Enum.Variant`, `mod.Enum.Variant`, `Self.Variant` o las variantes sueltas `Some/None/Ok/Error`.
    pub(super) fn resolve_variant_path(&mut self, path: &[Ident]) -> Option<(String, String)> {
        let last = path.last().unwrap();
        if path.len() == 1 {
            return match last.name.as_str() {
                "Some" | "None" => Some(("Option".into(), last.name.clone())),
                "Ok" | "Error" => Some(("Result".into(), last.name.clone())),
                n => {
                    self.err(last.span, format!("`{n}` no es una variante conocida; escribe `Enum.{n}`"));
                    None
                }
            };
        }
        let enum_path = &path[..path.len() - 1];
        let key = if enum_path.len() == 1 {
            let n = &enum_path[0].name;
            if n == "Self" {
                self.fcx.last().and_then(|c| c.self_ty.clone()).and_then(|t| match self.shallow(&t) {
                    Ty::Named(k, _) => Some(k.to_string()),
                    _ => None,
                })
            } else {
                self.lookup_type(self.cur_module, n)
            }
        } else {
            let mname = enum_path[..enum_path.len() - 1].iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
            self.find_module(self.cur_module, &mname).and_then(|m| self.mods[m].types.get(&enum_path.last().unwrap().name).cloned())
        };
        let Some(key) = key else {
            let ep = enum_path.last().unwrap();
            self.err(ep.span, format!("no se encuentra el enum `{}`", ep.name));
            return None;
        };
        let Some((_, variants)) = self.enum_def(&key) else {
            self.err(enum_path.last().unwrap().span, format!("`{}` no es un enum", enum_path.last().unwrap().name));
            return None;
        };
        if !variants.iter().any(|v| v.name == last.name) {
            let mut d = Diagnostic::error(last.span, format!("el enum `{}` no tiene la variante `{}`", enum_path.last().unwrap().name, last.name));
            if let Some(s) = self.suggest(&last.name, variants.iter().map(|v| v.name.clone())) {
                d = d.with_note(format!("¿quisiste decir `{s}`?"));
            }
            self.diags.push(d);
            return None;
        }
        Some((key, last.name.clone()))
    }

    pub(super) fn check_pattern(&mut self, p: &mut Pattern, scrut: &Ty) {
        let mut st = self.shallow(scrut);
        while let Ty::Ref(inner) = st.clone() {
            st = self.shallow(&inner);
        }
        match p {
            Pattern::Wild(_) => {}
            Pattern::Bind { name, ty } => {
                let bty = match ty {
                    Some(te) => {
                        let sc = self.body_scope();
                        let t = self.resolve_type(te, &sc);
                        let ok = match &st {
                            Ty::Union(members) => members.iter().any(|m| *m == t) || t.is_error(),
                            _ => self.can_unify(&t, &st),
                        };
                        if !ok {
                            self.err_label(te.span, format!("el tipo `{t}` no es una alternativa de `{}`", self.resolve(&st)), "no coincide");
                        }
                        t
                    }
                    None => st.clone(),
                };
                self.declare(&name.name.clone(), bty, name.span, true, true);
            }
            Pattern::Lit(e) => {
                let t = self.check_expr(e, Some(&st));
                if !self.coerce_noop(&t, &st) {
                    self.mismatch(e.span, &st, &t);
                }
            }
            Pattern::Tuple(items, span) => match &st {
                Ty::Tuple(tys) if tys.len() == items.len() => {
                    for (it, t) in items.iter_mut().zip(tys.clone()) {
                        self.check_pattern(it, &t);
                    }
                }
                Ty::Var(_) | Ty::Error => {
                    let vars: Vec<Ty> = items.iter().map(|_| self.fresh()).collect();
                    self.unify(&st, &Ty::Tuple(vars.clone()));
                    for (it, t) in items.iter_mut().zip(vars) {
                        self.check_pattern(it, &t);
                    }
                }
                other => {
                    let o = self.resolve(other);
                    self.err(*span, format!("el patrón de tupla de {} elementos no encaja con `{o}`", items.len()));
                }
            },
            Pattern::Variant { path, args, span, res } => {
                let Some((ekey, vname)) = self.resolve_variant_path(path) else { return };
                let (generics, variants) = self.enum_def(&ekey).unwrap();
                let targs: Vec<Ty> = generics.iter().map(|_| self.fresh()).collect();
                let ety = Ty::Named(Rc::from(ekey.as_str()), targs.clone());
                if !self.unify(&ety, &st) {
                    let s = self.resolve(&st);
                    if !s.is_error() {
                        self.err_label(*span, format!("el patrón `{}` es de tipo `{}`, pero se compara con `{s}`", path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("."), self.resolve(&ety)), "tipo incompatible");
                    }
                    return;
                }
                let map: HashMap<Rc<str>, Ty> = generics.iter().map(|g| g.name.clone()).zip(targs).collect();
                let variant = variants.iter().find(|v| v.name == vname).unwrap();
                let ftys: Vec<Ty> = variant.fields.iter().map(|t| t.subst(&map)).collect();
                match args {
                    Some(sub) => {
                        if sub.len() != ftys.len() {
                            self.err(*span, format!("la variante `{vname}` tiene {} campo(s), pero el patrón da {}", ftys.len(), sub.len()));
                        } else {
                            for (sp, t) in sub.iter_mut().zip(ftys) {
                                self.check_pattern(sp, &t);
                            }
                        }
                    }
                    None => {
                        if !ftys.is_empty() {
                            self.err(*span, format!("la variante `{vname}` tiene {} campo(s); usa `{vname}(..)` para desestructurarla", ftys.len()));
                        }
                    }
                }
                *res = Some((ekey, vname));
            }
        }
    }

    fn coerce_noop(&mut self, found: &Ty, expected: &Ty) -> bool {
        let f = self.shallow(found);
        if f == Ty::Null {
            return self.shallow(expected).is_option();
        }
        self.can_unify(found, expected)
    }

    // ---------- exhaustividad ----------

    fn to_npat(&self, p: &Pattern) -> NPat {
        match p {
            Pattern::Wild(_) => NPat::Wild,
            Pattern::Bind { ty: None, .. } => NPat::Wild,
            Pattern::Bind { ty: Some(_), name } => NPat::Ctor(format!("@{}", name.name), vec![]),
            Pattern::Lit(e) => match &e.kind {
                ExprKind::Bool(b) => NPat::Ctor(b.to_string(), vec![]),
                _ => NPat::Lit,
            },
            Pattern::Tuple(items, _) => NPat::Ctor("()".into(), items.iter().map(|i| self.to_npat(i)).collect()),
            Pattern::Variant { args, res, .. } => {
                let Some((ekey, v)) = res else { return NPat::Wild };
                let arity = self.enum_def(ekey).and_then(|(_, vs)| vs.iter().find(|x| &x.name == v).map(|x| x.fields.len())).unwrap_or(0);
                let subs = match args {
                    Some(a) => a.iter().map(|x| self.to_npat(x)).collect(),
                    None => vec![NPat::Wild; arity],
                };
                NPat::Ctor(v.clone(), subs)
            }
        }
    }

    /// Constructores de un tipo con firma completa: (nombre, tipos de campos).
    fn ctors_of(&self, ty: &Ty) -> Option<Vec<(String, Vec<Ty>)>> {
        match self.shallow(ty) {
            Ty::Bool => Some(vec![("true".into(), vec![]), ("false".into(), vec![])]),
            Ty::Tuple(v) => Some(vec![("()".into(), v)]),
            Ty::Ref(t) => self.ctors_of(&t),
            Ty::Named(k, args) => {
                let (generics, variants) = self.enum_def(&k)?;
                let map: HashMap<Rc<str>, Ty> = generics.iter().map(|g| g.name.clone()).zip(args).collect();
                Some(variants.iter().map(|v| (v.name.clone(), v.fields.iter().map(|t| t.subst(&map)).collect())).collect())
            }
            _ => None,
        }
    }

    fn union_ctors(&self, ty: &Ty) -> Option<Vec<String>> {
        match self.shallow(ty) {
            Ty::Union(m) => Some(m.iter().map(|t| format!("@{}", self.resolve(t))).collect()),
            _ => None,
        }
    }

    fn specialize(rows: &[Vec<NPat>], ctor: &str, arity: usize) -> Vec<Vec<NPat>> {
        let mut out = vec![];
        for r in rows {
            match &r[0] {
                NPat::Ctor(c, subs) if c == ctor => {
                    let mut nr = subs.clone();
                    nr.resize(arity, NPat::Wild);
                    nr.extend_from_slice(&r[1..]);
                    out.push(nr);
                }
                NPat::Wild => {
                    let mut nr = vec![NPat::Wild; arity];
                    nr.extend_from_slice(&r[1..]);
                    out.push(nr);
                }
                _ => {}
            }
        }
        out
    }

    fn default_rows(rows: &[Vec<NPat>]) -> Vec<Vec<NPat>> {
        rows.iter().filter(|r| matches!(r[0], NPat::Wild)).map(|r| r[1..].to_vec()).collect()
    }

    /// ¿Hay algún valor cubierto por `q` que no cubra ninguna fila?
    fn useful(&self, rows: &[Vec<NPat>], q: &[NPat], tys: &[Ty]) -> bool {
        if q.is_empty() {
            return rows.is_empty();
        }
        let rest_tys = &tys[1..];
        let ty = &tys[0];
        let sig: Option<Vec<(String, Vec<Ty>)>> = self
            .ctors_of(ty)
            .or_else(|| self.union_ctors(ty).map(|v| v.into_iter().map(|c| (c, vec![])).collect()));
        match &q[0] {
            NPat::Ctor(c, subs) => {
                let arity = sig.as_ref().and_then(|s| s.iter().find(|(n, _)| n == c)).map(|(_, f)| f.len()).unwrap_or(subs.len());
                let ftys: Vec<Ty> = sig.as_ref().and_then(|s| s.iter().find(|(n, _)| n == c)).map(|(_, f)| f.clone()).unwrap_or_else(|| vec![Ty::Error; arity]);
                let spec = Self::specialize(rows, c, arity);
                let mut nq = subs.clone();
                nq.resize(arity, NPat::Wild);
                nq.extend_from_slice(&q[1..]);
                let mut ntys = ftys;
                ntys.extend_from_slice(rest_tys);
                self.useful(&spec, &nq, &ntys)
            }
            NPat::Lit => {
                let def = Self::default_rows(rows);
                self.useful(&def, &q[1..], rest_tys)
            }
            NPat::Wild => {
                if let Some(sig) = &sig {
                    let used: HashSet<&str> = rows.iter().filter_map(|r| if let NPat::Ctor(c, _) = &r[0] { Some(c.as_str()) } else { None }).collect();
                    let complete = sig.iter().all(|(n, _)| used.contains(n.as_str()));
                    if complete {
                        return sig.iter().any(|(c, ftys)| {
                            let spec = Self::specialize(rows, c, ftys.len());
                            let mut nq = vec![NPat::Wild; ftys.len()];
                            nq.extend_from_slice(&q[1..]);
                            let mut ntys = ftys.clone();
                            ntys.extend_from_slice(rest_tys);
                            self.useful(&spec, &nq, &ntys)
                        });
                    }
                }
                let def = Self::default_rows(rows);
                self.useful(&def, &q[1..], rest_tys)
            }
        }
    }

    /// Comprueba exhaustividad del `match` y avisa de brazos inalcanzables.
    pub(super) fn check_match_coverage(&mut self, arms: &[MatchArm], scrut: &Ty, span: Span) {
        let sty = self.resolve(scrut);
        if sty.is_error() {
            return;
        }
        let mut rows: Vec<Vec<NPat>> = Vec::new();
        let tys = vec![sty.clone()];
        for arm in arms {
            let np = self.to_npat(&arm.pattern);
            if !self.useful(&rows, &[np.clone()], &tys) {
                self.warn("unreachable-pattern", arm.pattern.span(), "patrón inalcanzable: los brazos anteriores ya cubren todos estos casos");
            }
            if arm.guard.is_none() {
                rows.push(vec![np]);
            }
        }
        if self.useful(&rows, &[NPat::Wild], &tys) {
            let mut d = Diagnostic::error(span, "`match` no exhaustivo: faltan casos por cubrir").with_label("no cubre todos los valores");
            if let Some(sig) = self.ctors_of(&sty) {
                let covered: HashSet<String> = rows.iter().filter_map(|r| if let NPat::Ctor(c, _) = &r[0] { Some(c.clone()) } else { None }).collect();
                let missing: Vec<String> = sig.iter().map(|(n, _)| n.clone()).filter(|n| !covered.contains(n)).collect();
                if !missing.is_empty() {
                    d = d.with_note(format!("faltan: {}", missing.join(", ")));
                }
            }
            d = d.with_note("añade los casos que faltan o un brazo comodín `_ => ...`");
            self.diags.push(d);
        }
    }
}
