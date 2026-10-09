//! Valores del runtime, ámbitos y lugares (places) con semántica de valor y copy-on-write.

use crate::ast::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub type Cell = Rc<RefCell<Value>>;

pub fn cell(v: Value) -> Cell {
    Rc::new(RefCell::new(v))
}

#[derive(Debug)]
pub struct StructInfo {
    pub key: String,
    pub name: String,
    pub fields: Vec<String>,
}

impl StructInfo {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f == name)
    }
}

#[derive(Debug)]
pub struct EnumInfo {
    pub key: String,
    pub name: String,
    /// (nombre de la variante, aridad)
    pub variants: Vec<(String, usize)>,
}

impl EnumInfo {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.variants.iter().position(|(n, _)| n == name)
    }
}

#[derive(Clone, Debug)]
pub struct StructVal {
    pub info: Rc<StructInfo>,
    pub vals: Vec<Value>,
}

#[derive(Clone, Debug)]
pub struct VariantVal {
    pub info: Rc<EnumInfo>,
    pub idx: usize,
    pub vals: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Field(usize),
    Index(usize),
    Key(Key),
}

#[derive(Debug)]
pub struct RefVal {
    pub cell: Cell,
    pub path: Vec<Seg>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i64),
    Bool(bool),
    Char(char),
    Str(Rc<str>),
    Float(u64),
    Tuple(Vec<Key>),
    Variant(String, Vec<Key>),
    Struct(String, Vec<Key>),
    Unit,
}

/// Mapa/conjunto con orden de inserción.
#[derive(Clone, Debug, Default)]
pub struct OMap {
    pub entries: Vec<(Value, Value)>,
    index: HashMap<Key, usize>,
}

impl OMap {
    pub fn get(&self, k: &Value) -> Option<&Value> {
        let key = k.to_key()?;
        self.index.get(&key).map(|&i| &self.entries[i].1)
    }
    pub fn get_mut(&mut self, k: &Key) -> Option<&mut Value> {
        let i = *self.index.get(k)?;
        Some(&mut self.entries[i].1)
    }
    pub fn contains(&self, k: &Value) -> bool {
        k.to_key().is_some_and(|key| self.index.contains_key(&key))
    }
    /// Inserta y devuelve el valor anterior.
    pub fn insert(&mut self, k: Value, v: Value) -> Option<Value> {
        let key = k.to_key().expect("clave no hasheable");
        if let Some(&i) = self.index.get(&key) {
            return Some(std::mem::replace(&mut self.entries[i].1, v));
        }
        self.index.insert(key, self.entries.len());
        self.entries.push((k, v));
        None
    }
    pub fn remove(&mut self, k: &Value) -> Option<Value> {
        let key = k.to_key()?;
        let i = self.index.remove(&key)?;
        let (_, v) = self.entries.remove(i);
        for (_, idx) in self.index.iter_mut() {
            if *idx > i {
                *idx -= 1;
            }
        }
        Some(v)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }
}

#[derive(Debug)]
pub struct ClosureDef {
    pub params: &'static [ClosureParam],
    pub body: &'static Expr,
    pub env: Rc<Scope>,
    pub module: usize,
}

#[derive(Debug)]
pub enum FuncVal {
    Defs(Rc<Vec<Rc<super::FnDef>>>),
    Closure(Rc<ClosureDef>),
    Ctor { info: Rc<EnumInfo>, idx: usize },
}

#[derive(Debug)]
pub struct FutureState {
    pub thunk: Option<(Rc<FuncVal>, Vec<Value>, Option<Place>)>,
    pub result: Option<Value>,
}

#[derive(Debug)]
pub struct ChanState {
    pub queue: std::collections::VecDeque<Value>,
    pub senders: usize,
}

#[derive(Debug)]
pub struct HandleState {
    pub id: usize,
}

#[derive(Clone, Debug)]
pub enum Value {
    Nothing,
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    Str(Rc<str>),
    Array(Rc<Vec<Value>>),
    List(Rc<Vec<Value>>),
    Map(Rc<OMap>),
    Set(Rc<OMap>),
    Tuple(Rc<Vec<Value>>),
    Struct(Rc<StructVal>),
    Variant(Rc<VariantVal>),
    Func(Rc<FuncVal>),
    Ref(Rc<RefVal>),
    Box(Cell),
    Rc(Cell),
    Ptr(usize, i64),
    Future(Rc<RefCell<FutureState>>),
    Handle(Rc<HandleState>),
    Sender(Rc<RefCell<ChanState>>),
    Receiver(Rc<RefCell<ChanState>>),
}

