//! Funciones intrínsecas: la biblioteca estándar implementada por el runtime.

use super::*;
use std::cmp::Ordering;
use std::io::{BufRead, Read};

type Args = Vec<Value>;

fn ps(it: &Interp, r: PResult<Value>) -> R<Value> {
    r.map_err(|m| {
        let _ = it;
        Ctl::Panic(m)
    })
}

fn this(r: &Option<Place>) -> R<Place> {
    r.clone().ok_or_else(|| Ctl::Panic("método llamado sin receptor (error interno)".into()))
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Int(i) => *i as f64,
        Value::Float(f) => *f,
        _ => f64::NAN,
    }
}

fn s_of(v: &Value) -> Rc<str> {
    match v {
        Value::Str(s) => s.clone(),
        _ => Rc::from(""),
    }
}

fn list_of(v: &Value) -> Rc<Vec<Value>> {
    match v {
        Value::List(l) | Value::Array(l) => l.clone(),
        _ => Rc::new(vec![]),
    }
}

/// Aplica `f` al vector de la lista receptora (copy-on-write).
fn with_list<T>(it: &Interp, p: &Place, mut f: impl FnMut(&mut Vec<Value>) -> PResult<T>) -> R<T> {
    let _ = it;
    p.with_mut(|v| match v {
        Value::List(rc) => f(Rc::make_mut(rc)),
        _ => Err("el receptor no es una List".into()),
    })
    .map_err(Ctl::Panic)
}

fn with_omap<T>(p: &Place, mut f: impl FnMut(&mut OMap) -> PResult<T>) -> R<T> {
    p.with_mut(|v| match v {
        Value::Map(rc) | Value::Set(rc) => f(Rc::make_mut(rc)),
        _ => Err("el receptor no es un Map/Set".into()),
    })
    .map_err(Ctl::Panic)
}

fn merge_sort(it: &mut Interp, v: Vec<Value>, cmp: &mut dyn FnMut(&mut Interp, &Value, &Value) -> R<Ordering>) -> R<Vec<Value>> {
    if v.len() <= 1 {
        return Ok(v);
    }
    let mid = v.len() / 2;
    let right = v[mid..].to_vec();
    let left = v[..mid].to_vec();
    let l = merge_sort(it, left, cmp)?;
    let r = merge_sort(it, right, cmp)?;
    let mut out = Vec::with_capacity(l.len() + r.len());
    let (mut i, mut j) = (0, 0);
    while i < l.len() && j < r.len() {
        if cmp(it, &r[j], &l[i])? == Ordering::Less {
            out.push(r[j].clone());
            j += 1;
        } else {
            out.push(l[i].clone());
            i += 1;
        }
    }
    out.extend_from_slice(&l[i..]);
    out.extend_from_slice(&r[j..]);
    Ok(out)
}

fn math1(a: &Args, f: fn(f64) -> f64) -> R<Value> {
    Ok(Value::Float(f(num(&a[0]))))
}

fn format_c(it: &mut Interp, fmt: &str, args: &[Value]) -> R<String> {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    let mut ai = 0;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        while let Some(&n) = chars.peek() {
            if n.is_ascii_digit() || matches!(n, '.' | '-' | '+' | ' ' | '#' | 'l' | 'h' | 'z') {
                spec.push(n);
                chars.next();
            } else {
                break;
            }
        }
        let Some(conv) = chars.next() else { break };
        if conv == '%' {
            out.push('%');
            continue;
        }
        let arg = args.get(ai).cloned().unwrap_or(Value::Nothing);
        ai += 1;
        let width: usize = spec.trim_start_matches(['-', '+', ' ', '#']).split('.').next().unwrap_or("").trim_start_matches(['l', 'h', 'z']).parse().unwrap_or(0);
        let prec: Option<usize> = spec.split('.').nth(1).and_then(|p| p.trim_end_matches(['l', 'h']).parse().ok());
        let left = spec.starts_with('-');
        let body = match conv {
            'd' | 'i' | 'u' => arg.as_int().to_string(),
            'x' => format!("{:x}", arg.as_int()),
            'X' => format!("{:X}", arg.as_int()),
            'o' => format!("{:o}", arg.as_int()),
            'c' => match arg { Value::Char(c) => c.to_string(), Value::Int(i) => char::from_u32(i as u32).unwrap_or('?').to_string(), _ => "?".into() },
            'f' | 'F' => format!("{:.*}", prec.unwrap_or(6), num(&arg)),
            'e' => format!("{:e}", num(&arg)),
            'g' => format!("{}", num(&arg)),
            's' => it.display(&arg)?,
            'p' => it.display(&arg)?,
            other => format!("%{other}"),
        };
        let padded = if body.chars().count() < width {
            let pad = " ".repeat(width - body.chars().count());
            if left { format!("{body}{pad}") } else { format!("{pad}{body}") }
        } else {
            body
        };
        out.push_str(&padded);
    }
    Ok(out)
}

