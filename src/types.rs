//! Representación semántica de tipos.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Char,
    Str,
    Nothing,
    /// Tipo de `return`, `break`, `panic`...: compatible con cualquier tipo.
    Never,
    /// Tipo del literal `null`.
    Null,
    /// Tipo tras un error: suprime errores en cascada.
    Error,
    Named(Rc<str>, Vec<Ty>),
    Param(Rc<str>),
    Var(u32),
    Array(Box<Ty>, usize),
    Tuple(Vec<Ty>),
    Ref(Box<Ty>),
    Ptr(Box<Ty>),
    Func(Vec<Ty>, Box<Ty>),
    Union(Vec<Ty>),
}

/// Nombre calificado de un tipo declarado por el usuario. El módulo `core` no lleva prefijo.
pub fn qualify(module: &str, name: &str) -> String {
    if module == "core" { name.to_string() } else { format!("{module}::{name}") }
}

impl Ty {
    pub fn named(name: &str, args: Vec<Ty>) -> Ty {
        Ty::Named(Rc::from(name), args)
    }
    pub fn option(inner: Ty) -> Ty {
        Ty::named("Option", vec![inner])
    }
    pub fn result(ok: Ty, err: Ty) -> Ty {
        Ty::named("Result", vec![ok, err])
    }
    pub fn list(inner: Ty) -> Ty {
        Ty::named("List", vec![inner])
    }
    pub fn is_option(&self) -> bool {
        matches!(self, Ty::Named(n, _) if &**n == "Option")
    }
    pub fn is_error(&self) -> bool {
        matches!(self, Ty::Error)
    }
    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float)
    }

    /// Clave de despacho de métodos (impls) para este tipo.
    pub fn head_key(&self) -> Option<String> {
        Some(match self {
            Ty::Int => "$int".into(),
            Ty::Float => "$float".into(),
            Ty::Bool => "$bool".into(),
            Ty::Char => "$char".into(),
            Ty::Str => "$string".into(),
            Ty::Nothing => "$nothing".into(),
            Ty::Named(n, _) => n.to_string(),
            Ty::Array(..) => "Array".into(),
            Ty::Tuple(_) => "Tuple".into(),
            Ty::Func(..) => "Fn".into(),
            _ => return None,
        })
    }

    pub fn subst(&self, map: &HashMap<Rc<str>, Ty>) -> Ty {
        match self {
            Ty::Param(p) => map.get(p).cloned().unwrap_or_else(|| self.clone()),
            Ty::Named(n, a) => Ty::Named(n.clone(), a.iter().map(|t| t.subst(map)).collect()),
            Ty::Array(t, n) => Ty::Array(Box::new(t.subst(map)), *n),
            Ty::Tuple(v) => Ty::Tuple(v.iter().map(|t| t.subst(map)).collect()),
            Ty::Ref(t) => Ty::Ref(Box::new(t.subst(map))),
            Ty::Ptr(t) => Ty::Ptr(Box::new(t.subst(map))),
            Ty::Func(p, r) => Ty::Func(p.iter().map(|t| t.subst(map)).collect(), Box::new(r.subst(map))),
            Ty::Union(v) => Ty::Union(v.iter().map(|t| t.subst(map)).collect()),
            _ => self.clone(),
        }
    }

    pub fn contains_var(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Named(_, a) | Ty::Tuple(a) | Ty::Union(a) => a.iter().any(Ty::contains_var),
            Ty::Array(t, _) | Ty::Ref(t) | Ty::Ptr(t) => t.contains_var(),
            Ty::Func(p, r) => p.iter().any(Ty::contains_var) || r.contains_var(),
            _ => false,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int => write!(f, "$int"),
            Ty::Float => write!(f, "$float"),
            Ty::Bool => write!(f, "$bool"),
            Ty::Char => write!(f, "$char"),
            Ty::Str => write!(f, "$string"),
            Ty::Nothing => write!(f, "nothing"),
            Ty::Never => write!(f, "!"),
            Ty::Null => write!(f, "null"),
            Ty::Error => write!(f, "<error>"),
            Ty::Var(_) => write!(f, "_"),
            Ty::Param(p) => write!(f, "{p}"),
            Ty::Named(n, a) => {
                if &**n == "Option" && a.len() == 1 {
                    let inner = a[0].to_string();
                    return if matches!(a[0], Ty::Union(_) | Ty::Func(..)) { write!(f, "({inner})?") } else { write!(f, "{inner}?") };
                }
                let short = n.rsplit("::").next().unwrap_or(n);
                write!(f, "{short}")?;
                if !a.is_empty() {
                    write!(f, "<{}>", a.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", "))?;
                }
                Ok(())
            }
            Ty::Array(t, n) => write!(f, "{t}[{n}]"),
            Ty::Tuple(v) => write!(f, "({})", v.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ")),
            Ty::Ref(t) => write!(f, "&{t}"),
            Ty::Ptr(t) => write!(f, "*{t}"),
            Ty::Func(p, r) => write!(f, "({}) -> {r}", p.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ")),
            Ty::Union(v) => write!(f, "{}", v.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(" | ")),
        }
    }
}