impl Value {
    pub fn str(s: impl AsRef<str>) -> Value {
        Value::Str(Rc::from(s.as_ref()))
    }

    pub fn type_key(&self) -> String {
        match self {
            Value::Nothing => "$nothing".into(),
            Value::Int(_) => "$int".into(),
            Value::Float(_) => "$float".into(),
            Value::Bool(_) => "$bool".into(),
            Value::Char(_) => "$char".into(),
            Value::Str(_) => "$string".into(),
            Value::Array(_) => "Array".into(),
            Value::List(_) => "List".into(),
            Value::Map(_) => "Map".into(),
            Value::Set(_) => "Set".into(),
            Value::Tuple(_) => "Tuple".into(),
            Value::Struct(s) => s.info.key.clone(),
            Value::Variant(v) => v.info.key.clone(),
            Value::Func(_) => "Fn".into(),
            Value::Ref(_) => "Ref".into(),
            Value::Box(_) => "Box".into(),
            Value::Rc(_) => "Rc".into(),
            Value::Ptr(..) => "Ptr".into(),
            Value::Future(_) => "Future".into(),
            Value::Handle(_) => "thread::JoinHandle".into(),
            Value::Sender(_) => "channel::Sender".into(),
            Value::Receiver(_) => "channel::Receiver".into(),
        }
    }

    pub fn to_key(&self) -> Option<Key> {
        Some(match self {
            Value::Int(i) => Key::Int(*i),
            Value::Bool(b) => Key::Bool(*b),
            Value::Char(c) => Key::Char(*c),
            Value::Str(s) => Key::Str(s.clone()),
            Value::Float(f) => Key::Float(f.to_bits()),
            Value::Nothing => Key::Unit,
            Value::Tuple(v) | Value::Array(v) | Value::List(v) => Key::Tuple(v.iter().map(|x| x.to_key()).collect::<Option<_>>()?),
            Value::Variant(v) => Key::Variant(format!("{}.{}", v.info.key, v.info.variants[v.idx].0), v.vals.iter().map(|x| x.to_key()).collect::<Option<_>>()?),
            Value::Struct(s) => Key::Struct(s.info.key.clone(), s.vals.iter().map(|x| x.to_key()).collect::<Option<_>>()?),
            Value::Ref(_) | Value::Box(_) | Value::Rc(_) => return None,
            _ => return None,
        })
    }

    pub fn as_int(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            Value::Char(c) => *c as i64,
            _ => 0,
        }
    }
}

// ---------- ámbitos ----------

#[derive(Debug)]
pub struct Scope {
    pub vars: RefCell<Vec<(Rc<str>, Cell)>>,
    pub parent: Option<Rc<Scope>>,
}

impl Scope {
    pub fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Scope { vars: RefCell::new(Vec::new()), parent })
    }

    pub fn declare(&self, name: &str, v: Value) -> Cell {
        let c = cell(v);
        self.vars.borrow_mut().push((Rc::from(name), c.clone()));
        c
    }

    pub fn lookup(&self, name: &str) -> Option<Cell> {
        {
            let vars = self.vars.borrow();
            for (n, c) in vars.iter().rev() {
                if &**n == name {
                    return Some(c.clone());
                }
            }
        }
        match &self.parent {
            Some(p) => p.lookup(name),
            None => None,
        }
    }
}

// ---------- lugares ----------

/// Resultado de acceder a un lugar: éxito o mensaje de pánico.
pub type PResult<T> = Result<T, String>;

pub fn read_path(cell: &Cell, path: &[Seg]) -> PResult<Value> {
    let v = cell.borrow();
    read_in(&v, path)
}

