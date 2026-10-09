//! Evaluación de expresiones, lugares, patrones, operadores y conversiones.

use super::*;
use std::cmp::Ordering;

impl Interp {
    pub fn eval(&mut self, e: &'static Expr, env: &Rc<Scope>) -> R<Value> {
        match &e.kind {
            ExprKind::Int(i) => Ok(Value::Int(*i)),
            ExprKind::Float(f) => Ok(Value::Float(*f)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Char(c) => Ok(Value::Char(*c)),
            ExprKind::Null => Ok(self.none()),
            ExprKind::Str(parts) => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        StrPiece::Lit(s) => out.push_str(s),
                        StrPiece::Expr(x) => {
                            let v = self.eval(x, env)?;
                            out.push_str(&self.display(&v)?);
                        }
                    }
                }
                Ok(Value::str(out))
            }
            ExprKind::Ident(name) => match env.lookup(name) {
                Some(c) => Ok(c.borrow().clone()),
                None => self.panic(format!("variable `{name}` no encontrada (error interno)")),
            },
            ExprKind::Global { module, name } => {
                if let Some(defs) = self.modules[*module].fns.get(name) {
                    return Ok(Value::Func(Rc::new(FuncVal::Defs(defs.clone()))));
                }
                self.const_value(*module, name)
            }
            ExprKind::TypeStatic { key, name } => match self.lookup_def_set(key, name) {
                Some(defs) => Ok(Value::Func(Rc::new(FuncVal::Defs(defs)))),
                None => self.panic(format!("`{key}.{name}` no existe")),
            },
            ExprKind::VariantRef { enum_key, variant } => {
                let info = self.enums.get(enum_key).cloned().unwrap();
                let idx = info.index(variant).unwrap();
                if info.variants[idx].1 == 0 {
                    Ok(Value::Variant(Rc::new(VariantVal { info, idx, vals: vec![] })))
                } else {
                    Ok(Value::Func(Rc::new(FuncVal::Ctor { info, idx })))
                }
            }
            ExprKind::SomeWrap(x) => {
                let v = self.eval(x, env)?;
                Ok(self.some(v))
            }
            ExprKind::Unary(op, x) => self.eval_unary(*op, x, env),
            ExprKind::Binary(op, l, r) => {
                match op {
                    BinOp::And => {
                        let a = self.eval(l, env)?;
                        if let Value::Bool(false) = a { return Ok(Value::Bool(false)); }
                        return self.eval(r, env);
                    }
                    BinOp::Or => {
                        let a = self.eval(l, env)?;
                        if let Value::Bool(true) = a { return Ok(Value::Bool(true)); }
                        return self.eval(r, env);
                    }
                    _ => {}
                }
                let a = self.eval(l, env)?;
                let b = self.eval(r, env)?;
                self.binop(*op, &a, &b)
            }
            ExprKind::Assign(op, l, r) => {
                let rv = self.eval(r, env)?;
                let place = self.expr_place(l, env)?;
                match op {
                    None => {
                        if let ExprKind::Index(base, idx) = &l.kind {
                            // `m[k] = v` inserta en mapas
                            let bp = self.expr_place(base, env)?;
                            let bp = self.follow_place(bp)?;
                            if let Value::Map(_) = bp.get().map_err(Ctl::Panic)? {
                                let iv = self.eval(idx, env)?;
                                return self.map_store(&bp, iv, rv).map(|_| Value::Nothing);
                            }
                        }
                        self.pr(place.set(rv))?;
                    }
                    Some(bop) => {
                        let cur = self.pr(place.get())?;
                        let nv = self.binop(*bop, &cur, &rv)?;
                        self.pr(place.set(nv))?;
                    }
                }
                Ok(Value::Nothing)
            }
            ExprKind::IncDec { target, inc, prefix } => {
                let place = self.expr_place(target, env)?;
                let cur = self.pr(place.get())?;
                let cur = self.deref_ref_keep(&cur, &place)?;
                let delta = if *inc { 1 } else { -1 };
                let nv = match &cur {
                    Value::Int(i) => Value::Int(i.checked_add(delta).ok_or_else(|| Ctl::Panic("desbordamiento aritmético".into()))?),
                    Value::Float(f) => Value::Float(f + delta as f64),
                    _ => return self.panic("`++`/`--` requieren un número"),
                };
                self.pr(place.set(nv.clone()))?;
                Ok(if *prefix { nv } else { cur })
            }
            ExprKind::Ternary(c, a, b) => {
                let cv = self.eval(c, env)?;
                if matches!(cv, Value::Bool(true)) { self.eval(a, env) } else { self.eval(b, env) }
            }
            ExprKind::Call { callee, args } => self.eval_call(e, callee, args, env),
            ExprKind::Field(base, name) => {
                let b = self.eval(base, env)?;
                self.get_field(&b, &name.name)
            }
            ExprKind::Index(base, idx) => {
                let b = self.eval(base, env)?;
                let i = self.eval(idx, env)?;
                self.index_value(&b, &i)
            }
            ExprKind::Slice { base, lo, hi } => {
                let b = self.eval(base, env)?;
                let b = self.deref_value(&b)?;
                let lo = match lo { Some(x) => Some(self.eval(x, env)?.as_int()), None => None };
                let hi = match hi { Some(x) => Some(self.eval(x, env)?.as_int()), None => None };
                self.slice_value(&b, lo, hi)
            }
            ExprKind::Array(items) => {
                let mut v = Vec::with_capacity(items.len());
                for i in items { v.push(self.eval(i, env)?); }
                Ok(Value::Array(Rc::new(v)))
            }
            ExprKind::Tuple(items) => {
                let mut v = Vec::with_capacity(items.len());
                for i in items { v.push(self.eval(i, env)?); }
                Ok(Value::Tuple(Rc::new(v)))
            }
            ExprKind::Map(entries) => {
                let mut m = OMap::default();
                for (k, v) in entries {
                    let kv = self.eval(k, env)?;
                    let vv = self.eval(v, env)?;
                    self.check_key(&kv)?;
                    m.insert(kv, vv);
                }
                Ok(Value::Map(Rc::new(m)))
            }
            ExprKind::Set(items) => {
                let mut m = OMap::default();
                for k in items {
                    let kv = self.eval(k, env)?;
                    self.check_key(&kv)?;
                    m.insert(kv, Value::Nothing);
                }
                Ok(Value::Set(Rc::new(m)))
            }
            ExprKind::StructLit { fields, key, .. } => {
                let info = self.structs.get(key).cloned().unwrap();
                let mut vals = vec![Value::Nothing; info.fields.len()];
                for (name, ex) in fields {
                    let v = self.eval(ex, env)?;
                    let idx = info.index(&name.name).unwrap();
                    vals[idx] = v;
                }
                Ok(Value::Struct(Rc::new(StructVal { info, vals })))
            }
            ExprKind::If { cond, then, els } => {
                let c = self.eval(cond, env)?;
                if matches!(c, Value::Bool(true)) {
                    self.eval_block(then, env)
                } else if let Some(e) = els {
                    self.eval(e, env)
                } else {
                    Ok(Value::Nothing)
                }
            }
            ExprKind::Match { scrut, arms } => {
                let v = self.eval(scrut, env)?;
                for arm in arms {
                    let scope = Scope::new(Some(env.clone()));
                    if self.match_pattern(&arm.pattern, &v, &scope)? {
                        if let Some(g) = &arm.guard {
                            let gv = self.eval(g, &scope)?;
                            if !matches!(gv, Value::Bool(true)) {
                                continue;
                            }
                        }
                        let r = self.eval(&arm.body, &scope);
                        if self.has_drop {
                            self.drop_scope(&scope)?;
                        }
                        return r;
                    }
                }
                self.panic("ningún patrón del `match` coincide (error interno: match no exhaustivo)")
            }
            ExprKind::Closure { params, body, .. } => Ok(Value::Func(Rc::new(FuncVal::Closure(Rc::new(ClosureDef { params, body, env: env.clone(), module: self.cur_module }))))),
            ExprKind::Block(b) | ExprKind::Unsafe(b) => self.eval_block(b, env),
            ExprKind::Cast { expr, ty, lossy } => {
                let v = self.eval(expr, env)?;
                let v = self.deref_ref(&v)?;
                self.cast(v, ty, *lossy)
            }
            ExprKind::Try(inner) => {
                let v = self.eval(inner, env)?;
                let v = self.deref_ref(&v)?;
                match &v {
                    Value::Variant(vv) if vv.info.key == "Result" => {
                        if vv.idx == 0 { Ok(vv.vals[0].clone()) } else { Err(Ctl::Return(v.clone())) }
                    }
                    Value::Variant(vv) if vv.info.key == "Option" => {
                        if vv.idx == 0 { Ok(vv.vals[0].clone()) } else { Err(Ctl::Return(v.clone())) }
                    }
                    _ => self.panic("`?` aplicado a un valor que no es Result ni Option"),
                }
            }
            ExprKind::Await(inner) => {
                let v = self.eval(inner, env)?;
                let v = self.deref_value(&v)?;
                match &v {
                    Value::Future(f) => self.run_future(f),
                    _ => self.panic("`await` sobre un valor que no es un Future"),
                }
            }
            ExprKind::Return(v) => {
                let val = match v {
                    Some(x) => self.eval(x, env)?,
                    None => Value::Nothing,
                };
                Err(Ctl::Return(val))
            }
            ExprKind::Break => Err(Ctl::Break),
            ExprKind::While { cond, body } => {
                loop {
                    let c = self.eval(cond, env)?;
                    if !matches!(c, Value::Bool(true)) { break; }
                    match self.eval_block(body, env) {
                        Ok(_) => {}
                        Err(Ctl::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Nothing)
            }
            ExprKind::Loop { kind, body } => self.eval_loop(kind, body, env),
            ExprKind::Asm(_) => self.panic("el ensamblador en línea no está soportado por el backend intérprete"),
        }
    }

