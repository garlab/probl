//! A readable rendering of the IR, for tests and debugging.

use crate::ir::*;
use crate::liveness::Liveness;
use probl_syntax::sexpr::percent;
use std::fmt::Write;

/// Render the whole program; with `liveness`, each statement is followed by
/// the variables live after it.
pub fn program(p: &Program, liveness: Option<&Liveness>) -> String {
    let mut out = String::new();
    for (id, f) in p.functions.iter().enumerate() {
        let printer = Printer { p, f, liveness };
        let params: Vec<String> = (0..f.n_params).map(|s| printer.slot(s)).collect();
        let captures: Vec<String> = f
            .captures
            .iter()
            .map(|c| match c.source {
                CaptureSource::Global(g) => {
                    format!("{} <- global {}", printer.slot(c.slot), p.main().slots[g as usize].name)
                }
                CaptureSource::Parent(s) => format!("{} <- parent slot {s}", printer.slot(c.slot)),
            })
            .collect();
        write!(out, "fn#{id} {} ({})", f.name, params.join(", ")).unwrap();
        if !captures.is_empty() {
            write!(out, " captures [{}]", captures.join(", ")).unwrap();
        }
        out.push('\n');
        printer.block(&f.body, 1, &mut out);
    }
    out
}

struct Printer<'a> {
    p: &'a Program,
    f: &'a Function,
    liveness: Option<&'a Liveness>,
}