fn read_in(v: &Value, path: &[Seg]) -> PResult<Value> {
    if path.is_empty() {
        return Ok(v.clone());
    }
    match v {
        Value::Ref(r) => {
            let mut full = r.path.clone();
            full.extend_from_slice(path);
            read_path(&r.cell, &full)
        }
        Value::Box(c) | Value::Rc(c) => read_path(c, path),
        Value::Struct(s) => match &path[0] {
            Seg::Field(i) => read_in(&s.vals[*i], &path[1..]),
            _ => Err("acceso inválido a un struct".into()),
        },
        Value::Tuple(t) | Value::Array(t) | Value::List(t) => match &path[0] {
            Seg::Index(i) => match t.get(*i) {
                Some(x) => read_in(x, &path[1..]),
                None => Err(format!("índice {i} fuera de rango (longitud {})", t.len())),
            },
            _ => Err("acceso inválido".into()),
        },
        Value::Map(m) => match &path[0] {
            Seg::Key(k) => match m.entries.iter().find(|(kk, _)| kk.to_key().as_ref() == Some(k)) {
                Some((_, x)) => read_in(x, &path[1..]),
                None => Err("clave inexistente en el mapa".into()),
            },
            _ => Err("acceso inválido".into()),
        },
        Value::Variant(vv) => match &path[0] {
            Seg::Field(i) => read_in(&vv.vals[*i], &path[1..]),
            _ => Err("acceso inválido a una variante".into()),
        },
        _ => Err("acceso inválido a un valor".into()),
    }
}

pub fn with_path_mut<R>(cell: &Cell, path: &[Seg], f: &mut dyn FnMut(&mut Value) -> PResult<R>) -> PResult<R> {
    let mut guard = cell.borrow_mut();
    descend(&mut guard, path, f)
}

fn descend<R>(v: &mut Value, path: &[Seg], f: &mut dyn FnMut(&mut Value) -> PResult<R>) -> PResult<R> {
    if path.is_empty() {
        return f(v);
    }
    match v {
        Value::Ref(r) => {
            let r = r.clone();
            let mut full = r.path.clone();
            full.extend_from_slice(path);
            with_path_mut(&r.cell, &full, f)
        }
        Value::Box(c) | Value::Rc(c) => {
            let c = c.clone();
            with_path_mut(&c, path, f)
        }
        Value::Struct(s) => {
            let s = Rc::make_mut(s);
            match &path[0] {
                Seg::Field(i) => descend(&mut s.vals[*i], &path[1..], f),
                _ => Err("acceso inválido a un struct".into()),
            }
        }
        Value::Variant(vv) => {
            let vv = Rc::make_mut(vv);
            match &path[0] {
                Seg::Field(i) => descend(&mut vv.vals[*i], &path[1..], f),
                _ => Err("acceso inválido a una variante".into()),
            }
        }
        Value::Tuple(t) | Value::Array(t) | Value::List(t) => {
            let len = t.len();
            let t = Rc::make_mut(t);
            match &path[0] {
                Seg::Index(i) => match t.get_mut(*i) {
                    Some(x) => descend(x, &path[1..], f),
                    None => Err(format!("índice {i} fuera de rango (longitud {len})")),
                },
                _ => Err("acceso inválido".into()),
            }
        }
        Value::Map(m) => {
            let m = Rc::make_mut(m);
            match &path[0] {
                Seg::Key(k) => match m.get_mut(k) {
                    Some(x) => descend(x, &path[1..], f),
                    None => Err("clave inexistente en el mapa".into()),
                },
                _ => Err("acceso inválido".into()),
            }
        }
        _ => Err("acceso inválido a un valor".into()),
    }
}

/// Un lugar de memoria: variable (celda) más camino de campos/índices.
#[derive(Clone, Debug)]
pub struct Place {
    pub cell: Cell,
    pub path: Vec<Seg>,
}

impl Place {
    pub fn temp(v: Value) -> Place {
        Place { cell: cell(v), path: vec![] }
    }
    pub fn get(&self) -> PResult<Value> {
        read_path(&self.cell, &self.path)
    }
    pub fn with_mut<R>(&self, mut f: impl FnMut(&mut Value) -> PResult<R>) -> PResult<R> {
        with_path_mut(&self.cell, &self.path, &mut f)
    }
    pub fn set(&self, v: Value) -> PResult<()> {
        let mut slot = Some(v);
        self.with_mut(|dst| {
            *dst = slot.take().unwrap();
            Ok(())
        })
    }
    pub fn to_ref(&self) -> Value {
        Value::Ref(Rc::new(RefVal { cell: self.cell.clone(), path: self.path.clone() }))
    }
}
