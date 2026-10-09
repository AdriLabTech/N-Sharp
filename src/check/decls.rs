//! Recolección de declaraciones, imports, resolución de tipos y firmas.

use super::*;
use crate::types::qualify;

pub struct TyScope {
    pub module: usize,
    pub generics: Vec<Rc<str>>,
    pub self_ty: Option<Ty>,
}

impl Checker {
    pub(super) fn collect_all(&mut self, asts: &mut [Module]) {
        let mod_index: HashMap<String, usize> = self.mod_names.iter().cloned().enumerate().map(|(i, n)| (n, i)).collect();
        // P1: nombres de tipos, funciones y constantes.
        let mut value_names: Vec<HashSet<String>> = vec![HashSet::new(); asts.len()];
        for (mi, ast) in asts.iter().enumerate() {
            for it in &ast.items {
                match &it.kind {
                    ItemKind::Struct(s) => self.register_type(mi, &s.name, s.is_pub, DefKind::Struct(vec![]), s.fields.is_empty()),
                    ItemKind::Enum(e) => self.register_type(mi, &e.name, e.is_pub, DefKind::Enum(vec![]), false),
                    ItemKind::Trait(t) => self.register_type(mi, &t.name, t.is_pub, DefKind::Trait, false),
                    ItemKind::Alias(a) => {
                        self.register_type(mi, &a.name, a.is_pub, DefKind::Alias, false);
                        let key = qualify(&self.mod_names[mi], &a.name.name);
                        self.alias_defs.insert(key, (mi, a.generics.iter().map(|g| g.name.name.clone()).collect(), a.ty.clone()));
                    }
                    ItemKind::Fn(f) => { value_names[mi].insert(f.name.name.clone()); }
                    ItemKind::Extern { fns, .. } => for f in fns { value_names[mi].insert(f.name.name.clone()); },
                    ItemKind::Const(c) | ItemKind::Static(c) => { value_names[mi].insert(c.name.name.clone()); }
                    _ => {}
                }
            }
        }
        // P2: imports.
        for (mi, ast) in asts.iter().enumerate() {
            for it in &ast.items {
                let ItemKind::Import(paths) = &it.kind else { continue };
                for ip in paths {
                    self.resolve_import(mi, ip, &mod_index, &value_names);
                }
            }
        }
        // P3: genéricos, campos y variantes.
        for (mi, ast) in asts.iter().enumerate() {
            for it in &ast.items {
                match &it.kind {
                    ItemKind::Struct(s) => self.define_struct(mi, s),
                    ItemKind::Enum(e) => self.define_enum(mi, e),
                    ItemKind::Trait(t) => self.define_trait_header(mi, t),
                    _ => {}
                }
            }
        }
        // P4: firmas de traits, impls, funciones libres y constantes.
        self.item_sigs = vec![]; // se reinicia abajo con el tamaño correcto
        let mut item_sigs: Vec<Vec<Vec<usize>>> = Vec::new();
        for (mi, ast) in asts.iter().enumerate() {
            let mut per_item = Vec::new();
            for it in &ast.items {
                let mut ids = Vec::new();
                match &it.kind {
                    ItemKind::Trait(t) => ids = self.define_trait_methods(mi, t),
                    _ => {}
                }
                per_item.push(ids);
            }
            item_sigs.push(per_item);
        }
        for (mi, ast) in asts.iter().enumerate() {
            for (ii, it) in ast.items.iter().enumerate() {
                match &it.kind {
                    ItemKind::Fn(f) => {
                        let sig = self.make_sig(f, mi, &[], None, &[]);
                        let id = self.push_sig(mi, sig, true);
                        item_sigs[mi][ii] = vec![id];
                    }
                    ItemKind::Extern { fns, .. } => {
                        for f in fns {
                            let sig = self.make_sig(f, mi, &[], None, &[]);
                            let id = self.push_sig(mi, sig, true);
                            item_sigs[mi][ii].push(id);
                        }
                    }
                    ItemKind::Const(c) | ItemKind::Static(c) => {
                        let sc = TyScope { module: mi, generics: vec![], self_ty: None };
                        let ty = self.resolve_type(&c.ty, &sc);
                        let is_static = matches!(it.kind, ItemKind::Static(_));
                        if self.mods[mi].consts.insert(c.name.name.clone(), (ty, is_static)).is_some() {
                            self.err(c.name.span, format!("`{}` está definido más de una vez", c.name.name));
                        }
                    }
                    _ => {}
                }
            }
        }
        for (mi, ast) in asts.iter_mut().enumerate() {
            for (ii, it) in ast.items.iter_mut().enumerate() {
                if let ItemKind::Impl(imp) = &mut it.kind {
                    item_sigs[mi][ii] = self.define_impl(mi, imp);
                    self.impl_of_item.insert((mi, ii), self.impls.len() - 1);
                }
            }
        }
        self.item_sigs = item_sigs;
        self.check_overload_duplicates();
    }

