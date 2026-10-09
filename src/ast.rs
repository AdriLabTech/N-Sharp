//! AST de N#.

use crate::diag::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Prim {
    Int,
    Float,
    Bool,
    Char,
    Str,
    Nothing,
}

impl Prim {
    pub fn name(self) -> &'static str {
        match self {
            Prim::Int => "$int",
            Prim::Float => "$float",
            Prim::Bool => "$bool",
            Prim::Char => "$char",
            Prim::Str => "$string",
            Prim::Nothing => "$nothing",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    Prim(Prim),
    /// `a.b.Name<args>`; el último segmento es el nombre del tipo.
    Named { path: Vec<Ident>, args: Vec<TypeExpr> },
    Ref(Box<TypeExpr>),
    Ptr(Box<TypeExpr>),
    Optional(Box<TypeExpr>),
    Union(Vec<TypeExpr>),
    Func { params: Vec<TypeExpr>, ret: Box<TypeExpr> },
    Tuple(Vec<TypeExpr>),
    Array { elem: Box<TypeExpr>, len: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    Ref,
    Deref,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    RefEq,
    And,
    Or,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+", Sub => "-", Mul => "*", Div => "/", Rem => "%", Pow => "^", BitAnd => "&", BitOr => "|",
            BitXor => "^^", Shl => "<<", Shr => ">>", Eq => "==", Ne => "!=", Lt => "<", Gt => ">", Le => "<=",
            Ge => ">=", RefEq => "===", And => "&&", Or => "||",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StrPiece {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub enum LoopKind {
    /// `loop (init; cond; update)`
    C { init: Option<Box<Stmt>>, cond: Option<Box<Expr>>, update: Option<Box<Expr>> },
    /// `loop (var [type] : iterable)`
    Each { var: Ident, ty: Option<TypeExpr>, iter: Box<Expr> },
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ClosureParam {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    Str(Vec<StrPiece>),
    Null,
    Ident(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `a = b` (op = None) o `a += b`.
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    IncDec { target: Box<Expr>, inc: bool, prefix: bool },
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call { callee: Box<Expr>, args: Vec<Expr> },
    Field(Box<Expr>, Ident),
    Index(Box<Expr>, Box<Expr>),
    Slice { base: Box<Expr>, lo: Option<Box<Expr>>, hi: Option<Box<Expr>> },
    Array(Vec<Expr>),
    Tuple(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Set(Vec<Expr>),
    /// `key` lo rellena el checker (nombre calificado del struct).
    StructLit { path: Vec<Ident>, fields: Vec<(Ident, Expr)>, key: String },
    If { cond: Box<Expr>, then: Block, els: Option<Box<Expr>> },
    Match { scrut: Box<Expr>, arms: Vec<MatchArm> },
    Closure { params: Vec<ClosureParam>, ret: Option<TypeExpr>, body: Box<Expr> },
    Block(Block),
    Unsafe(Block),
    Cast { expr: Box<Expr>, ty: TypeExpr, lossy: bool },
    /// Operador `?` de propagación.
    Try(Box<Expr>),
    Await(Box<Expr>),
    Return(Option<Box<Expr>>),
    Break,
    While { cond: Box<Expr>, body: Block },
    Loop { kind: LoopKind, body: Block },
    Asm(String),
    /// Insertado por el checker: coerción implícita `T` → `Option<T>`.
    SomeWrap(Box<Expr>),
    /// Resuelto por el checker: función, constante o static de un módulo.
    Global { module: usize, name: String },
    /// Resuelto por el checker: función asociada `Tipo.nombre`.
    TypeStatic { key: String, name: String },
    /// Resuelto por el checker: variante de enum (con o sin datos).
    VariantRef { enum_key: String, variant: String },
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Expresión final sin `;`: valor del bloque.
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct DestructTarget {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    /// `walrus`: escrito como `x := e;` (en vez de `let x = e;`); solo lo usa el formateador.
    Let { name: Ident, ty: Option<TypeExpr>, init: Option<Expr>, walrus: bool },
    /// `(x $int, y $int) = point;`
    Destructure { targets: Vec<DestructTarget>, init: Expr },
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Wild(Span),
    Bind { name: Ident, ty: Option<TypeExpr> },
    Lit(Box<Expr>),
    Variant { path: Vec<Ident>, args: Option<Vec<Pattern>>, span: Span, res: Option<(String, String)> },
    Tuple(Vec<Pattern>, Span),
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wild(s) | Pattern::Tuple(_, s) => *s,
            Pattern::Bind { name, .. } => name.span,
            Pattern::Lit(e) => e.span,
            Pattern::Variant { span, .. } => *span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GenericParam {
    pub name: Ident,
    pub bounds: Vec<TypeExpr>,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug)]
pub struct Attribute {
    pub name: Ident,
    pub args: Vec<Ident>,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub variadic: bool,
    pub ret: TypeExpr,
    pub body: Option<Block>,
    pub is_pub: bool,
    pub is_unsafe: bool,
    pub is_async: bool,
    pub is_extern: bool,
    pub doc: Option<String>,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub is_pub: bool,
}

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<FieldDecl>,
    pub is_pub: bool,
    pub doc: Option<String>,
    pub attrs: Vec<Attribute>,
}

#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: Ident,
    pub fields: Vec<TypeExpr>,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub variants: Vec<VariantDecl>,
    pub is_pub: bool,
    pub doc: Option<String>,
    pub attrs: Vec<Attribute>,
}

#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub methods: Vec<FnDecl>,
    pub is_pub: bool,
    pub doc: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ImplDecl {
    /// Rellenados por el checker: clave del tipo destino y del trait.
    pub key: String,
    pub trait_key: Option<String>,
    pub generics: Vec<GenericParam>,
    pub trait_: Option<TypeExpr>,
    pub target: TypeExpr,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ImportPath {
    pub path: Vec<Ident>,
    pub wildcard: bool,
    pub alias: Option<Ident>,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub value: Expr,
    pub is_pub: bool,
    pub doc: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AliasDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub ty: TypeExpr,
    pub is_pub: bool,
    pub doc: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ItemKind {
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Trait(TraitDecl),
    Impl(ImplDecl),
    Import(Vec<ImportPath>),
    Const(ConstDecl),
    Static(ConstDecl),
    Alias(AliasDecl),
    Test { name: String, body: Block },
    Bench { name: String, body: Block },
    Extern { abi: String, fns: Vec<FnDecl> },
    Macro { name: Ident },
    /// Código global (script mode).
    Stmt(Stmt),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Module {
    pub items: Vec<Item>,
}

impl Block {
    /// Recorre en profundidad todas las expresiones del bloque (incluidas las de sentencias).
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        for s in &self.stmts {
            match &s.kind {
                StmtKind::Let { init, .. } => {
                    if let Some(e) = init { e.walk(f); }
                }
                StmtKind::Destructure { init, .. } => init.walk(f),
                StmtKind::Expr(e) => e.walk(f),
            }
        }
        if let Some(t) = &self.tail {
            t.walk(f);
        }
    }
}

impl Expr {
    /// Visita esta expresión y todas las subexpresiones (preorden).
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        use ExprKind::*;
        match &self.kind {
            Int(_) | Float(_) | Bool(_) | Char(_) | Null | Ident(_) | Break | Asm(_) | Global { .. } | TypeStatic { .. }
            | VariantRef { .. } => {}
            Str(parts) => {
                for p in parts {
                    if let StrPiece::Expr(e) = p { e.walk(f); }
                }
            }
            Unary(_, e) | Try(e) | Await(e) | SomeWrap(e) => e.walk(f),
            Cast { expr, .. } => expr.walk(f),
            Field(e, _) => e.walk(f),
            IncDec { target, .. } => target.walk(f),
            Binary(_, a, b) | Assign(_, a, b) | Index(a, b) => { a.walk(f); b.walk(f); }
            Ternary(a, b, c) => { a.walk(f); b.walk(f); c.walk(f); }
            Call { callee, args } => { callee.walk(f); for a in args { a.walk(f); } }
            Slice { base, lo, hi } => {
                base.walk(f);
                if let Some(e) = lo { e.walk(f); }
                if let Some(e) = hi { e.walk(f); }
            }
            Array(v) | Tuple(v) | Set(v) => for e in v { e.walk(f); },
            Map(v) => for (k, e) in v { k.walk(f); e.walk(f); },
            StructLit { fields, .. } => for (_, e) in fields { e.walk(f); },
            If { cond, then, els } => {
                cond.walk(f);
                then.walk(f);
                if let Some(e) = els { e.walk(f); }
            }
            Match { scrut, arms } => {
                scrut.walk(f);
                for a in arms {
                    if let Some(g) = &a.guard { g.walk(f); }
                    a.body.walk(f);
                }
            }
            Closure { body, .. } => body.walk(f),
            Block(b) | Unsafe(b) => b.walk(f),
            Return(e) => if let Some(e) = e { e.walk(f); },
            While { cond, body } => { cond.walk(f); body.walk(f); }
            Loop { kind, body } => {
                match kind {
                    LoopKind::C { init, cond, update } => {
                        if let Some(s) = init {
                            match &s.kind {
                                StmtKind::Let { init: Some(e), .. } => e.walk(f),
                                StmtKind::Expr(e) => e.walk(f),
                                _ => {}
                            }
                        }
                        if let Some(e) = cond { e.walk(f); }
                        if let Some(e) = update { e.walk(f); }
                    }
                    LoopKind::Each { iter, .. } => iter.walk(f),
                }
                body.walk(f);
            }
        }
    }
}
