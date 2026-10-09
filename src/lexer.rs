//! Lexer de N#: UTF-8, comentarios anidables, strings con interpolación, sin indentación significativa.

use crate::diag::{Diagnostic, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// Rango de bytes (en el fichero) de la expresión interpolada `{...}`.
    Expr(usize, usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Kw(&'static str),
    /// Tipo primitivo: `$int`, `$float`, ...
    Prim(&'static str),
    /// `$nombre` que no es un primitivo (variable de macro); fuera de `macro` es un error.
    MacroVar(String),
    Int(i64),
    Float(f64),
    Char(char),
    Str(Vec<StrPart>),
    Sym(&'static str),
    /// Solo con `keep_trivia`: comentario completo (incluye `#`, `!# #!` o `///`).
    Comment(String),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// Comentarios `///` inmediatamente anteriores.
    pub doc: Option<String>,
}

pub const KEYWORDS: &[&str] = &[
    "fn", "if", "else", "loop", "while", "break", "return", "struct", "enum", "trait", "impl", "match", "import",
    "pub", "const", "static", "true", "false", "null", "nothing", "unsafe", "async", "await", "self", "Self", "as",
    "type", "test", "assert", "exit", "panic", "let",
];

const PRIMS: &[&str] = &["int", "float", "bool", "char", "string", "nothing"];

const SYMS: &[&str] = &[
    "===", "==", "!=", "<=", ">=", "<<", ">>", "&&", "||", "^^", "++", "--", "+=", "-=", "*=", "/=", "%=", "->", "=>",
    ":=", "..", "+", "-", "*", "/", "%", "^", "&", "|", "~", "!", "<", ">", "=", "?", ":", ";", ",", ".", "(", ")", "[",
    "]", "{", "}", "@",
];

pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    end: usize,
    file: u32,
    keep_trivia: bool,
    pub diags: Vec<Diagnostic>,
}

pub fn lex(file: u32, src: &str, keep_trivia: bool) -> (Vec<Token>, Vec<Diagnostic>) {
    lex_range(file, src, 0, src.len(), keep_trivia)
}

/// Lexea solo `src[lo..hi]` conservando los offsets del fichero (para interpolaciones).
pub fn lex_range(file: u32, src: &str, lo: usize, hi: usize, keep_trivia: bool) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lx = Lexer { src, pos: lo, end: hi, file, keep_trivia, diags: vec![] };
    let toks = lx.run();
    (toks, lx.diags)
}

impl<'a> Lexer<'a> {
    fn span(&self, lo: usize, hi: usize) -> Span {
        Span::new(self.file, lo, hi)
    }
    fn err(&mut self, lo: usize, hi: usize, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(self.span(lo, hi.max(lo + 1)), msg));
    }
    fn peek(&self) -> Option<char> {
        if self.pos < self.end { self.src[self.pos..self.end].chars().next() } else { None }
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        self.src[self.pos..self.end].chars().nth(n)
    }
    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..self.end].starts_with(s)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn run(&mut self) -> Vec<Token> {
        let mut out: Vec<Token> = Vec::new();
        let mut pending_doc: Vec<String> = Vec::new();
        loop {
            // espacios
            while let Some(c) = self.peek() {
                if c.is_whitespace() { self.pos += c.len_utf8(); } else { break; }
            }
            if self.pos >= self.end {
                break;
            }
            let start = self.pos;
            // comentarios
            if self.starts_with("///") && !self.starts_with("////") {
                while let Some(c) = self.peek() { if c == '\n' { break; } self.pos += c.len_utf8(); }
                let text = &self.src[start..self.pos];
                if self.keep_trivia {
                    out.push(Token { tok: Tok::Comment(text.to_string()), span: self.span(start, self.pos), doc: None });
                } else {
                    pending_doc.push(text[3..].strip_prefix(' ').unwrap_or(&text[3..]).trim_end().to_string());
                }
                continue;
            }
            if self.starts_with("!#") {
                self.pos += 2;
                let mut depth = 1;
                while depth > 0 {
                    if self.pos >= self.end {
                        self.err(start, start + 2, "comentario multilínea sin cerrar");
                        break;
                    }
                    if self.starts_with("!#") { depth += 1; self.pos += 2; }
                    else if self.starts_with("#!") { depth -= 1; self.pos += 2; }
                    else { self.bump(); }
                }
                if self.keep_trivia {
                    out.push(Token { tok: Tok::Comment(self.src[start..self.pos].to_string()), span: self.span(start, self.pos), doc: None });
                }
                continue;
            }
            if self.peek() == Some('#') {
                while let Some(c) = self.peek() { if c == '\n' { break; } self.pos += c.len_utf8(); }
                if self.keep_trivia {
                    out.push(Token { tok: Tok::Comment(self.src[start..self.pos].to_string()), span: self.span(start, self.pos), doc: None });
                }
                continue;
            }
            let c = self.peek().unwrap();
            let tok = if c.is_alphabetic() || c == '_' {
                self.ident()
            } else if c == '$' {
                self.prim()
            } else if c.is_ascii_digit() {
                self.number()
            } else if c == '"' {
                self.string()
            } else if c == '\'' {
                self.char_lit()
            } else {
                self.symbol()
            };
            let Some(tok) = tok else { continue };
            let doc = if pending_doc.is_empty() { None } else { Some(std::mem::take(&mut pending_doc).join("\n")) };
            out.push(Token { tok, span: self.span(start, self.pos), doc });
        }
        out.push(Token { tok: Tok::Eof, span: self.span(self.end, self.end), doc: None });
        out
    }

    fn ident(&mut self) -> Option<Tok> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' { self.pos += c.len_utf8(); } else { break; }
        }
        let word = &self.src[start..self.pos];
        if word.chars().all(|c| c == '_') {
            // `_` y `__`...: reservados como wildcard; el parser los trata como patrón.
            return Some(Tok::Ident(word.to_string()));
        }
        if word == "as" && self.peek() == Some('!') && !matches!(self.peek_at(1), Some('=') | Some('#')) {
            self.pos += 1;
            return Some(Tok::Kw("as!"));
        }
        if let Some(kw) = KEYWORDS.iter().find(|k| **k == word) {
            return Some(Tok::Kw(kw));
        }
        Some(Tok::Ident(word.to_string()))
    }

    fn prim(&mut self) -> Option<Tok> {
        let start = self.pos;
        self.pos += 1;
        let ws = self.pos;
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' { self.pos += c.len_utf8(); } else { break; }
        }
        let word = &self.src[ws..self.pos];
        match PRIMS.iter().find(|p| **p == word) {
            Some(p) => Some(Tok::Prim(p)),
            None if word.is_empty() => {
                self.err(start, self.pos, "`$` está reservado para los tipos primitivos");
                None
            }
            None => Some(Tok::MacroVar(word.to_string())),
        }
    }

    fn number(&mut self) -> Option<Tok> {
        let start = self.pos;
        let radix = if self.starts_with("0x") { Some(16) } else if self.starts_with("0b") { Some(2) } else { None };
        if let Some(radix) = radix {
            self.pos += 2;
            let ds = self.pos;
            while let Some(c) = self.peek() {
                if c.is_digit(radix) || c == '_' { self.pos += 1; } else { break; }
            }
            let digits: String = self.src[ds..self.pos].chars().filter(|c| *c != '_').collect();
            if digits.is_empty() {
                self.err(start, self.pos, "literal numérico sin dígitos");
                return None;
            }
            return match i64::from_str_radix(&digits, radix).or_else(|_| u64::from_str_radix(&digits, radix).map(|v| v as i64)) {
                Ok(v) => Some(Tok::Int(v)),
                Err(_) => { self.err(start, self.pos, "literal entero fuera de rango"); None }
            };
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' { self.pos += 1; } else { break; }
        }
        let is_float = self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit());
        if is_float {
            self.pos += 1;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '_' { self.pos += 1; } else { break; }
            }
            if matches!(self.peek(), Some('e') | Some('E')) && self.peek_at(1).is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+') {
                self.err(start, self.pos + 1, "N# v0.1 no admite notación científica");
            }
            let txt: String = self.src[start..self.pos].chars().filter(|c| *c != '_').collect();
            return Some(Tok::Float(txt.parse().unwrap_or(0.0)));
        }
        if matches!(self.peek(), Some('e') | Some('E')) && self.peek_at(1).is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+') {
            self.err(start, self.pos + 1, "N# v0.1 no admite notación científica");
        }
        let txt: String = self.src[start..self.pos].chars().filter(|c| *c != '_').collect();
        if txt.len() > 1 && txt.starts_with('0') && txt.chars().all(|c| c.is_ascii_digit()) && !self.src[start..self.pos].contains('_') {
            self.err(start, self.pos, "N# v0.1 no admite literales octales");
            return None;
        }
        match txt.parse::<i64>() {
            Ok(v) => Some(Tok::Int(v)),
            Err(_) => { self.err(start, self.pos, "literal entero fuera de rango de $int (64 bits)"); None }
        }
    }

    fn escape(&mut self, quote: char) -> Option<char> {
        let start = self.pos - 1; // la barra
        let c = self.bump()?;
        Some(match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            '{' if quote == '"' => '{',
            '}' if quote == '"' => '}',
            'u' => {
                if self.peek() != Some('{') {
                    self.err(start, self.pos, "escape `\\u` requiere `\\u{XXXX}`");
                    return None;
                }
                self.pos += 1;
                let hs = self.pos;
                while let Some(c) = self.peek() { if c == '}' { break; } self.pos += c.len_utf8(); }
                let hex = &self.src[hs..self.pos];
                if self.peek() == Some('}') { self.pos += 1; }
                match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => ch,
                    None => { self.err(start, self.pos, "escape unicode inválido"); return None; }
                }
            }
            other => {
                self.err(start, self.pos, format!("escape desconocido `\\{other}`"));
                return None;
            }
        })
    }

    fn char_lit(&mut self) -> Option<Tok> {
        let start = self.pos;
        self.pos += 1;
        let c = match self.bump() {
            Some('\\') => self.escape('\'')?,
            Some('\'') | None => { self.err(start, self.pos, "literal de carácter vacío"); return None; }
            Some(c) => c,
        };
        if self.peek() == Some('\'') {
            self.pos += 1;
            Some(Tok::Char(c))
        } else {
            self.err(start, self.pos, "literal de carácter sin cerrar (¿un string usa comillas dobles?)");
            None
        }
    }

    fn string(&mut self) -> Option<Tok> {
        let start = self.pos;
        let multiline = self.starts_with("\"\"\"");
        self.pos += if multiline { 3 } else { 1 };
        if multiline {
            // El salto de línea inmediatamente posterior a `"""` de apertura no forma parte del contenido.
            if self.starts_with("\r\n") { self.pos += 2; } else if self.starts_with("\n") { self.pos += 1; }
        }
        let mut parts: Vec<StrPart> = Vec::new();
        let mut cur = String::new();
        loop {
            if self.pos >= self.end {
                self.err(start, start + 1, "string sin cerrar");
                return None;
            }
            if multiline && self.starts_with("\"\"\"") { self.pos += 3; break; }
            let c = self.bump().unwrap();
            match c {
                '"' if !multiline => break,
                '\n' if !multiline => {
                    self.err(start, self.pos - 1, "string sin cerrar (usa \"\"\" para varias líneas)");
                    return None;
                }
                '\\' => match self.escape('"') { Some(ch) => cur.push(ch), None => {} },
                '{' => {
                    let es = self.pos;
                    let mut depth = 1;
                    while depth > 0 {
                        let Some(c) = self.peek() else { break };
                        match c {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            '"' => { self.skip_inner_string(); continue; }
                            _ => {}
                        }
                        if depth == 0 { break; }
                        self.pos += c.len_utf8();
                    }
                    if depth != 0 {
                        self.err(es - 1, es, "interpolación `{` sin cerrar en el string");
                        return None;
                    }
                    let ee = self.pos;
                    self.pos += 1;
                    if !cur.is_empty() { parts.push(StrPart::Lit(std::mem::take(&mut cur))); }
                    if self.src[es..ee].trim().is_empty() {
                        self.err(es - 1, ee + 1, "interpolación vacía; usa `\\{` para una llave literal");
                    } else {
                        parts.push(StrPart::Expr(es, ee));
                    }
                }
                '}' => {
                    self.err(self.pos - 1, self.pos, "`}` suelto en un string; usa `\\}` para una llave literal");
                }
                c => cur.push(c),
            }
        }
        if !cur.is_empty() || parts.is_empty() { parts.push(StrPart::Lit(cur)); }
        Some(Tok::Str(parts))
    }

    fn skip_inner_string(&mut self) {
        self.pos += 1;
        while let Some(c) = self.peek() {
            self.pos += c.len_utf8();
            if c == '\\' { if let Some(n) = self.peek() { self.pos += n.len_utf8(); } }
            else if c == '"' { break; }
        }
    }

    fn symbol(&mut self) -> Option<Tok> {
        let start = self.pos;
        for s in SYMS {
            if self.starts_with(s) {
                self.pos += s.len();
                return Some(Tok::Sym(s));
            }
        }
        let c = self.bump().unwrap();
        self.err(start, self.pos, format!("carácter inesperado `{c}`"));
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        let (t, d) = lex(0, s, false);
        assert!(d.is_empty(), "{d:?}");
        t.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn basics() {
        assert_eq!(toks("let x $int = 0xFF;")[..5], [Tok::Kw("let"), Tok::Ident("x".into()), Tok::Prim("int"), Tok::Sym("="), Tok::Int(255)]);
        assert_eq!(toks("1_000 0b101 3.14")[..3], [Tok::Int(1000), Tok::Int(5), Tok::Float(3.14)]);
        assert_eq!(toks("a ^^ b ^ c === d")[1], Tok::Sym("^^"));
    }

    #[test]
    fn nested_comments_and_ranges() {
        assert_eq!(toks("1 !# a !# b #! c #! 2")[..2], [Tok::Int(1), Tok::Int(2)]);
        assert_eq!(toks("1..4")[..3], [Tok::Int(1), Tok::Sym(".."), Tok::Int(4)]);
    }

    #[test]
    fn strings() {
        let t = toks("\"Hola, {name}!\"");
        assert!(matches!(&t[0], Tok::Str(p) if p.len() == 3));
        let t = toks("\"\"\"\nHola\nMundo\n\"\"\"");
        assert_eq!(t[0], Tok::Str(vec![StrPart::Lit("Hola\nMundo\n".into())]));
    }

    #[test]
    fn errors() {
        let (t, _) = lex(0, "$var", false);
        assert_eq!(t[0].tok, Tok::MacroVar("var".into()));
        let (_, d) = lex(0, "017", false);
        assert!(!d.is_empty());
    }
}