    fn register_type(&mut self, mi: usize, name: &Ident, is_pub: bool, kind: DefKind, opaque_candidate: bool) {
        let key = qualify(&self.mod_names[mi], &name.name);
        if self.types.contains_key(&key) {
            self.err(name.span, format!("el tipo `{}` está definido más de una vez", name.name));
            return;
        }
        let opaque = opaque_candidate && self.mod_std[mi] && matches!(kind, DefKind::Struct(_));
        self.mods[mi].types.insert(name.name.clone(), key.clone());
        self.types.insert(key.clone(), TypeDef { name: name.name.clone(), key, module: mi, generics: vec![], kind, is_pub, span: name.span, opaque });
    }

    fn resolve_import(&mut self, mi: usize, ip: &ImportPath, mod_index: &HashMap<String, usize>, values: &[HashSet<String>]) {
        let segs: Vec<&str> = ip.path.iter().map(|i| i.name.as_str()).collect();
        for len in (1..=segs.len()).rev() {
            let name = segs[..len].join(".");
            let Some(&m) = mod_index.get(&name) else { continue };
            let rest = &segs[len..];
            let span = ip.path[0].span;
            if rest.is_empty() {
                if ip.wildcard {
                    self.mods[mi].wildcards.push(m);
                } else {
                    let alias = ip.alias.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| segs[len - 1].to_string());
                    self.mods[mi].imports.insert(alias, ImportTarget::Module(m));
                }
            } else if rest.len() == 1 && !ip.wildcard {
                let item = rest[0];
                let is_type = self.mods[m].types.contains_key(item);
                let is_val = values[m].contains(item);
                if !is_type && !is_val {
                    let cands: Vec<String> = self.mods[m].types.keys().cloned().chain(values[m].iter().cloned()).collect();
                    let mut d = Diagnostic::error(ip.path.last().unwrap().span, format!("el módulo `{name}` no tiene un elemento `{item}`"));
                    if let Some(s) = self.suggest(item, cands.into_iter()) {
                        d = d.with_note(format!("¿quisiste decir `{s}`?"));
                    }
                    self.diags.push(d);
                    return;
                }
                let alias = ip.alias.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| item.to_string());
                self.mods[mi].imports.insert(alias, ImportTarget::Item(m, item.to_string()));
            } else {
                self.err(span, format!("ruta de import no válida: `{}`", segs.join(".")));
            }
            return;
        }
    }

    fn define_struct(&mut self, mi: usize, s: &StructDecl) {
        let key = qualify(&self.mod_names[mi], &s.name.name);
        let names: Vec<Rc<str>> = s.generics.iter().map(|g| Rc::from(g.name.name.as_str())).collect();
        let sc = TyScope { module: mi, generics: names, self_ty: None };
        let generics = self.resolve_generics(&s.generics, &sc);
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        for f in &s.fields {
            if !seen.insert(f.name.name.clone()) {
                self.err(f.name.span, format!("el campo `{}` está duplicado", f.name.name));
            }
            let ty = self.resolve_type(&f.ty, &sc);
            fields.push(FieldInfo { name: f.name.name.clone(), ty, is_pub: f.is_pub });
        }
        if let Some(d) = self.types.get_mut(&key) {
            d.generics = generics;
            d.kind = DefKind::Struct(fields);
        }
    }

    fn define_enum(&mut self, mi: usize, e: &EnumDecl) {
        let key = qualify(&self.mod_names[mi], &e.name.name);
        let names: Vec<Rc<str>> = e.generics.iter().map(|g| Rc::from(g.name.name.as_str())).collect();
        let sc = TyScope { module: mi, generics: names, self_ty: None };
        let generics = self.resolve_generics(&e.generics, &sc);
        let mut variants = Vec::new();
        let mut seen = HashSet::new();
        for v in &e.variants {
            if !seen.insert(v.name.name.clone()) {
                self.err(v.name.span, format!("la variante `{}` está duplicada", v.name.name));
            }
            let fields = v.fields.iter().map(|t| self.resolve_type(t, &sc)).collect();
            variants.push(VariantInfo { name: v.name.name.clone(), fields });
        }
        if variants.is_empty() {
            self.err(e.name.span, format!("el enum `{}` no tiene variantes", e.name.name));
        }
        if let Some(d) = self.types.get_mut(&key) {
            d.generics = generics;
            d.kind = DefKind::Enum(variants);
        }
    }

    fn define_trait_header(&mut self, mi: usize, t: &TraitDecl) {
        let key = qualify(&self.mod_names[mi], &t.name.name);
        let names: Vec<Rc<str>> = t.generics.iter().map(|g| Rc::from(g.name.name.as_str())).collect();
        let sc = TyScope { module: mi, generics: names, self_ty: None };
        let generics = self.resolve_generics(&t.generics, &sc);
        if let Some(d) = self.types.get_mut(&key) {
            d.generics = generics.clone();
        }
        self.traits.insert(key.clone(), TraitInfo { key, name: t.name.name.clone(), module: mi, generics, methods: vec![] });
    }

    fn define_trait_methods(&mut self, mi: usize, t: &TraitDecl) -> Vec<usize> {
        let key = qualify(&self.mod_names[mi], &t.name.name);
        let mut outer: Vec<Rc<str>> = t.generics.iter().map(|g| Rc::from(g.name.name.as_str())).collect();
        outer.push(Rc::from("Self"));
        let mut ids = vec![];
        for m in &t.methods {
            let sig = self.make_sig(m, mi, &outer, Some(Ty::Param(Rc::from("Self"))), &[]);
            let id = self.sigs.len();
            self.sigs.push(sig);
            ids.push(id);
            if let Some(ti) = self.traits.get_mut(&key) {
                ti.methods.push((m.name.name.clone(), id));
            }
        }
        ids
    }

    fn define_impl(&mut self, mi: usize, imp: &mut ImplDecl) -> Vec<usize> {
        // genéricos explícitos + implícitos (`impl Stack<T>` sin declarar `<T>`)
        let mut gnames: Vec<Rc<str>> = imp.generics.iter().map(|g| Rc::from(g.name.name.as_str())).collect();
        let mut implicit = Vec::new();
        self.collect_implicit_generics(&imp.target, mi, &gnames, &mut implicit);
        if let Some(t) = &imp.trait_ {
            self.collect_implicit_generics(t, mi, &gnames, &mut implicit);
        }
        for n in &implicit {
            gnames.push(Rc::from(n.as_str()));
        }
        let sc0 = TyScope { module: mi, generics: gnames.clone(), self_ty: None };
        let mut generics = self.resolve_generics(&imp.generics, &sc0);
        for n in implicit {
            generics.push(GenericInfo { name: Rc::from(n.as_str()), bounds: vec![] });
        }
        let target = self.resolve_type(&imp.target, &sc0);
        let sc = TyScope { module: mi, generics: gnames.clone(), self_ty: Some(target.clone()) };
        let trait_ref = imp.trait_.as_ref().and_then(|t| self.resolve_trait_ref(t, &sc));
        imp.key = target.head_key().unwrap_or_default();
        imp.trait_key = trait_ref.as_ref().map(|t| t.key.clone());
        if imp.key.is_empty() && !target.is_error() {
            self.err(imp.target.span, format!("no se puede implementar sobre el tipo `{target}`"));
        }
        let mut methods: Vec<(String, usize)> = Vec::new();
        let mut ids = Vec::new();
        for m in &imp.methods {
            if methods.iter().any(|(n, _)| *n == m.name.name) {
                // sobrecarga: permitida; los duplicados exactos se detectan luego
            }
            if m.body.is_none() && !self.mod_std[mi] {
                self.err_label(m.span, format!("el método `{}` no tiene cuerpo", m.name.name), "se esperaba `{ ... }`");
            }
            let sig = self.make_sig(m, mi, &gnames, Some(target.clone()), &[]);
            let id = self.sigs.len();
            self.sigs.push(sig);
            methods.push((m.name.name.clone(), id));
            ids.push(id);
        }
        if let (Some(tr), Some(head)) = (&trait_ref, target.head_key()) {
            if !self.trait_impls.insert((tr.key.clone(), head.clone())) {
                self.err(imp.span, format!("ya existe una implementación de `{}` para `{}`", tr.key.rsplit("::").next().unwrap_or(&tr.key), target));
            }
            if let Some(ti) = self.traits.get(&tr.key).cloned() {
                if !self.mod_std[mi] {
                    for (tname, tsig) in &ti.methods {
                        let present = methods.iter().find(|(n, _)| n == tname);
                        match present {
                            Some((_, sid)) => self.check_impl_method_compat(*sid, *tsig, &target, &tr.args, &ti),
                            None if !self.sigs[*tsig].has_body => {
                                self.err_label(imp.span, format!("falta implementar `{}` del trait `{}`", tname, ti.name), "implementación incompleta");
                            }
                            None => {}
                        }
                    }
                    for (n, sid) in &methods {
                        if !ti.methods.iter().any(|(tn, _)| tn == n) {
                            let span = self.sigs[*sid].span;
                            self.err(span, format!("`{n}` no es un método del trait `{}`", ti.name));
                        }
                    }
                }
                // métodos por defecto heredados
                for (tname, tsig) in &ti.methods {
                    if self.sigs[*tsig].has_body && !methods.iter().any(|(n, _)| n == tname) {
                        methods.push((tname.clone(), *tsig));
                    }
                }
            }
        }
        let idx = self.impls.len();
        if let Some(head) = target.head_key() {
            self.impls_by_head.entry(head).or_default().push(idx);
        }
        self.impls.push(ImplInfo { module: mi, generics, trait_ref, target, methods });
        ids
    }

    fn check_impl_method_compat(&mut self, impl_sig: usize, trait_sig: usize, target: &Ty, trait_args: &[Ty], ti: &TraitInfo) {
        let mut map: HashMap<Rc<str>, Ty> = HashMap::new();
        map.insert(Rc::from("Self"), target.clone());
        for (g, a) in ti.generics.iter().zip(trait_args) {
            map.insert(g.name.clone(), a.clone());
        }
        let isg = self.sigs[impl_sig].clone();
        let tsg = self.sigs[trait_sig].clone();
        let expected: Vec<Ty> = tsg.params.iter().map(|t| t.subst(&map)).collect();
        let ret = tsg.ret.subst(&map);
        if isg.params.len() != expected.len() {
            self.err(isg.span, format!("`{}` declara {} parámetros pero el trait espera {}", isg.name, isg.params.len(), expected.len()));
            return;
        }
        for (a, b) in isg.params.iter().zip(&expected).chain(std::iter::once((&isg.ret, &ret))) {
            if !Self::loosely_equal(a, b) {
                self.err(isg.span, format!("la firma de `{}` no coincide con la del trait: se esperaba `{b}` pero hay `{a}`", isg.name));
                return;
            }
        }
    }

    fn loosely_equal(a: &Ty, b: &Ty) -> bool {
        fn has_param(t: &Ty) -> bool {
            match t {
                Ty::Param(_) => true,
                Ty::Named(_, a) | Ty::Tuple(a) | Ty::Union(a) => a.iter().any(has_param),
                Ty::Array(t, _) | Ty::Ref(t) | Ty::Ptr(t) => has_param(t),
                Ty::Func(p, r) => p.iter().any(has_param) || has_param(r),
                _ => false,
            }
        }
        a == b || has_param(a) || has_param(b) || a.is_error() || b.is_error()
    }

    fn collect_implicit_generics(&self, te: &TypeExpr, mi: usize, declared: &[Rc<str>], out: &mut Vec<String>) {
        match &te.kind {
            TypeKind::Named { path, args } => {
                for a in args {
                    if let TypeKind::Named { path: p2, args: a2 } = &a.kind {
                        if p2.len() == 1 && a2.is_empty() {
                            let n = &p2[0].name;
                            if n != "Self" && !declared.iter().any(|d| &**d == n) && !out.contains(n) && self.lookup_type(mi, n).is_none() {
                                out.push(n.clone());
                                continue;
                            }
                        }
                    }
                    self.collect_implicit_generics(a, mi, declared, out);
                }
                let _ = path;
            }
            TypeKind::Ref(t) | TypeKind::Ptr(t) | TypeKind::Optional(t) => self.collect_implicit_generics(t, mi, declared, out),
            TypeKind::Array { elem, .. } => self.collect_implicit_generics(elem, mi, declared, out),
            TypeKind::Tuple(v) | TypeKind::Union(v) => for t in v { self.collect_implicit_generics(t, mi, declared, out); },
            TypeKind::Func { params, ret } => {
                for t in params { self.collect_implicit_generics(t, mi, declared, out); }
                self.collect_implicit_generics(ret, mi, declared, out);
            }
            TypeKind::Prim(_) => {}
        }
    }

    pub(super) fn make_sig(&mut self, f: &FnDecl, mi: usize, outer: &[Rc<str>], self_ty: Option<Ty>, _x: &[()]) -> FnSig {
        let mut gnames: Vec<Rc<str>> = outer.to_vec();
        let own: Vec<Rc<str>> = f.generics.iter().map(|g| Rc::from(g.name.name.as_str())).collect();
        gnames.extend(own.iter().cloned());
        let sc = TyScope { module: mi, generics: gnames, self_ty };
        let generics = self.resolve_generics(&f.generics, &sc);
        let params: Vec<Ty> = f.params.iter().map(|p| self.resolve_type(&p.ty, &sc)).collect();
        let ret = self.resolve_type(&f.ret, &sc);
        let has_body = f.body.is_some();
        let is_std = self.mod_std[mi];
        let is_static = if has_body {
            let mut uses = false;
            f.body.as_ref().unwrap().walk(&mut |e| {
                if matches!(&e.kind, ExprKind::Ident(n) if n == "self") { uses = true; }
            });
            !uses
        } else {
            is_std && f.name.name == "new" || f.is_extern
        };
        FnSig {
            name: f.name.name.clone(),
            module: mi,
            generics,
            params,
            ret,
            variadic: f.variadic,
            is_unsafe: f.is_unsafe || f.is_extern && f.body.is_none(),
            is_async: f.is_async,
            is_pub: f.is_pub || is_std,
            is_extern: f.is_extern,
            has_body,
            is_static,
            span: f.name.span,
        }
    }

    fn push_sig(&mut self, mi: usize, sig: FnSig, register: bool) -> usize {
        let id = self.sigs.len();
        if register {
            let name = sig.name.clone();
            self.mods[mi].fns.entry(name).or_default().push(id);
        }
        self.sigs.push(sig);
        id
    }

    fn check_overload_duplicates(&mut self) {
        for mi in 0..self.mods.len() {
            let names: Vec<(String, Vec<usize>)> = self.mods[mi].fns.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for (name, ids) in names {
                for i in 0..ids.len() {
                    for j in i + 1..ids.len() {
                        let (a, b) = (&self.sigs[ids[i]], &self.sigs[ids[j]]);
                        if a.params == b.params {
                            let span = b.span;
                            let msg = if a.ret != b.ret {
                                format!("overload ambiguo de `{name}`: el tipo de retorno no participa en la resolución de sobrecargas")
                            } else {
                                format!("la función `{name}` está definida más de una vez con los mismos parámetros")
                            };
                            self.err(span, msg);
                        }
                    }
                }
            }
        }
    }

    // ---------- resolución de nombres de tipo ----------

    pub(super) fn lookup_type(&self, module: usize, name: &str) -> Option<String> {
        if let Some(k) = self.mods[module].types.get(name) {
            return Some(k.clone());
        }
        match self.mods[module].imports.get(name) {
            Some(ImportTarget::Item(m, item)) => {
                if let Some(k) = self.mods[*m].types.get(item) {
                    return Some(k.clone());
                }
            }
            _ => {}
        }
        for w in &self.mods[module].wildcards {
            if let Some(k) = self.mods[*w].types.get(name) {
                if self.types.get(k).is_some_and(|d| d.is_pub || self.mod_std[d.module]) {
                    return Some(k.clone());
                }
            }
        }
        if module != 0 {
            if let Some(k) = self.mods[0].types.get(name) {
                return Some(k.clone());
            }
        }
        None
    }

    pub(super) fn find_module(&self, module: usize, name: &str) -> Option<usize> {
        match self.mods[module].imports.get(name) {
            Some(ImportTarget::Module(m)) => Some(*m),
            _ => None,
        }
    }

    pub(super) fn resolve_generics(&mut self, gs: &[GenericParam], sc: &TyScope) -> Vec<GenericInfo> {
        let mut out = Vec::new();
        for g in gs {
            let mut bounds = Vec::new();
            for b in &g.bounds {
                if let Some(tr) = self.resolve_trait_ref(b, sc) {
                    bounds.push(tr);
                }
            }
            out.push(GenericInfo { name: Rc::from(g.name.name.as_str()), bounds });
        }
        out
    }

    pub(super) fn resolve_trait_ref(&mut self, te: &TypeExpr, sc: &TyScope) -> Option<TraitRef> {
        let TypeKind::Named { path, args } = &te.kind else {
            self.err(te.span, "se esperaba un trait");
            return None;
        };
        let name = &path.last().unwrap().name;
        let key = if path.len() == 1 {
            self.lookup_type(sc.module, name)
        } else {
            let mname = path[..path.len() - 1].iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
            self.find_module(sc.module, &mname).and_then(|m| self.mods[m].types.get(name).cloned())
        };
        match key {
            Some(k) if self.types.get(&k).is_some_and(|d| matches!(d.kind, DefKind::Trait)) => {
                let args = args.iter().map(|a| self.resolve_type(a, sc)).collect();
                Some(TraitRef { key: k, args })
            }
            Some(_) => {
                self.err(te.span, format!("`{name}` no es un trait"));
                None
            }
            None => {
                let cands: Vec<String> = self.traits.values().map(|t| t.name.clone()).collect();
                let mut d = Diagnostic::error(te.span, format!("no se encuentra el trait `{name}`"));
                if let Some(s) = self.suggest(name, cands.into_iter()) {
                    d = d.with_note(format!("¿quisiste decir `{s}`?"));
                }
                self.diags.push(d);
                None
            }
        }
    }

    pub(super) fn resolve_type(&mut self, te: &TypeExpr, sc: &TyScope) -> Ty {
        self.resolve_type_depth(te, sc, 0)
    }

    fn resolve_type_depth(&mut self, te: &TypeExpr, sc: &TyScope, depth: u32) -> Ty {
        if depth > 32 {
            self.err(te.span, "alias de tipo recursivo");
            return Ty::Error;
        }
        match &te.kind {
            TypeKind::Prim(p) => match p {
                Prim::Int => Ty::Int,
                Prim::Float => Ty::Float,
                Prim::Bool => Ty::Bool,
                Prim::Char => Ty::Char,
                Prim::Str => Ty::Str,
                Prim::Nothing => Ty::Nothing,
            },
            TypeKind::Ref(t) => Ty::Ref(Box::new(self.resolve_type_depth(t, sc, depth))),
            TypeKind::Ptr(t) => Ty::Ptr(Box::new(self.resolve_type_depth(t, sc, depth))),
            TypeKind::Optional(t) => {
                let inner = self.resolve_type_depth(t, sc, depth);
                Ty::option(inner)
            }
            TypeKind::Union(v) => {
                let mut out: Vec<Ty> = vec![];
                for t in v {
                    let r = self.resolve_type_depth(t, sc, depth);
                    match r {
                        Ty::Union(inner) => for i in inner { if !out.contains(&i) { out.push(i) } },
                        other => if !out.contains(&other) { out.push(other) },
                    }
                }
                if out.len() == 1 { out.pop().unwrap() } else { Ty::Union(out) }
            }
            TypeKind::Func { params, ret } => Ty::Func(
                params.iter().map(|p| self.resolve_type_depth(p, sc, depth)).collect(),
                Box::new(self.resolve_type_depth(ret, sc, depth)),
            ),
            TypeKind::Tuple(v) => Ty::Tuple(v.iter().map(|t| self.resolve_type_depth(t, sc, depth)).collect()),
            TypeKind::Array { elem, len } => Ty::Array(Box::new(self.resolve_type_depth(elem, sc, depth)), *len),
            TypeKind::Named { path, args } => {
                let last = path.last().unwrap();
                if path.len() == 1 {
                    if sc.generics.iter().any(|g| **g == *last.name) {
                        if !args.is_empty() {
                            self.err(te.span, format!("el parámetro genérico `{}` no admite argumentos", last.name));
                        }
                        return Ty::Param(Rc::from(last.name.as_str()));
                    }
                    if last.name == "Self" {
                        return match &sc.self_ty {
                            Some(t) => t.clone(),
                            None => {
                                self.err(te.span, "`Self` solo puede usarse dentro de `impl` o `trait`");
                                Ty::Error
                            }
                        };
                    }
                }
                let key = if path.len() == 1 {
                    self.lookup_type(sc.module, &last.name)
                } else {
                    let mname = path[..path.len() - 1].iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
                    let m = self.find_module(sc.module, &mname);
                    match m {
                        Some(m) => self.mods[m].types.get(&last.name).cloned(),
                        None => {
                            self.err(path[0].span, format!("no se encuentra el módulo `{mname}`"));
                            return Ty::Error;
                        }
                    }
                };
                let Some(key) = key else {
                    let cands: Vec<String> = self.mods[sc.module].types.keys().cloned().chain(self.mods[0].types.keys().cloned()).collect();
                    let mut d = Diagnostic::error(last.span, format!("no se encuentra el tipo `{}`", last.name)).with_label("tipo desconocido");
                    if let Some(s) = self.suggest(&last.name, cands.into_iter()) {
                        d = d.with_note(format!("¿quisiste decir `{s}`?"));
                    }
                    if last.name.chars().next().is_some_and(|c| c.is_lowercase()) {
                        d = d.with_note("los tipos primitivos llevan `$` (p. ej. `$int`); los tipos de usuario no");
                    }
                    self.diags.push(d);
                    return Ty::Error;
                };
                let def = self.types[&key].clone();
                if def.module != sc.module && !def.is_pub && !self.mod_std[def.module] {
                    self.err(last.span, format!("el tipo `{}` es privado en su módulo", def.name));
                }
                let targs: Vec<Ty> = args.iter().map(|a| self.resolve_type_depth(a, sc, depth)).collect();
                if matches!(def.kind, DefKind::Trait) {
                    self.err_label(te.span, format!("`{}` es un trait, no un tipo", def.name), "no se puede usar como tipo")
                        ;
                    self.diags.last_mut().unwrap().notes.push(format!("usa un genérico con cota: `<T: {}>`", def.name));
                    return Ty::Error;
                }
                if targs.len() != def.generics.len() {
                    self.err(
                        te.span,
                        format!("el tipo `{}` espera {} argumento(s) genérico(s), pero se dieron {}", def.name, def.generics.len(), targs.len()),
                    );
                    return Ty::Error;
                }
                if matches!(def.kind, DefKind::Alias) {
                    let (am, gnames, inner) = self.alias_defs[&key].clone();
                    let asc = TyScope { module: am, generics: gnames.iter().map(|g| Rc::from(g.as_str())).collect(), self_ty: None };
                    let resolved = self.resolve_type_depth(&inner, &asc, depth + 1);
                    let map: HashMap<Rc<str>, Ty> = gnames.iter().map(|g| Rc::from(g.as_str())).zip(targs).collect();
                    return resolved.subst(&map);
                }
                Ty::Named(Rc::from(key.as_str()), targs)
            }
        }
    }
}