pub fn extern_c(_it: &mut Interp, _r: Option<Place>, _a: Args) -> R<Value> {
    Err(Ctl::Panic("función extern sin implementación en el backend intérprete".into()))
}

impl Interp {
    pub fn heap_place(&mut self, addr: usize, off: i64) -> R<Place> {
        if addr == 0 {
            return self.panic("desreferencia de un puntero nulo");
        }
        match self.heap.get(addr - 1) {
            Some(Some(block)) => {
                if off < 0 || off as usize >= block.len() {
                    return self.panic(format!("acceso fuera de los límites del bloque (desplazamiento {off}, tamaño {})", block.len()));
                }
                Ok(Place { cell: block[off as usize].clone(), path: vec![] })
            }
            Some(None) => self.panic("uso de memoria después de `free`"),
            None => self.panic("puntero inválido"),
        }
    }

    pub fn run_thread(&mut self, id: usize) -> R<()> {
        let Some(task) = self.threads[id].task.take() else { return Ok(()) };
        let r = self.call_value(&task, vec![], Span::default())?;
        self.threads[id].result = Some(r);
        Ok(())
    }

    pub fn call_extern(&mut self, def: &FnDef, args: Args) -> R<Value> {
        match def.name.as_str() {
            "printf" => {
                let f = s_of(&args[0]);
                let s = format_c(self, &f, &args[1..])?;
                let _ = write!(self.out, "{s}");
                Ok(Value::Int(s.len() as i64))
            }
            "puts" => {
                let s = s_of(&args[0]);
                let _ = writeln!(self.out, "{s}");
                Ok(Value::Int(0))
            }
            "putchar" => {
                if let Some(c) = char::from_u32(args[0].as_int() as u32) {
                    let _ = write!(self.out, "{c}");
                }
                Ok(Value::Int(args[0].as_int()))
            }
            "abs" | "labs" => Ok(Value::Int(args[0].as_int().abs())),
            "strlen" => Ok(Value::Int(s_of(&args[0]).len() as i64)),
            "getpid" => Ok(Value::Int(std::process::id() as i64)),
            "exit" => Err(Ctl::Exit(args[0].as_int() as i32)),
            "sqrt" => Ok(Value::Float(num(&args[0]).sqrt())),
            "pow" => Ok(Value::Float(num(&args[0]).powf(num(&args[1])))),
            other => self.panic(format!("la función extern `{other}` no está disponible en el backend intérprete (solo printf, puts, putchar, abs, strlen, getpid, exit, sqrt, pow)")),
        }
    }
}