impl Printer<'_> {
    fn slot(&self, s: SlotId) -> String {
        match self.f.slots.get(s as usize) {
            Some(info) if info.name == "(temporary)" => format!("${s}"),
            Some(info) => format!("{}_{s}", info.name),
            None => format!("?{s}"),
        }
    }

    fn block(&self, b: &Block, depth: usize, out: &mut String) {
        for s in &b.stmts {
            self.stmt(s, depth, out);
        }
    }

    fn line(&self, s: &Stmt, depth: usize, text: String, out: &mut String) {
        let indent = "  ".repeat(depth);
        write!(out, "{indent}{text}").unwrap();
        if let Some(l) = self.liveness {
            if !matches!(
                s.kind,
                StmtKind::If { .. } | StmtKind::Chance { .. } | StmtKind::Loop { .. }
            ) {
                let live: Vec<String> = l.after[s.id as usize].iter().map(|s| self.slot(s)).collect();
                write!(out, "    [live: {}]", live.join(" ")).unwrap();
            }
        }
        out.push('\n');
    }

    fn stmt(&self, s: &Stmt, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);
        match &s.kind {
            StmtKind::Set { place, value } => {
                self.line(s, depth, format!("{} = {}", self.place(place), self.expr(value)), out)
            }
            StmtKind::Draw { place, dist } => {
                self.line(s, depth, format!("{} ~ {}", self.place(place), self.expr(dist)), out)
            }
            StmtKind::Take { place, bag } => self.line(
                s,
                depth,
                format!("{} = {}.take()", self.place(place), self.place(bag)),
                out,
            ),
            StmtKind::Call { dest, callee, args } => {
                let args: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                let callee = match callee {
                    Callee::Fn { func, capture_args } => {
                        let caps: Vec<String> = capture_args.iter().map(|&c| self.slot(c)).collect();
                        let caps = if caps.is_empty() {
                            String::new()
                        } else {
                            format!(" with [{}]", caps.join(", "))
                        };
                        format!("fn#{func} {}{caps}", self.p.functions[*func as usize].name)
                    }
                    Callee::Value(e) => self.expr(e),
                };
                self.line(
                    s,
                    depth,
                    format!("{} = call {callee}({})", self.place(dest), args.join(", ")),
                    out,
                )
            }
            StmtKind::If { cond, then, otherwise } => {
                writeln!(out, "{indent}if {} {{", self.expr(cond)).unwrap();
                self.block(then, depth + 1, out);
                if !otherwise.stmts.is_empty() {
                    writeln!(out, "{indent}}} else {{").unwrap();
                    self.block(otherwise, depth + 1, out);
                }
                writeln!(out, "{indent}}}").unwrap();
            }
            StmtKind::Chance {
                arms,
                otherwise,
                exhaustive,
            } => {
                let kind = if *exhaustive { "chance (value)" } else { "chance" };
                writeln!(out, "{indent}{kind} {{").unwrap();
                for (w, b) in arms {
                    writeln!(out, "{indent}  {} => {{", self.expr(w)).unwrap();
                    self.block(b, depth + 2, out);
                    writeln!(out, "{indent}  }}").unwrap();
                }
                if let Some(b) = otherwise {
                    writeln!(out, "{indent}  else => {{").unwrap();
                    self.block(b, depth + 2, out);
                    writeln!(out, "{indent}  }}").unwrap();
                }
                writeln!(out, "{indent}}}").unwrap();
            }
            StmtKind::Loop { body, bounded } => {
                let head = match self.liveness {
                    Some(l) => {
                        let live: Vec<String> = l.loop_head[s.id as usize].iter().map(|s| self.slot(s)).collect();
                        format!("    [live at head: {}]", live.join(" "))
                    }
                    None => String::new(),
                };
                let kind = if *bounded { "loop (bounded)" } else { "loop" };
                writeln!(out, "{indent}{kind} {{{head}").unwrap();
                self.block(body, depth + 1, out);
                writeln!(out, "{indent}}}").unwrap();
            }
            StmtKind::Break => self.line(s, depth, "break".into(), out),
            StmtKind::Continue => self.line(s, depth, "continue".into(), out),
            StmtKind::Return(e) => self.line(s, depth, format!("return {}", self.expr(e)), out),
            StmtKind::Observe { value, from } => {
                let text = match from {
                    Some(d) => format!("observe {} from {}", self.expr(value), self.expr(d)),
                    None => format!("observe {}", self.expr(value)),
                };
                self.line(s, depth, text, out)
            }
            StmtKind::Report { site, value, key } => {
                let label = &self.p.reports[*site as usize].label;
                let key = key
                    .as_ref()
                    .map(|k| format!(" by {}", self.expr(k)))
                    .unwrap_or_default();
                self.line(s, depth, format!("report {}{key} as {label:?}", self.expr(value)), out)
            }
            StmtKind::Fail { message } => self.line(s, depth, format!("fail {message:?}"), out),
            StmtKind::Check { slot, ty } => self.line(
                s,
                depth,
                format!("check {}: {}", self.slot(*slot), ty.describe(self.p)),
                out,
            ),
        }
    }

    fn place(&self, place: &Place) -> String {
        let mut text = self.slot(place.slot);
        for elem in &place.path {
            match elem {
                PathElem::Field(name) => write!(text, ".{name}").unwrap(),
                PathElem::Index(e) => write!(text, "[{}]", self.expr(e)).unwrap(),
            }
        }
        text
    }

    fn expr(&self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Lit(l) => match l {
                Lit::Unit => "()".into(),
                Lit::Bool(b) => b.to_string(),
                Lit::Int(v) => v.to_string(),
                Lit::Float(v) | Lit::FloatConstant(v) => format!("{v:?}"),
                Lit::Prob(v) => percent(*v),
                Lit::Str(s) => format!("{s:?}"),
                Lit::Dice { count, sides } => format!("{count}d{sides}"),
                Lit::Enum { ty, variant } => {
                    let t = &self.p.enums[*ty as usize];
                    format!("{}.{}", t.name, t.variants[*variant as usize])
                }
            },
            ExprKind::Slot(s) => self.slot(*s),
            ExprKind::Unary(op, inner) => {
                let op = match op {
                    probl_syntax::ast::UnOp::Neg => "-",
                    probl_syntax::ast::UnOp::Not => "not",
                    probl_syntax::ast::UnOp::Typeof => "typeof",
                };
                format!("({op} {})", self.expr(inner))
            }
            ExprKind::Binary(op, a, b) => {
                format!("({} {} {})", op.symbol(), self.expr(a), self.expr(b))
            }
            ExprKind::List(items) => {
                let items: Vec<String> = items.iter().map(|i| self.expr(i)).collect();
                format!("[{}]", items.join(", "))
            }
            ExprKind::Map(entries) => {
                let entries: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", self.expr(k), self.expr(v)))
                    .collect();
                format!(
                    "[{}]",
                    if entries.is_empty() {
                        ":".into()
                    } else {
                        entries.join(", ")
                    }
                )
            }
            ExprKind::Record { ty, fields } => {
                let name = ty
                    .map(|t| format!("{} ", self.p.records[t as usize].name))
                    .unwrap_or_default();
                let fields: Vec<String> = fields.iter().map(|(n, v)| format!("{n}: {}", self.expr(v))).collect();
                format!("{name}{{ {} }}", fields.join(", "))
            }
            ExprKind::Field(inner, name) => format!("{}.{name}", self.expr(inner)),
            ExprKind::Index(a, b) => format!("{}[{}]", self.expr(a), self.expr(b)),
            ExprKind::With(base, fields) => {
                let fields: Vec<String> = fields.iter().map(|(n, v)| format!("{n}: {}", self.expr(v))).collect();
                format!("({} with {{ {} }})", self.expr(base), fields.join(", "))
            }
            ExprKind::Builtin { func, args, .. } => {
                let args: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                format!("{}({})", func.name(), args.join(", "))
            }
            ExprKind::Closure { func, capture_args } => {
                let caps: Vec<String> = capture_args.iter().map(|&c| self.slot(c)).collect();
                format!("closure fn#{func} [{}]", caps.join(", "))
            }
            ExprKind::Simulate { func, capture_args } => {
                let caps: Vec<String> = capture_args.iter().map(|&c| self.slot(c)).collect();
                format!("simulate fn#{func} [{}]", caps.join(", "))
            }
            ExprKind::Interp(parts) => {
                let parts: Vec<String> = parts
                    .iter()
                    .map(|p| match p {
                        InterpPart::Lit(s) => format!("{s:?}"),
                        InterpPart::Expr(e) => self.expr(e),
                    })
                    .collect();
                format!("str({})", parts.join(" "))
            }
            ExprKind::Input(i) => {
                let input = &self.p.inputs[*i as usize];
                format!("read#{i}({:?}, {})", input.path, input.format.name())
            }
        }
    }
}