    fn check_key(&self, k: &Value) -> R<()> {
        if k.to_key().is_none() {
            return self.panic(format!("el valor de tipo `{}` no puede usarse como clave", k.type_key()));
        }
        Ok(())
    }

    fn eval_loop(&mut self, kind: &'static LoopKind, body: &'static Block, env: &Rc<Scope>) -> R<Value> {
        match kind {
            LoopKind::C { init, cond, update } => {
                let scope = Scope::new(Some(env.clone()));
                if let Some(s) = init {
                    self.exec_stmt(s, &scope)?;
                }
                loop {
                    if let Some(c) = cond {
                        let cv = self.eval(c, &scope)?;
                        if !matches!(cv, Value::Bool(true)) { break; }
                    }
                    match self.eval_block(body, &scope) {
                        Ok(_) => {}
                        Err(Ctl::Break) => break,
                        Err(e) => return Err(e),
                    }
                    if let Some(u) = update {
                        self.eval(u, &scope)?;
                    }
                }
            }
            LoopKind::Each { var, iter, .. } => {
                let it = self.eval(iter, env)?;
                let items = self.iter_items(&it)?;
                for item in items {
                    let scope = Scope::new(Some(env.clone()));
                    scope.declare(&var.name, item);
                    let r = self.eval_block_in(body, &scope);
                    if self.has_drop {
                        self.drop_scope(&scope)?;
                    }
                    match r {
                        Ok(_) => {}
                        Err(Ctl::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(Value::Nothing)
    }

    pub fn iter_items(&mut self, v: &Value) -> R<Vec<Value>> {
        let v = self.deref_value(v)?;
        Ok(match &v {
            Value::Array(a) | Value::List(a) => (**a).clone(),
            Value::Set(s) => s.entries.iter().map(|(k, _)| k.clone()).collect(),
            Value::Map(m) => m.entries.iter().map(|(k, v)| Value::Tuple(Rc::new(vec![k.clone(), v.clone()]))).collect(),
            Value::Str(s) => s.chars().map(Value::Char).collect(),
            Value::Struct(_) | Value::Variant(_) => {
                let key = v.type_key();
                match self.lookup_def_set(&key, "items") {
                    Some(defs) => {
                        let r = self.call_def(&defs[0].clone(), vec![], Some(Place::temp(v.clone())), Span::default())?;
                        match self.deref_value(&r)? {
                            Value::List(l) => (*l).clone(),
                            _ => return self.panic("`items` debe devolver una List"),
                        }
                    }
                    None => return self.panic(format!("`{key}` no es iterable")),
                }
            }
            _ => return self.panic(format!("`{}` no es iterable", v.type_key())),
        })
    }

    // ---------- lugares ----------

    /// Evalúa `e` como lugar modificable (o crea un temporal si no lo es).
    pub fn expr_place(&mut self, e: &'static Expr, env: &Rc<Scope>) -> R<Place> {
        match &e.kind {
            ExprKind::Ident(name) => match env.lookup(name) {
                Some(c) => Ok(Place { cell: c, path: vec![] }),
                None => self.panic(format!("variable `{name}` no encontrada")),
            },
            ExprKind::Global { module, name } if self.modules[*module].consts.contains_key(name) => self.const_place(*module, name),
            ExprKind::Field(base, name) => {
                let bp = self.expr_place(base, env)?;
                let bp = self.follow_place(bp)?;
                let v = self.pr(bp.get())?;
                let v = self.deref_ref(&v)?;
                match &v {
                    Value::Struct(s) => match s.info.index(&name.name) {
                        Some(i) => {
                            let mut p = bp.clone();
                            p.path.push(Seg::Field(i));
                            Ok(p)
                        }
                        None => self.panic(format!("el campo `{}` no existe", name.name)),
                    },
                    _ => self.panic(format!("acceso al campo `{}` sobre `{}`", name.name, v.type_key())),
                }
            }
            ExprKind::Index(base, idx) => {
                let bp = self.expr_place(base, env)?;
                let bp = self.follow_place(bp)?;
                let iv = self.eval(idx, env)?;
                let bv = self.pr(bp.get())?;
                let mut p = bp.clone();
                match &bv {
                    Value::Array(a) | Value::List(a) | Value::Tuple(a) => {
                        let i = iv.as_int();
                        if i < 0 || i as usize >= a.len() {
                            return self.panic(format!("índice {i} fuera de rango (longitud {})", a.len()));
                        }
                        p.path.push(Seg::Index(i as usize));
                    }
                    Value::Map(_) => match iv.to_key() {
                        Some(k) => p.path.push(Seg::Key(k)),
                        None => return self.panic("clave de mapa no válida"),
                    },
                    other => return self.panic(format!("no se puede indexar un `{}`", other.type_key())),
                }
                Ok(p)
            }
            ExprKind::Unary(UnOp::Deref, inner) => {
                let v = self.eval(inner, env)?;
                self.deref_place(&v)
            }
            _ => {
                let v = self.eval(e, env)?;
                Ok(Place::temp(v))
            }
        }
    }

    pub fn deref_place(&mut self, v: &Value) -> R<Place> {
        match v {
            Value::Ref(r) => Ok(Place { cell: r.cell.clone(), path: r.path.clone() }),
            Value::Box(c) | Value::Rc(c) => Ok(Place { cell: c.clone(), path: vec![] }),
            Value::Ptr(addr, off) => self.heap_place(*addr, *off),
            other => self.panic(format!("no se puede desreferenciar un `{}`", other.type_key())),
        }
    }

    /// Si el lugar contiene una referencia, apunta a su destino (repetidamente).
    pub fn follow_place(&mut self, mut p: Place) -> R<Place> {
        for _ in 0..64 {
            let v = self.pr(p.get())?;
            match v {
                Value::Ref(r) => p = Place { cell: r.cell.clone(), path: r.path.clone() },
                _ => return Ok(p),
            }
        }
        Ok(p)
    }

    fn deref_ref_keep(&self, v: &Value, _p: &Place) -> R<Value> {
        Ok(v.clone())
    }

    fn map_store(&mut self, mp: &Place, k: Value, v: Value) -> R<()> {
        self.check_key(&k)?;
        let mut slot = Some((k, v));
        self.pr(mp.with_mut(|dst| {
            if let Value::Map(m) = dst {
                let (k, v) = slot.take().unwrap();
                Rc::make_mut(m).insert(k, v);
            }
            Ok(())
        }))
    }

    pub fn get_field(&mut self, base: &Value, name: &str) -> R<Value> {
        let v = self.deref_value(base)?;
        match &v {
            Value::Struct(s) => match s.info.index(name) {
                Some(i) => Ok(s.vals[i].clone()),
                None => self.panic(format!("el campo `{name}` no existe")),
            },
            other => self.panic(format!("acceso al campo `{name}` sobre `{}`", other.type_key())),
        }
    }

    pub fn index_value(&mut self, base: &Value, idx: &Value) -> R<Value> {
        let b = self.deref_value(base)?;
        match &b {
            Value::Array(a) | Value::List(a) | Value::Tuple(a) => {
                let i = idx.as_int();
                if i < 0 || i as usize >= a.len() {
                    return self.panic(format!("índice {i} fuera de rango (longitud {})", a.len()));
                }
                Ok(a[i as usize].clone())
            }
            Value::Str(s) => {
                let i = idx.as_int();
                match (i >= 0).then(|| s.chars().nth(i as usize)).flatten() {
                    Some(c) => Ok(Value::Char(c)),
                    None => self.panic(format!("índice {i} fuera de rango en el string")),
                }
            }
            Value::Map(m) => match m.get(idx) {
                Some(v) => Ok(v.clone()),
                None => {
                    let k = self.debug_str(idx)?;
                    self.panic(format!("la clave {k} no existe en el mapa"))
                }
            },
            Value::Ptr(addr, off) => {
                let i = idx.as_int();
                let p = self.heap_place(*addr, off + i)?;
                self.pr(p.get())
            }
            other => self.panic(format!("no se puede indexar un `{}`", other.type_key())),
        }
    }

    fn slice_value(&mut self, b: &Value, lo: Option<i64>, hi: Option<i64>) -> R<Value> {
        let len = match b {
            Value::Array(a) | Value::List(a) => a.len(),
            Value::Str(s) => s.chars().count(),
            other => return self.panic(format!("no se puede tomar un slice de `{}`", other.type_key())),
        } as i64;
        let (l, h) = (lo.unwrap_or(0), hi.unwrap_or(len));
        if l < 0 || h > len || l > h {
            return self.panic(format!("slice [{l}..{h}] fuera de rango (longitud {len})"));
        }
        Ok(match b {
            Value::Array(a) | Value::List(a) => Value::List(Rc::new(a[l as usize..h as usize].to_vec())),
            Value::Str(s) => Value::str(s.chars().skip(l as usize).take((h - l) as usize).collect::<String>()),
            _ => unreachable!(),
        })
    }

    // ---------- llamadas ----------

    fn eval_call(&mut self, e: &'static Expr, callee: &'static Expr, args: &'static [Expr], env: &Rc<Scope>) -> R<Value> {
        let span = e.span;
        // llamadas especiales
        if let ExprKind::Ident(n) = &callee.kind {
            if env.lookup(n).is_none() {
                match n.as_str() {
                    "panic" => {
                        let msg = match args.first() {
                            Some(a) => {
                                let v = self.eval(a, env)?;
                                self.display(&v)?
                            }
                            None => "panic explícito".to_string(),
                        };
                        return self.panic(msg);
                    }
                    "exit" => {
                        let v = self.eval(&args[0], env)?;
                        return Err(Ctl::Exit(v.as_int() as i32));
                    }
                    "assert" => {
                        let c = self.eval(&args[0], env)?;
                        if !matches!(c, Value::Bool(true)) {
                            let msg = match args.get(1) {
                                Some(a) => {
                                    let v = self.eval(a, env)?;
                                    self.display(&v)?
                                }
                                None => "falló la aserción".to_string(),
                            };
                            return self.panic(format!("aserción fallida: {msg}"));
                        }
                        return Ok(Value::Nothing);
                    }
                    _ => {}
                }
            }
        }
        match &callee.kind {
            ExprKind::Field(base, name) => self.eval_method_call(base, name, args, env, span),
            ExprKind::Global { module, name } if self.modules[*module].fns.contains_key(name) => {
                let defs = self.modules[*module].fns[name].clone();
                let argv = self.eval_args(args, env)?;
                let def = self.select_def(&defs, &argv)?;
                self.call_def(&def, argv, None, span)
            }
            _ => {
                let f = self.eval(callee, env)?;
                let argv = self.eval_args(args, env)?;
                self.call_value(&f, argv, span)
            }
        }
    }

    fn eval_args(&mut self, args: &'static [Expr], env: &Rc<Scope>) -> R<Vec<Value>> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            out.push(self.eval(a, env)?);
        }
        Ok(out)
    }

    fn eval_method_call(&mut self, base: &'static Expr, name: &'static Ident, args: &'static [Expr], env: &Rc<Scope>, span: Span) -> R<Value> {
        let mut place = self.expr_place(base, env)?;
        let argv = self.eval_args(args, env)?;
        for _ in 0..16 {
            let v = self.pr(place.get())?;
            let key = v.type_key();
            if let Some(defs) = self.lookup_def_set(&key, &name.name) {
                let def = self.select_def(&defs, &argv)?;
                return self.call_def(&def, argv, Some(place), span);
            }
            match &v {
                Value::Ref(r) => place = Place { cell: r.cell.clone(), path: r.path.clone() },
                Value::Box(c) | Value::Rc(c) => place = Place { cell: c.clone(), path: vec![] },
                Value::Struct(s) => {
                    if let Some(i) = s.info.index(&name.name) {
                        let f = s.vals[i].clone();
                        return self.call_value(&f, argv, span);
                    }
                    break;
                }
                _ => break,
            }
        }
        let v = self.pr(place.get())?;
        self.panic(format!("el tipo `{}` no tiene el método `{}`", v.type_key(), name.name))
    }

    // ---------- patrones ----------

    pub fn match_pattern(&mut self, p: &'static Pattern, v: &Value, scope: &Rc<Scope>) -> R<bool> {
        match p {
            Pattern::Wild(_) => Ok(true),
            Pattern::Bind { name, ty } => {
                if let Some(te) = ty {
                    let d = self.deref_ref(v)?;
                    let tag = tag_of(te, &[]);
                    if !self.tag_matches(&tag, &d) {
                        return Ok(false);
                    }
                    scope.declare(&name.name, d);
                } else {
                    scope.declare(&name.name, v.clone());
                }
                Ok(true)
            }
            Pattern::Lit(e) => {
                let lit = self.eval(e, scope)?;
                let d = self.deref_ref(v)?;
                Ok(self.values_equal(&lit, &d))
            }
            Pattern::Tuple(items, _) => {
                let d = self.deref_ref(v)?;
                let Value::Tuple(vals) = &d else { return Ok(false) };
                if vals.len() != items.len() { return Ok(false); }
                for (sp, x) in items.iter().zip(vals.iter()) {
                    if !self.match_pattern(sp, x, scope)? { return Ok(false); }
                }
                Ok(true)
            }
            Pattern::Variant { res, args, .. } => {
                let d = self.deref_ref(v)?;
                let Value::Variant(vv) = &d else { return Ok(false) };
                let (ekey, vname) = res.as_ref().unwrap();
                if vv.info.key != *ekey || vv.info.variants[vv.idx].0 != *vname {
                    return Ok(false);
                }
                if let Some(subs) = args {
                    for (sp, x) in subs.iter().zip(vv.vals.iter()) {
                        if !self.match_pattern(sp, x, scope)? { return Ok(false); }
                    }
                }
                Ok(true)
            }
        }
    }

    // ---------- operadores ----------

    fn eval_unary(&mut self, op: UnOp, x: &'static Expr, env: &Rc<Scope>) -> R<Value> {
        match op {
            UnOp::Ref => {
                let p = self.expr_place(x, env)?;
                Ok(p.to_ref())
            }
            UnOp::Deref => {
                let v = self.eval(x, env)?;
                match &v {
                    Value::Ref(r) => self.pr(read_path(&r.cell, &r.path)),
                    Value::Box(c) | Value::Rc(c) => Ok(c.borrow().clone()),
                    Value::Ptr(addr, off) => {
                        let p = self.heap_place(*addr, *off)?;
                        self.pr(p.get())
                    }
                    other => self.panic(format!("no se puede desreferenciar un `{}`", other.type_key())),
                }
            }
            UnOp::Neg => {
                let v = self.eval(x, env)?;
                match v {
                    Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(|| Ctl::Panic("desbordamiento aritmético".into())),
                    Value::Float(f) => Ok(Value::Float(-f)),
                    _ => self.panic("negación de un valor no numérico"),
                }
            }
            UnOp::Not => {
                let v = self.eval(x, env)?;
                Ok(Value::Bool(!matches!(v, Value::Bool(true))))
            }
            UnOp::BitNot => {
                let v = self.eval(x, env)?;
                Ok(Value::Int(!v.as_int()))
            }
        }
    }

    pub fn binop(&mut self, op: BinOp, a: &Value, b: &Value) -> R<Value> {
        use BinOp::*;
        let a = &self.deref_ref(a)?;
        let b = &self.deref_ref(b)?;
        match op {
            Eq => return Ok(Value::Bool(self.values_equal(a, b))),
            Ne => return Ok(Value::Bool(!self.values_equal(a, b))),
            RefEq => return Ok(Value::Bool(self.ref_equal(a, b))),
            Lt | Gt | Le | Ge => {
                let ord = self.compare_values(a, b)?;
                let r = match (op, ord) {
                    (_, None) => false,
                    (Lt, Some(o)) => o == Ordering::Less,
                    (Gt, Some(o)) => o == Ordering::Greater,
                    (Le, Some(o)) => o != Ordering::Greater,
                    (_, Some(o)) => o != Ordering::Less,
                };
                return Ok(Value::Bool(r));
            }
            _ => {}
        }
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => {
                let (x, y) = (*x, *y);
                let ovf = || Ctl::Panic("desbordamiento aritmético".into());
                Ok(Value::Int(match op {
                    Add => x.checked_add(y).ok_or_else(ovf)?,
                    Sub => x.checked_sub(y).ok_or_else(ovf)?,
                    Mul => x.checked_mul(y).ok_or_else(ovf)?,
                    Div => {
                        if y == 0 { return self.panic("división entre cero"); }
                        x.checked_div(y).ok_or_else(ovf)?
                    }
                    Rem => {
                        if y == 0 { return self.panic("división entre cero (resto)"); }
                        x.checked_rem(y).ok_or_else(ovf)?
                    }
                    Pow => {
                        if y < 0 { return self.panic("exponente negativo en una potencia entera"); }
                        let mut r: i64 = 1;
                        let mut base = x;
                        let mut e = y;
                        while e > 0 {
                            if e & 1 == 1 { r = r.checked_mul(base).ok_or_else(ovf)?; }
                            e >>= 1;
                            if e > 0 { base = base.checked_mul(base).ok_or_else(ovf)?; }
                        }
                        r
                    }
                    BitAnd => x & y,
                    BitOr => x | y,
                    BitXor => x ^ y,
                    Shl => {
                        if !(0..64).contains(&y) { return self.panic("desplazamiento fuera de rango"); }
                        x << y
                    }
                    Shr => {
                        if !(0..64).contains(&y) { return self.panic("desplazamiento fuera de rango"); }
                        x >> y
                    }
                    _ => return self.panic("operador no válido para enteros"),
                }))
            }
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(match op {
                Add => x + y,
                Sub => x - y,
                Mul => x * y,
                Div => x / y,
                Rem => x % y,
                Pow => x.powf(*y),
                _ => return self.panic("operador no válido para decimales"),
            })),
            (Value::Str(x), Value::Str(y)) if op == Add => Ok(Value::str(format!("{x}{y}"))),
            (Value::Str(x), Value::Char(y)) if op == Add => Ok(Value::str(format!("{x}{y}"))),
            (Value::Ptr(addr, off), Value::Int(n)) => match op {
                Add => Ok(Value::Ptr(*addr, off + n)),
                Sub => Ok(Value::Ptr(*addr, off - n)),
                _ => self.panic("operador no válido para punteros"),
            },
            (Value::Bool(x), Value::Bool(y)) => Ok(Value::Bool(match op {
                BitAnd => x & y,
                BitOr => x | y,
                BitXor => x ^ y,
                _ => return self.panic("operador no válido para booleanos"),
            })),
            _ => self.panic(format!("operador `{}` no aplicable a `{}` y `{}`", op.symbol(), a.type_key(), b.type_key())),
        }
    }

    pub fn ref_equal(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Box(x), Value::Box(y)) | (Value::Rc(x), Value::Rc(y)) => Rc::ptr_eq(x, y),
            (Value::Future(x), Value::Future(y)) => Rc::ptr_eq(x, y),
            (Value::Ptr(a1, o1), Value::Ptr(a2, o2)) => a1 == a2 && o1 == o2,
            _ => false,
        }
    }