pub fn table() -> HashMap<&'static str, IntrinsicFn> {
    let mut t: HashMap<&'static str, IntrinsicFn> = HashMap::new();
    macro_rules! reg {
        ($k:expr, $f:expr) => {
            t.insert($k, $f)
        };
    }

    // ---------- globales ----------
    reg!("global.println", |it, _, a| {
        let s = match a.first() { Some(v) => it.display(v)?, None => String::new() };
        let _ = writeln!(it.out, "{s}");
        Ok(Value::Nothing)
    });
    reg!("global.print", |it, _, a| {
        let s = it.display(&a[0])?;
        let _ = write!(it.out, "{s}");
        Ok(Value::Nothing)
    });
    reg!("global.eprintln", |it, _, a| {
        let s = it.display(&a[0])?;
        let _ = it.out.flush();
        eprintln!("{s}");
        Ok(Value::Nothing)
    });
    reg!("global.eprint", |it, _, a| {
        let s = it.display(&a[0])?;
        let _ = it.out.flush();
        eprint!("{s}");
        Ok(Value::Nothing)
    });
    reg!("global.input", |it, _, _| {
        let _ = it.out.flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        Ok(Value::str(line.trim_end_matches(['\n', '\r'])))
    });
    reg!("global.readFile", |it, _, a| {
        Ok(match std::fs::read_to_string(&*s_of(&a[0])) {
            Ok(s) => it.ok(Value::str(s)),
            Err(e) => it.err(Value::str(format!("{}: {e}", s_of(&a[0])))),
        })
    });
    reg!("global.writeFile", |it, _, a| {
        Ok(match std::fs::write(&*s_of(&a[0]), s_of(&a[1]).as_bytes()) {
            Ok(_) => it.ok(Value::Nothing),
            Err(e) => it.err(Value::str(format!("{}: {e}", s_of(&a[0])))),
        })
    });
    reg!("global.args", |it, _, _| Ok(Value::List(Rc::new(it.args.iter().map(Value::str).collect()))));
    reg!("global.env", |it, _, a| Ok(it.opt(std::env::var(&*s_of(&a[0])).ok().map(Value::str))));
    reg!("global.to_string", |it, _, a| Ok(Value::str(it.display(&a[0])?)));
    reg!("global.assert_eq", |it, _, a| {
        if !it.values_equal(&a[0], &a[1]) {
            let (l, r) = (it.debug_str(&a[0])?, it.debug_str(&a[1])?);
            return it.panic(format!("aserción fallida: `izquierda == derecha`\n  izquierda: {l}\n  derecha:   {r}"));
        }
        Ok(Value::Nothing)
    });
    reg!("global.assert_ne", |it, _, a| {
        if it.values_equal(&a[0], &a[1]) {
            let l = it.debug_str(&a[0])?;
            return it.panic(format!("aserción fallida: `izquierda != derecha`\n  ambos: {l}"));
        }
        Ok(Value::Nothing)
    });
    reg!("global.block_on", |it, _, a| match it.deref_value(&a[0])? {
        Value::Future(f) => it.run_future(&f),
        _ => it.panic("block_on espera un Future"),
    });
    reg!("global.__repl_show", |it, _, a| {
        if !matches!(a[0], Value::Nothing) {
            let s = it.debug_str(&a[0])?;
            let _ = writeln!(it.out, "{s}");
        }
        Ok(Value::Nothing)
    });
    reg!("global.malloc", |it, _, a| {
        let n = a[0].as_int();
        if n <= 0 {
            return it.panic("malloc requiere un tamaño positivo");
        }
        it.heap.push(Some((0..n).map(|_| cell(Value::Int(0))).collect()));
        Ok(Value::Ptr(it.heap.len(), 0))
    });
    reg!("global.free", |it, _, a| match &a[0] {
        Value::Ptr(addr, off) => {
            if *addr == 0 {
                return Ok(Value::Nothing);
            }
            if *off != 0 {
                return it.panic("free de un puntero que no es el inicio del bloque");
            }
            match it.heap.get_mut(addr - 1) {
                Some(slot @ Some(_)) => {
                    *slot = None;
                    Ok(Value::Nothing)
                }
                Some(None) => it.panic("doble `free` del mismo bloque"),
                None => it.panic("free de un puntero inválido"),
            }
        }
        _ => it.panic("free espera un puntero"),
    });

    // ---------- List ----------
    reg!("List.new", |_, _, _| Ok(Value::List(Rc::new(vec![]))));
    reg!("List.length", |_, r, _| Ok(Value::Int(list_of(&this(&r)?.get().map_err(Ctl::Panic)?).len() as i64)));
    reg!("List.is_empty", |_, r, _| Ok(Value::Bool(list_of(&this(&r)?.get().map_err(Ctl::Panic)?).is_empty())));
    reg!("List.add", |it, r, mut a| {
        let item = a.pop().unwrap();
        let p = this(&r)?;
        let mut slot = Some(item);
        with_list(it, &p, |v| {
            v.push(slot.take().unwrap());
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("List.get", |it, r, a| {
        let l = list_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let i = a[0].as_int();
        Ok(it.opt((i >= 0).then(|| l.get(i as usize).cloned()).flatten()))
    });
    reg!("List.first", |it, r, _| Ok(it.opt(list_of(&this(&r)?.get().map_err(Ctl::Panic)?).first().cloned())));
    reg!("List.last", |it, r, _| Ok(it.opt(list_of(&this(&r)?.get().map_err(Ctl::Panic)?).last().cloned())));
    reg!("List.insert", |it, r, a| {
        let p = this(&r)?;
        let i = a[0].as_int();
        let mut slot = Some(a[1].clone());
        with_list(it, &p, |v| {
            if i < 0 || i as usize > v.len() {
                return Err(format!("índice {i} fuera de rango para insertar (longitud {})", v.len()));
            }
            v.insert(i as usize, slot.take().unwrap());
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("List.remove_at", |it, r, a| {
        let p = this(&r)?;
        let i = a[0].as_int();
        let v = with_list(it, &p, |v| {
            if i < 0 || i as usize >= v.len() {
                return Err(format!("índice {i} fuera de rango (longitud {})", v.len()));
            }
            Ok(v.remove(i as usize))
        })?;
        Ok(v)
    });
    reg!("List.pop", |it, r, _| {
        let p = this(&r)?;
        let v = with_list(it, &p, |v| Ok(v.pop()))?;
        Ok(it.opt(v))
    });
    reg!("List.clear", |it, r, _| {
        with_list(it, &this(&r)?, |v| {
            v.clear();
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("List.contains", |it, r, a| {
        let l = list_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        Ok(Value::Bool(l.iter().any(|x| it.values_equal(x, &a[0]))))
    });
    reg!("List.index_of", |it, r, a| {
        let l = list_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let pos = l.iter().position(|x| it.values_equal(x, &a[0]));
        Ok(it.opt(pos.map(|p| Value::Int(p as i64))))
    });
    reg!("List.reverse", |it, r, _| {
        with_list(it, &this(&r)?, |v| {
            v.reverse();
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("List.extend", |it, r, a| {
        let other = list_of(&it.deref_value(&a[0])?);
        with_list(it, &this(&r)?, |v| {
            v.extend(other.iter().cloned());
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("List.swap", |it, r, a| {
        let (i, j) = (a[0].as_int(), a[1].as_int());
        with_list(it, &this(&r)?, |v| {
            let n = v.len() as i64;
            if i < 0 || j < 0 || i >= n || j >= n {
                return Err(format!("índices {i}, {j} fuera de rango (longitud {n})"));
            }
            v.swap(i as usize, j as usize);
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("List.sort", |it, r, _| {
        let p = this(&r)?;
        let l = list_of(&p.get().map_err(Ctl::Panic)?);
        let sorted = merge_sort(it, (*l).clone(), &mut |it, a, b| Ok(it.compare_values(a, b)?.unwrap_or(Ordering::Equal)))?;
        p.set(Value::List(Rc::new(sorted))).map_err(Ctl::Panic)?;
        Ok(Value::Nothing)
    });
    reg!("List.sort_by", |it, r, a| {
        let p = this(&r)?;
        let l = list_of(&p.get().map_err(Ctl::Panic)?);
        let f = a[0].clone();
        let sorted = merge_sort(it, (*l).clone(), &mut |it, x, y| {
            let c = it.call_value(&f, vec![x.clone(), y.clone()], Span::default())?;
            Ok(c.as_int().cmp(&0))
        })?;
        p.set(Value::List(Rc::new(sorted))).map_err(Ctl::Panic)?;
        Ok(Value::Nothing)
    });
    reg!("List.join", |it, r, a| {
        let l = list_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let sep = s_of(&a[0]);
        let mut parts = vec![];
        for x in l.iter() {
            parts.push(it.display(x)?);
        }
        Ok(Value::str(parts.join(&sep)))
    });
    reg!("List.sum", |it, r, _| {
        let l = list_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let mut acc = Value::Int(0);
        for (i, x) in l.iter().enumerate() {
            acc = if i == 0 { x.clone() } else { it.binop(BinOp::Add, &acc, x)? };
        }
        Ok(acc)
    });

    // ---------- Array ----------
    reg!("Array.length", |_, r, _| Ok(Value::Int(list_of(&this(&r)?.get().map_err(Ctl::Panic)?).len() as i64)));
    reg!("Array.contains", |it, r, a| {
        let l = list_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        Ok(Value::Bool(l.iter().any(|x| it.values_equal(x, &a[0]))))
    });
    reg!("Array.to_list", |_, r, _| Ok(Value::List(list_of(&this(&r)?.get().map_err(Ctl::Panic)?))));

    // ---------- Set ----------
    reg!("Set.new", |_, _, _| Ok(Value::Set(Rc::new(OMap::default()))));
    reg!("Set.add", |it, r, a| {
        if a[0].to_key().is_none() {
            return it.panic("valor no válido como elemento de un Set");
        }
        let mut slot = Some(a[0].clone());
        let fresh = with_omap(&this(&r)?, |m| Ok(m.insert(slot.take().unwrap(), Value::Nothing).is_none()))?;
        Ok(Value::Bool(fresh))
    });
    reg!("Set.remove", |_, r, a| Ok(Value::Bool(with_omap(&this(&r)?, |m| Ok(m.remove(&a[0]).is_some()))?)));
    reg!("Set.contains", |_, r, a| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Set(m) => Ok(Value::Bool(m.contains(&a[0]))),
        _ => Ok(Value::Bool(false)),
    });
    reg!("Set.length", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Set(m) => Ok(Value::Int(m.len() as i64)),
        _ => Ok(Value::Int(0)),
    });
    reg!("Set.is_empty", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Set(m) => Ok(Value::Bool(m.len() == 0)),
        _ => Ok(Value::Bool(true)),
    });
    reg!("Set.clear", |_, r, _| {
        with_omap(&this(&r)?, |m| {
            m.clear();
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("Set.to_list", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Set(m) => Ok(Value::List(Rc::new(m.entries.iter().map(|(k, _)| k.clone()).collect()))),
        _ => Ok(Value::List(Rc::new(vec![]))),
    });

    // ---------- Map ----------
    reg!("Map.new", |_, _, _| Ok(Value::Map(Rc::new(OMap::default()))));
    reg!("Map.insert", |it, r, mut a| {
        let v = a.pop().unwrap();
        let k = a.pop().unwrap();
        if k.to_key().is_none() {
            return it.panic("valor no válido como clave de un Map");
        }
        let mut slot = Some((k, v));
        let old = with_omap(&this(&r)?, |m| {
            let (k, v) = slot.take().unwrap();
            Ok(m.insert(k, v))
        })?;
        Ok(it.opt(old))
    });
    reg!("Map.get", |it, r, a| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Map(m) => Ok(it.opt(m.get(&a[0]).cloned())),
        _ => Ok(it.none()),
    });
    reg!("Map.remove", |it, r, a| {
        let old = with_omap(&this(&r)?, |m| Ok(m.remove(&a[0])))?;
        Ok(it.opt(old))
    });
    reg!("Map.contains_key", |_, r, a| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Map(m) => Ok(Value::Bool(m.contains(&a[0]))),
        _ => Ok(Value::Bool(false)),
    });
    reg!("Map.length", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Map(m) => Ok(Value::Int(m.len() as i64)),
        _ => Ok(Value::Int(0)),
    });
    reg!("Map.is_empty", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Map(m) => Ok(Value::Bool(m.len() == 0)),
        _ => Ok(Value::Bool(true)),
    });
    reg!("Map.clear", |_, r, _| {
        with_omap(&this(&r)?, |m| {
            m.clear();
            Ok(())
        })?;
        Ok(Value::Nothing)
    });
    reg!("Map.keys", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Map(m) => Ok(Value::List(Rc::new(m.entries.iter().map(|(k, _)| k.clone()).collect()))),
        _ => Ok(Value::List(Rc::new(vec![]))),
    });
    reg!("Map.values", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Map(m) => Ok(Value::List(Rc::new(m.entries.iter().map(|(_, v)| v.clone()).collect()))),
        _ => Ok(Value::List(Rc::new(vec![]))),
    });

    // ---------- Box / Rc ----------
    reg!("Box.new", |_, _, mut a| Ok(Value::Box(cell(a.pop().unwrap()))));
    reg!("Box.get", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Box(c) => Ok(c.borrow().clone()),
        _ => Ok(Value::Nothing),
    });
    reg!("Box.set", |_, r, mut a| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Box(c) => {
            *c.borrow_mut() = a.pop().unwrap();
            Ok(Value::Nothing)
        }
        _ => Ok(Value::Nothing),
    });
    reg!("Rc.new", |_, _, mut a| Ok(Value::Rc(cell(a.pop().unwrap()))));
    reg!("Rc.get", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Rc(c) => Ok(c.borrow().clone()),
        _ => Ok(Value::Nothing),
    });
    reg!("Rc.set", |_, r, mut a| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Rc(c) => {
            *c.borrow_mut() = a.pop().unwrap();
            Ok(Value::Nothing)
        }
        _ => Ok(Value::Nothing),
    });
    reg!("Rc.clone", |_, r, _| this(&r)?.get().map_err(Ctl::Panic));
    reg!("Rc.strong_count", |_, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        // -1: la copia temporal que acabamos de leer del lugar
        Value::Rc(c) => Ok(Value::Int(Rc::strong_count(&c) as i64 - 1)),
        _ => Ok(Value::Int(0)),
    });

    // ---------- $string ----------
    reg!("$string.length", |_, r, _| Ok(Value::Int(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).chars().count() as i64)));
    reg!("$string.is_empty", |_, r, _| Ok(Value::Bool(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).is_empty())));
    reg!("$string.to_upper", |_, r, _| Ok(Value::str(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).to_uppercase())));
    reg!("$string.to_lower", |_, r, _| Ok(Value::str(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).to_lowercase())));
    reg!("$string.trim", |_, r, _| Ok(Value::str(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).trim())));
    reg!("$string.contains", |_, r, a| Ok(Value::Bool(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).contains(&*s_of(&a[0])))));
    reg!("$string.starts_with", |_, r, a| Ok(Value::Bool(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).starts_with(&*s_of(&a[0])))));
    reg!("$string.ends_with", |_, r, a| Ok(Value::Bool(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).ends_with(&*s_of(&a[0])))));
    reg!("$string.split", |_, r, a| {
        let s = s_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let sep = s_of(&a[0]);
        let parts: Vec<Value> = if sep.is_empty() { s.chars().map(|c| Value::str(c.to_string())).collect() } else { s.split(&*sep).map(Value::str).collect() };
        Ok(Value::List(Rc::new(parts)))
    });
    reg!("$string.lines", |_, r, _| {
        let s = s_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        Ok(Value::List(Rc::new(s.lines().map(Value::str).collect())))
    });
    reg!("$string.replace", |_, r, a| Ok(Value::str(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).replace(&*s_of(&a[0]), &s_of(&a[1])))));
    reg!("$string.substring", |it, r, a| {
        let s = s_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let n = s.chars().count() as i64;
        let (lo, hi) = (a[0].as_int(), a[1].as_int());
        if lo < 0 || hi > n || lo > hi {
            return it.panic(format!("substring({lo}, {hi}) fuera de rango (longitud {n})"));
        }
        Ok(Value::str(s.chars().skip(lo as usize).take((hi - lo) as usize).collect::<String>()))
    });
    reg!("$string.char_at", |it, r, a| {
        let s = s_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let i = a[0].as_int();
        Ok(it.opt((i >= 0).then(|| s.chars().nth(i as usize)).flatten().map(Value::Char)))
    });
    reg!("$string.chars", |_, r, _| Ok(Value::List(Rc::new(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).chars().map(Value::Char).collect()))));
    reg!("$string.repeat", |it, r, a| {
        if a[0].as_int() < 0 {
            return it.panic("repeat con un número negativo");
        }
        Ok(Value::str(s_of(&this(&r)?.get().map_err(Ctl::Panic)?).repeat(a[0].as_int() as usize)))
    });
    reg!("$string.index_of", |it, r, a| {
        let s = s_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        let pos = s.find(&*s_of(&a[0])).map(|b| Value::Int(s[..b].chars().count() as i64));
        Ok(it.opt(pos))
    });
    reg!("$string.parse_int", |it, r, _| {
        let s = s_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        Ok(match s.parse::<i64>() {
            Ok(i) => it.ok(Value::Int(i)),
            Err(_) => it.err(Value::str(format!("no se puede convertir \"{s}\" a $int"))),
        })
    });
    reg!("$string.parse_float", |it, r, _| {
        let s = s_of(&this(&r)?.get().map_err(Ctl::Panic)?);
        Ok(match s.parse::<f64>() {
            Ok(f) => it.ok(Value::Float(f)),
            Err(_) => it.err(Value::str(format!("no se puede convertir \"{s}\" a $float"))),
        })
    });
    reg!("$string.to_string", |_, r, _| this(&r)?.get().map_err(Ctl::Panic));

    // ---------- números, char, bool ----------
    reg!("$int.to_string", |it, r, _| Ok(Value::str(it.display(&this(&r)?.get().map_err(Ctl::Panic)?)?)));
    reg!("$int.abs", |it, r, _| match this(&r)?.get().map_err(Ctl::Panic)? {
        Value::Int(i) => i.checked_abs().map(Value::Int).ok_or_else(|| Ctl::Panic("desbordamiento aritmético".into())),
        _ => it.panic("abs sobre un valor no entero"),
    });
    reg!("$int.min", |_, r, a| Ok(Value::Int(this(&r)?.get().map_err(Ctl::Panic)?.as_int().min(a[0].as_int()))));
    reg!("$int.max", |_, r, a| Ok(Value::Int(this(&r)?.get().map_err(Ctl::Panic)?.as_int().max(a[0].as_int()))));
    reg!("$float.to_string", |it, r, _| Ok(Value::str(it.display(&this(&r)?.get().map_err(Ctl::Panic)?)?)));
    reg!("$float.abs", |_, r, _| Ok(Value::Float(num(&this(&r)?.get().map_err(Ctl::Panic)?).abs())));
    reg!("$float.floor", |_, r, _| Ok(Value::Float(num(&this(&r)?.get().map_err(Ctl::Panic)?).floor())));
    reg!("$float.ceil", |_, r, _| Ok(Value::Float(num(&this(&r)?.get().map_err(Ctl::Panic)?).ceil())));
    reg!("$float.round", |_, r, _| Ok(Value::Float(num(&this(&r)?.get().map_err(Ctl::Panic)?).round())));
    reg!("$float.sqrt", |_, r, _| Ok(Value::Float(num(&this(&r)?.get().map_err(Ctl::Panic)?).sqrt())));
    reg!("$float.min", |_, r, a| Ok(Value::Float(num(&this(&r)?.get().map_err(Ctl::Panic)?).min(num(&a[0])))));
    reg!("$float.max", |_, r, a| Ok(Value::Float(num(&this(&r)?.get().map_err(Ctl::Panic)?).max(num(&a[0])))));
    reg!("$float.is_nan", |_, r, _| Ok(Value::Bool(num(&this(&r)?.get().map_err(Ctl::Panic)?).is_nan())));
    fn ch(r: &Option<Place>) -> R<char> {
        match this(r)?.get().map_err(Ctl::Panic)? {
            Value::Char(c) => Ok(c),
            _ => Ok('\0'),
        }
    }
    reg!("$char.to_string", |_, r, _| Ok(Value::str(ch(&r)?.to_string())));
    reg!("$char.is_digit", |_, r, _| Ok(Value::Bool(ch(&r)?.is_ascii_digit())));
    reg!("$char.is_alpha", |_, r, _| Ok(Value::Bool(ch(&r)?.is_alphabetic())));
    reg!("$char.is_whitespace", |_, r, _| Ok(Value::Bool(ch(&r)?.is_whitespace())));
    reg!("$char.is_upper", |_, r, _| Ok(Value::Bool(ch(&r)?.is_uppercase())));
    reg!("$char.is_lower", |_, r, _| Ok(Value::Bool(ch(&r)?.is_lowercase())));
    reg!("$char.to_upper", |_, r, _| Ok(Value::Char(ch(&r)?.to_uppercase().next().unwrap_or('\0'))));
    reg!("$char.to_lower", |_, r, _| Ok(Value::Char(ch(&r)?.to_lowercase().next().unwrap_or('\0'))));
    reg!("$bool.to_string", |it, r, _| Ok(Value::str(it.display(&this(&r)?.get().map_err(Ctl::Panic)?)?)));

    // ---------- math ----------
    reg!("math.sqrt", |it, _, a| {
        let x = num(&a[0]);
        if x < 0.0 { return it.panic("raíz cuadrada de un número negativo"); }
        Ok(Value::Float(x.sqrt()))
    });
    reg!("math.abs", |it, _, a| match &a[0] {
        Value::Int(i) => i.checked_abs().map(Value::Int).ok_or_else(|| Ctl::Panic("desbordamiento aritmético".into())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        _ => it.panic("abs sobre un valor no numérico"),
    });
    reg!("math.pow", |it, _, a| it.binop(BinOp::Pow, &a[0], &a[1]));
    reg!("math.floor", |_, _, a| math1(&a, f64::floor));
    reg!("math.ceil", |_, _, a| math1(&a, f64::ceil));
    reg!("math.round", |_, _, a| math1(&a, f64::round));
    reg!("math.trunc", |_, _, a| math1(&a, f64::trunc));
    reg!("math.sin", |_, _, a| math1(&a, f64::sin));
    reg!("math.cos", |_, _, a| math1(&a, f64::cos));
    reg!("math.tan", |_, _, a| math1(&a, f64::tan));
    reg!("math.asin", |_, _, a| math1(&a, f64::asin));
    reg!("math.acos", |_, _, a| math1(&a, f64::acos));
    reg!("math.atan", |_, _, a| math1(&a, f64::atan));
    reg!("math.atan2", |_, _, a| Ok(Value::Float(num(&a[0]).atan2(num(&a[1])))));
    reg!("math.ln", |_, _, a| math1(&a, f64::ln));
    reg!("math.log10", |_, _, a| math1(&a, f64::log10));
    reg!("math.exp", |_, _, a| math1(&a, f64::exp));
    reg!("math.min", |it, _, a| Ok(if it.compare_values(&a[0], &a[1])? == Some(Ordering::Greater) { a[1].clone() } else { a[0].clone() }));
    reg!("math.max", |it, _, a| Ok(if it.compare_values(&a[0], &a[1])? == Some(Ordering::Less) { a[1].clone() } else { a[0].clone() }));
    reg!("math.clamp", |it, _, a| {
        if it.compare_values(&a[1], &a[2])? == Some(Ordering::Greater) {
            return it.panic("clamp con límite inferior mayor que el superior");
        }
        if it.compare_values(&a[0], &a[1])? == Some(Ordering::Less) { return Ok(a[1].clone()); }
        if it.compare_values(&a[0], &a[2])? == Some(Ordering::Greater) { return Ok(a[2].clone()); }
        Ok(a[0].clone())
    });

    // ---------- time / process / io / fs ----------
    reg!("time.now_ms", |_, _, _| {
        let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        Ok(Value::Int(d.as_millis() as i64))
    });
    reg!("time.monotonic_ns", |_, _, _| {
        thread_local! { static START: std::time::Instant = std::time::Instant::now(); }
        Ok(Value::Int(START.with(|s| s.elapsed().as_nanos() as i64)))
    });
    reg!("time.sleep_ms", |it, _, a| {
        let _ = it.out.flush();
        std::thread::sleep(std::time::Duration::from_millis(a[0].as_int().max(0) as u64));
        Ok(Value::Nothing)
    });
    reg!("process.pid", |_, _, _| Ok(Value::Int(std::process::id() as i64)));
    reg!("process.cwd", |_, _, _| Ok(Value::str(std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default())));
    reg!("io.read_line", |it, _, _| {
        let _ = it.out.flush();
        let mut line = String::new();
        let n = std::io::stdin().lock().read_line(&mut line).unwrap_or(0);
        Ok(if n == 0 { it.none() } else { it.some(Value::str(line.trim_end_matches(['\n', '\r']))) })
    });
    reg!("io.read_all", |it, _, _| {
        let _ = it.out.flush();
        let mut s = String::new();
        let _ = std::io::stdin().lock().read_to_string(&mut s);
        Ok(Value::str(s))
    });
    reg!("io.flush", |it, _, _| {
        let _ = it.out.flush();
        Ok(Value::Nothing)
    });
    reg!("fs.read", |it, _, a| Ok(match std::fs::read_to_string(&*s_of(&a[0])) {
        Ok(s) => it.ok(Value::str(s)),
        Err(e) => it.err(Value::str(format!("{}: {e}", s_of(&a[0])))),
    }));
    reg!("fs.write", |it, _, a| Ok(match std::fs::write(&*s_of(&a[0]), s_of(&a[1]).as_bytes()) {
        Ok(_) => it.ok(Value::Nothing),
        Err(e) => it.err(Value::str(format!("{}: {e}", s_of(&a[0])))),
    }));
    reg!("fs.append", |it, _, a| {
        use std::fs::OpenOptions;
        let r = OpenOptions::new().create(true).append(true).open(&*s_of(&a[0])).and_then(|mut f| f.write_all(s_of(&a[1]).as_bytes()));
        Ok(match r {
            Ok(_) => it.ok(Value::Nothing),
            Err(e) => it.err(Value::str(format!("{}: {e}", s_of(&a[0])))),
        })
    });
    reg!("fs.exists", |_, _, a| Ok(Value::Bool(std::path::Path::new(&*s_of(&a[0])).exists())));
    reg!("fs.remove", |it, _, a| {
        let p = s_of(&a[0]);
        let r = if std::path::Path::new(&*p).is_dir() { std::fs::remove_dir_all(&*p) } else { std::fs::remove_file(&*p) };
        Ok(match r {
            Ok(_) => it.ok(Value::Nothing),
            Err(e) => it.err(Value::str(format!("{p}: {e}"))),
        })
    });
    reg!("fs.create_dir", |it, _, a| Ok(match std::fs::create_dir_all(&*s_of(&a[0])) {
        Ok(_) => it.ok(Value::Nothing),
        Err(e) => it.err(Value::str(format!("{}: {e}", s_of(&a[0])))),
    }));
    reg!("fs.list_dir", |it, _, a| Ok(match std::fs::read_dir(&*s_of(&a[0])) {
        Ok(rd) => {
            let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
            names.sort();
            it.ok(Value::List(Rc::new(names.into_iter().map(Value::str).collect())))
        }
        Err(e) => it.err(Value::str(format!("{}: {e}", s_of(&a[0])))),
    }));

    // ---------- hilos y canales (cooperativos) ----------
    reg!("thread.spawn", |it, _, mut a| {
        it.threads.push(ThreadSlot { task: Some(a.pop().unwrap()), result: None });
        Ok(Value::Handle(Rc::new(HandleState { id: it.threads.len() - 1 })))
    });
    reg!("thread.sleep", |it, _, a| {
        let _ = it.out.flush();
        std::thread::sleep(std::time::Duration::from_millis(a[0].as_int().max(0) as u64));
        Ok(Value::Nothing)
    });
    reg!("thread::JoinHandle.join", |it, r, _| {
        let Value::Handle(h) = this(&r)?.get().map_err(Ctl::Panic)? else { return it.panic("join sobre un valor que no es un hilo") };
        it.run_thread(h.id)?;
        Ok(it.threads[h.id].result.clone().unwrap_or(Value::Nothing))
    });
    reg!("channel.new", |_, _, _| {
        let st = Rc::new(RefCell::new(ChanState { queue: Default::default(), senders: 1 }));
        Ok(Value::Tuple(Rc::new(vec![Value::Sender(st.clone()), Value::Receiver(st)])))
    });
    reg!("channel::Sender.send", |it, r, mut a| {
        let Value::Sender(st) = this(&r)?.get().map_err(Ctl::Panic)? else { return it.panic("send sobre un valor que no es un Sender") };
        st.borrow_mut().queue.push_back(a.pop().unwrap());
        Ok(Value::Nothing)
    });
    reg!("channel::Receiver.try_recv", |it, r, _| {
        let Value::Receiver(st) = this(&r)?.get().map_err(Ctl::Panic)? else { return it.panic("recv sobre un valor que no es un Receiver") };
        let v = st.borrow_mut().queue.pop_front();
        Ok(it.opt(v))
    });
    reg!("channel::Receiver.recv", |it, r, _| {
        let Value::Receiver(st) = this(&r)?.get().map_err(Ctl::Panic)? else { return it.panic("recv sobre un valor que no es un Receiver") };
        loop {
            let v = st.borrow_mut().queue.pop_front();
            if let Some(v) = v {
                return Ok(it.some(v));
            }
            match it.threads.iter().position(|t| t.task.is_some()) {
                Some(id) => it.run_thread(id)?,
                None => return Ok(it.none()),
            }
        }
    });

    let _ = ps;
    t
}
