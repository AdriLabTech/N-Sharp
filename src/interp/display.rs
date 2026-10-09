//! Representación textual de valores (`println`, interpolación, `to_string`).

use super::*;

pub fn fmt_float(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "inf".into() } else { "-inf".into() }
    } else if f == f.trunc() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn escape_into(s: &str, quote: char, out: &mut String) {
    out.push(quote);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
}

impl Interp {
    /// Texto «de usuario»: strings y chars sin comillas; tipos con `Display` usan su `to_string`.
    pub fn display(&mut self, v: &Value) -> R<String> {
        let mut s = String::new();
        self.fmt_value(v, false, &mut s)?;
        Ok(s)
    }

    /// Texto estilo depuración: strings y chars con comillas.
    pub fn debug_str(&mut self, v: &Value) -> R<String> {
        let mut s = String::new();
        self.fmt_value(v, true, &mut s)?;
        Ok(s)
    }

    fn fmt_value(&mut self, v: &Value, debug: bool, out: &mut String) -> R<()> {
        match v {
            Value::Nothing => out.push_str("nothing"),
            Value::Int(i) => out.push_str(&i.to_string()),
            Value::Float(f) => out.push_str(&fmt_float(*f)),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Char(c) => {
                if debug { escape_into(&c.to_string(), '\'', out) } else { out.push(*c) }
            }
            Value::Str(s) => {
                if debug { escape_into(s, '"', out) } else { out.push_str(s) }
            }
            Value::Array(a) | Value::List(a) => {
                out.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    self.fmt_value(x, true, out)?;
                }
                out.push(']');
            }
            Value::Tuple(a) => {
                out.push('(');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    self.fmt_value(x, true, out)?;
                }
                out.push(')');
            }
            Value::Set(m) => {
                out.push('{');
                for (i, (k, _)) in m.entries.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    self.fmt_value(k, true, out)?;
                }
                out.push('}');
            }
            Value::Map(m) => {
                out.push('{');
                for (i, (k, val)) in m.entries.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    self.fmt_value(k, true, out)?;
                    out.push_str(": ");
                    self.fmt_value(val, true, out)?;
                }
                out.push('}');
            }
            Value::Struct(s) => {
                if let Some(defs) = self.lookup_def_set(&s.info.key, "to_string") {
                    if defs[0].decl.body.is_some() {
                        let r = self.call_def(&defs[0].clone(), vec![], Some(Place::temp(v.clone())), Span::default())?;
                        out.push_str(&self.display(&r)?);
                        return Ok(());
                    }
                }
                out.push_str(&s.info.name);
                if !s.vals.is_empty() {
                    out.push_str(" { ");
                    for (i, (n, x)) in s.info.fields.iter().zip(s.vals.iter()).enumerate() {
                        if i > 0 { out.push_str(", "); }
                        out.push_str(n);
                        out.push_str(": ");
                        self.fmt_value(x, true, out)?;
                    }
                    out.push_str(" }");
                }
            }
            Value::Variant(vv) => {
                if vv.info.key != "Option" && vv.info.key != "Result" {
                    if let Some(defs) = self.lookup_def_set(&vv.info.key, "to_string") {
                        if defs[0].decl.body.is_some() {
                            let r = self.call_def(&defs[0].clone(), vec![], Some(Place::temp(v.clone())), Span::default())?;
                            out.push_str(&self.display(&r)?);
                            return Ok(());
                        }
                    }
                }
                out.push_str(&vv.info.variants[vv.idx].0);
                if !vv.vals.is_empty() {
                    out.push('(');
                    for (i, x) in vv.vals.iter().enumerate() {
                        if i > 0 { out.push_str(", "); }
                        self.fmt_value(x, true, out)?;
                    }
                    out.push(')');
                }
            }
            Value::Func(_) => out.push_str("<fn>"),
            Value::Ref(_) | Value::Box(_) | Value::Rc(_) => {
                let d = self.deref_value(v)?;
                self.fmt_value(&d, debug, out)?;
            }
            Value::Ptr(a, o) => out.push_str(&format!("0x{:x}", (a << 20) as i64 + o)),
            Value::Future(_) => out.push_str("<future>"),
            Value::Handle(_) => out.push_str("<thread>"),
            Value::Sender(_) => out.push_str("<sender>"),
            Value::Receiver(_) => out.push_str("<receiver>"),
        }
        Ok(())
    }
}