    pub fn values_equal(&self, a: &Value, b: &Value) -> bool {
        let (a, b) = match (self.deref_ref(a), self.deref_ref(b)) {
            (Ok(a), Ok(b)) => (a, b),
            _ => return false,
        };
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Float(x), Value::Float(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Char(x), Value::Char(y)) => x == y,
            (Value::Str(x), Value::Str(y)) => x == y,
            (Value::Nothing, Value::Nothing) => true,
            (Value::Array(x), Value::Array(y)) | (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| self.values_equal(p, q))
            }
            (Value::Map(x), Value::Map(y)) => {
                x.len() == y.len() && x.entries.iter().all(|(k, v)| y.get(k).is_some_and(|w| self.values_equal(v, w)))
            }
            (Value::Set(x), Value::Set(y)) => x.len() == y.len() && x.entries.iter().all(|(k, _)| y.contains(k)),
            (Value::Struct(x), Value::Struct(y)) => {
                x.info.key == y.info.key && x.vals.iter().zip(y.vals.iter()).all(|(p, q)| self.values_equal(p, q))
            }
            (Value::Variant(x), Value::Variant(y)) => {
                x.info.key == y.info.key && x.idx == y.idx && x.vals.iter().zip(y.vals.iter()).all(|(p, q)| self.values_equal(p, q))
            }
            (Value::Box(x), Value::Box(y)) | (Value::Rc(x), Value::Rc(y)) => Rc::ptr_eq(x, y) || self.values_equal(&x.borrow(), &y.borrow()),
            (Value::Ptr(a1, o1), Value::Ptr(a2, o2)) => a1 == a2 && o1 == o2,
            (Value::Func(x), Value::Func(y)) => Rc::ptr_eq(x, y),
            _ => false,
        }
    }

    pub fn compare_values(&mut self, a: &Value, b: &Value) -> R<Option<Ordering>> {
        Ok(match (a, b) {
            (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
            (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
            (Value::Char(x), Value::Char(y)) => Some(x.cmp(y)),
            (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
            (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
            (Value::Ptr(a1, o1), Value::Ptr(a2, o2)) => Some((a1, o1).cmp(&(a2, o2))),
            (Value::Tuple(x), Value::Tuple(y)) => {
                for (p, q) in x.iter().zip(y.iter()) {
                    match self.compare_values(p, q)? {
                        Some(Ordering::Equal) => {}
                        other => return Ok(other),
                    }
                }
                Some(x.len().cmp(&y.len()))
            }
            (Value::Struct(_), _) | (Value::Variant(_), _) => {
                let key = a.type_key();
                match self.lookup_def_set(&key, "compare") {
                    Some(defs) => {
                        let r = self.call_def(&defs[0].clone(), vec![b.clone()], Some(Place::temp(a.clone())), Span::default())?;
                        Some(r.as_int().cmp(&0))
                    }
                    None => return self.panic(format!("`{key}` no implementa Comparable")),
                }
            }
            _ => return self.panic(format!("no se pueden ordenar `{}` y `{}`", a.type_key(), b.type_key())),
        })
    }

    fn cast(&mut self, v: Value, ty: &'static TypeExpr, lossy: bool) -> R<Value> {
        match &ty.kind {
            TypeKind::Prim(Prim::Float) => match v {
                Value::Int(i) => Ok(Value::Float(i as f64)),
                other => Ok(other),
            },
            TypeKind::Prim(Prim::Int) => match v {
                Value::Float(f) => Ok(Value::Int(f as i64)),
                Value::Char(c) => Ok(Value::Int(c as i64)),
                Value::Bool(b) => Ok(Value::Int(b as i64)),
                Value::Ptr(a, o) => Ok(Value::Int((a as i64) << 20 | (o & 0xFFFFF))),
                other => Ok(other),
            },
            TypeKind::Prim(Prim::Char) => match v {
                Value::Int(i) => match u32::try_from(i).ok().and_then(char::from_u32) {
                    Some(c) => Ok(Value::Char(c)),
                    None => self.panic(format!("{i} no es un carácter Unicode válido")),
                },
                other => Ok(other),
            },
            TypeKind::Prim(Prim::Bool) => match v {
                Value::Int(i) => Ok(Value::Bool(i != 0)),
                other => Ok(other),
            },
            TypeKind::Ptr(_) => match v {
                Value::Int(i) => Ok(Value::Ptr((i >> 20) as usize, i & 0xFFFFF)),
                other => Ok(other),
            },
            _ => {
                let _ = lossy;
                Ok(v)
            }
        }
    }
}
