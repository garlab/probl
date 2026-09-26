//! Random programs for differential testing (audit finding I4).
//!
//! Every program compiles and stays inside the part of the language the
//! oracle implements. The programs mix the things the semantics has to get
//! right: operands that assign variables, calls with splits and observations
//! inside, aliases of values and events, distributions used as recipes,
//! repeated and impossible observations, loops that break and continue,
//! `simulate`, and distributions with a single outcome.

/// SplitMix64: small, and good enough to pick program shapes.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number from 0 to `n - 1`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// True `percent` times out of 100.
    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// What an expression gives in each world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Int,
    /// A fact.
    Bool,
    Prob,
    /// An int, or a distribution of ints.
    DInt,
    /// A fact, or a distribution of facts.
    DBool,
    /// A list of ints of this length.
    List(usize),
}

impl Kind {
    /// Whether a value of this kind can be used where `want` is expected.
    fn fits(self, want: Kind) -> bool {
        self == want || matches!((self, want), (Kind::Int, Kind::DInt) | (Kind::Bool, Kind::DBool))
    }
}

#[derive(Clone, Debug)]
struct Var {
    name: String,
    kind: Kind,
    /// Declared with `var`, and not a loop counter.
    assignable: bool,
}

#[derive(Clone, Debug)]
struct Func {
    name: String,
    params: Vec<Kind>,
    ret: Kind,
    /// Whether its body may observe (so it can't be called after a report).
    observes: bool,
}

/// Where code is being generated.
#[derive(Clone, Debug)]
struct Ctx {
    /// Variables of the current function or `simulate` block, innermost
    /// scope last.
    scopes: Vec<Vec<Var>>,
    /// Variables that can be read but not assigned: the top-level ones in a
    /// function, the enclosing ones in `simulate`.
    outer: Vec<Var>,
    /// The kind a function returns, inside one.
    ret: Option<Kind>,
    /// Loops of the current function around this point.
    loops: usize,
    /// Whether `observe`, and calls to functions that observe, may appear.
    observe: bool,
    /// How many of the functions may be called: in a function, the earlier
    /// ones.
    callable: usize,
    /// In a recursive function: the function, and its parameter that counts
    /// down, which each recursive call passes one less.
    recursive: Option<(Func, String)>,
}

struct Gen {
    rng: Rng,
    names: usize,
    funcs: Vec<Func>,
    ctx: Ctx,
    reports: usize,
}

/// A random program. The same seed always gives the same program.
pub fn program(seed: u64) -> String {
    let mut g = Gen {
        rng: Rng::new(seed),
        names: 0,
        funcs: Vec::new(),
        ctx: Ctx {
            scopes: vec![Vec::new()],
            outer: Vec::new(),
            ret: None,
            loops: 0,
            observe: true,
            callable: 0,
            recursive: None,
        },
        reports: 0,
    };
    g.program()
}

const PROBS: &[&str] = &["0%", "1%", "10%", "12.5%", "25%", "30%", "50%", "75%", "90%", "100%"];
const DICE: &[&str] = &[
    "d1",
    "d2",
    "d2",
    "d3",
    "d3",
    "d4",
    "d6",
    "2d3",
    "one_of([1, 2, 4])",
    "one_of([3])",
];
const COMPARE: &[&str] = &["<", "<=", ">", ">=", "==", "!="];

impl Gen {
    fn program(&mut self) -> String {
        let mut out = String::new();

        // Top-level variables that the functions can read.
        for _ in 0..1 + self.rng.below(3) {
            out += &self.let_stmt();
            out.push('\n');
        }
        let globals = self.ctx.scopes[0].clone();

        for i in 0..self.rng.below(4) {
            out += &self.function(i, &globals);
        }
        self.ctx.callable = self.funcs.len();

        // The model: splits and evidence.
        for _ in 0..1 + self.rng.below(5) {
            out += &self.stmt(0, 2);
            out.push('\n');
        }

        // Then what to report: no observation may follow a report.
        self.ctx.observe = false;
        for _ in 0..1 + self.rng.below(4) {
            out += &self.report_stmt();
            out.push('\n');
        }
        out
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.names += 1;
        format!("{prefix}{}", self.names)
    }

    fn declare(&mut self, name: &str, kind: Kind, assignable: bool) {
        self.ctx.scopes.last_mut().unwrap().push(Var {
            name: name.to_string(),
            kind,
            assignable,
        });
    }

    fn visible(&self) -> Vec<Var> {
        self.ctx
            .outer
            .iter()
            .chain(self.ctx.scopes.iter().flatten())
            .cloned()
            .collect()
    }

    fn vars_of(&self, kind: Kind) -> Vec<String> {
        self.visible()
            .into_iter()
            .filter(|v| v.kind.fits(kind))
            .map(|v| v.name)
            .collect()
    }

    fn assignable(&self) -> Vec<Var> {
        self.ctx
            .scopes
            .iter()
            .flatten()
            .filter(|v| v.assignable)
            .cloned()
            .collect()
    }

    fn some_kind(&mut self) -> Kind {
        *self.rng.pick(&[
            Kind::Int,
            Kind::Int,
            Kind::Int,
            Kind::Bool,
            Kind::Bool,
            Kind::Prob,
            Kind::DInt,
            Kind::DInt,
            Kind::DBool,
            Kind::List(2),
            Kind::List(3),
        ])
    }

    fn small_int(&mut self) -> String {
        self.rng.below(7).to_string()
    }

    fn prob(&mut self) -> String {
        self.rng.pick(PROBS).to_string()
    }

    // ── Functions ────────────────────────────────────────────────────────

    fn function(&mut self, index: usize, globals: &[Var]) -> String {
        let name = format!("f{}", index + 1);
        // A recursive function counts down its first parameter.
        let recursive = self.rng.chance(20);
        let mut params: Vec<(String, Kind)> = Vec::new();
        if recursive {
            params.push((self.fresh("k"), Kind::Int));
        }
        for _ in 0..self.rng.below(if recursive { 2 } else { 3 }) {
            let kind = self.some_kind();
            params.push((self.fresh("a"), kind));
        }
        let ret = self.some_kind();
        let observes = self.rng.chance(40);
        let this = Func {
            name: name.clone(),
            params: params.iter().map(|(_, k)| *k).collect(),
            ret,
            observes,
        };
        let saved = std::mem::replace(
            &mut self.ctx,
            Ctx {
                scopes: vec![
                    params
                        .iter()
                        .map(|(n, k)| Var {
                            name: n.clone(),
                            kind: *k,
                            assignable: false,
                        })
                        .collect(),
                    Vec::new(),
                ],
                outer: globals.to_vec(),
                ret: Some(ret),
                loops: 0,
                observe: observes,
                callable: index,
                recursive: None,
            },
        );
        let mut body = String::new();
        if recursive {
            let k = params[0].0.clone();
            let base = self.expr(ret, 1);
            body += &format!("  if {k} <= 0 {{ return {base} }}\n");
            self.ctx.recursive = Some((this.clone(), k));
        }
        for _ in 0..self.rng.below(4) {
            body += &self.stmt(1, 1);
            body.push('\n');
        }
        if self.rng.chance(15) {
            body += "  print(\"called\")\n";
        }
        let value = self.expr(ret, 2);
        body += &format!("  {value}\n");
        self.ctx = saved;
        self.funcs.push(this);
        let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
        format!("fn {name}({}) {{\n{body}}}\n", names.join(", "))
    }

    // ── Statements ───────────────────────────────────────────────────────

    /// A statement at `indent`; `depth` limits how deeply blocks nest.
    fn stmt(&mut self, indent: usize, depth: u32) -> String {
        let pad = "  ".repeat(indent);
        loop {
            let s = match self.rng.below(100) {
                0..=19 => Some(self.let_stmt()),
                20..=36 => self.assign_stmt(),
                37..=46 if depth > 0 => Some(self.if_stmt(indent, depth)),
                47..=52 if depth > 0 => Some(self.chance_stmt(indent, depth)),
                53..=57 if depth > 0 => Some(self.match_stmt(indent, depth)),
                58..=68 if depth > 0 => Some(self.loop_stmt(indent, depth)),
                69..=80 => self.observe_stmt(),
                81..=86 => self.flow_stmt(),
                87..=92 => self.call_stmt(),
                _ => None,
            };
            if let Some(s) = s {
                return format!("{pad}{s}");
            }
        }
    }

    fn let_stmt(&mut self) -> String {
        let name = self.fresh("x");
        let assignable = self.rng.chance(40);
        let word = if assignable { "var" } else { "let" };
        if self.rng.chance(8) {
            // A probability that is itself uncertain: `simulate` keeps its
            // distribution, and drawing from it gives one probability.
            let e = self.simulate(Kind::Prob, 1);
            self.declare(&name, Kind::Prob, assignable);
            return format!("{word} {name} ~ {e}");
        }
        if self.rng.chance(35) {
            let (from, kind) = if self.rng.chance(70) {
                (Kind::DInt, Kind::Int)
            } else {
                (Kind::DBool, Kind::Bool)
            };
            // Drawing from a probability is an error, rarely on purpose.
            let e = if self.rng.chance(1) {
                self.prob()
            } else {
                self.expr(from, 2)
            };
            self.declare(&name, kind, assignable);
            return format!("{word} {name} ~ {e}");
        }
        let kind = self.some_kind();
        let e = self.expr(kind, 2);
        self.declare(&name, kind, assignable);
        format!("{word} {name} = {e}")
    }

    fn assign_stmt(&mut self) -> Option<String> {
        let targets = self.assignable();
        if targets.is_empty() {
            return None;
        }
        let v = self.rng.pick(&targets).clone();
        let name = v.name;
        Some(match v.kind {
            Kind::Int => match self.rng.below(6) {
                0 => format!("{name} = {}", self.expr(Kind::Int, 2)),
                1 => format!("{name} ~ {}", self.expr(Kind::DInt, 2)),
                2 | 3 => format!("{name} += {}", self.expr(Kind::Int, 2)),
                4 => format!("{name} -= {}", self.expr(Kind::Int, 2)),
                _ => format!("{name} *= {}", self.rng.below(3)),
            },
            Kind::Bool => match self.rng.below(2) {
                0 => format!("{name} = {}", self.expr(Kind::Bool, 2)),
                _ => format!("{name} ~ {}", self.expr(Kind::DBool, 2)),
            },
            Kind::DInt => match self.rng.below(2) {
                0 => format!("{name} = {}", self.expr(Kind::DInt, 2)),
                _ => format!("{name} += {}", self.expr(Kind::Int, 1)),
            },
            Kind::List(n) if n > 0 && self.rng.chance(70) => {
                let i = self.rng.below(n);
                match self.rng.below(2) {
                    0 => format!("{name}[{i}] = {}", self.expr(Kind::Int, 2)),
                    _ => format!("{name}[{i}] += {}", self.expr(Kind::Int, 2)),
                }
            }
            kind => format!("{name} = {}", self.expr(kind, 2)),
        })
    }

    /// `{ … }` with `n` statements in a scope of their own.
    fn block(&mut self, indent: usize, depth: u32, n: usize, first: &[String]) -> String {
        let pad = "  ".repeat(indent + 1);
        self.ctx.scopes.push(Vec::new());
        let mut s = String::from("{\n");
        for line in first {
            s += &format!("{pad}{line}\n");
        }
        for _ in 0..n {
            s += &self.stmt(indent + 1, depth);
            s.push('\n');
        }
        self.ctx.scopes.pop();
        s += &"  ".repeat(indent);
        s.push('}');
        s
    }

    fn if_stmt(&mut self, indent: usize, depth: u32) -> String {
        let c = self.cond(2);
        let n = 1 + self.rng.below(2);
        let mut s = format!("if {c} {}", self.block(indent, depth - 1, n, &[]));
        match self.rng.below(5) {
            0 | 1 => {
                let n = 1 + self.rng.below(2);
                s += &format!(" else {}", self.block(indent, depth - 1, n, &[]));
            }
            2 => {
                let c = self.cond(1);
                s += &format!(" else if {c} {}", self.block(indent, depth - 1, 1, &[]));
            }
            _ => {}
        }
        s
    }

    /// Weights for `chance` that add up to at most 100%, and their sum in
    /// percent. Rarely they add up to more, which is an error.
    fn weights(&mut self) -> (Vec<String>, usize) {
        let mut ws = Vec::new();
        let mut total = 0;
        for _ in 0..1 + self.rng.below(3) {
            let w = *self.rng.pick(&[10, 20, 25, 30, 40, 50, 60, 75]);
            if total + w > 100 && !self.rng.chance(2) {
                break;
            }
            total += w;
            ws.push(format!("{w}%"));
        }
        if ws.is_empty() {
            ws.push("50%".into());
            total = 50;
        }
        (ws, total)
    }

    fn chance_stmt(&mut self, indent: usize, depth: u32) -> String {
        let pad = "  ".repeat(indent + 1);
        let (ws, total) = self.weights();
        let mut s = String::from("chance {\n");
        for w in ws {
            let body = self.arm_stmt(indent + 1, depth - 1);
            s += &format!("{pad}{w} => {body}\n");
        }
        if total < 100 && self.rng.chance(60) {
            let body = self.arm_stmt(indent + 1, depth - 1);
            s += &format!("{pad}else => {body}\n");
        }
        s += &"  ".repeat(indent);
        s.push('}');
        s
    }

    /// The body of a `chance` or `match` arm: one statement.
    fn arm_stmt(&mut self, indent: usize, depth: u32) -> String {
        self.ctx.scopes.push(Vec::new());
        let s = loop {
            let s = match self.rng.below(6) {
                0 | 1 => self.assign_stmt(),
                2 => {
                    let n = 1 + self.rng.below(2);
                    Some(self.block(indent, depth, n, &[]))
                }
                3 => self.observe_stmt(),
                4 => self.call_stmt(),
                _ => self.flow_stmt(),
            };
            if let Some(s) = s {
                break s;
            }
        };
        self.ctx.scopes.pop();
        s
    }

    fn match_stmt(&mut self, indent: usize, depth: u32) -> String {
        let pad = "  ".repeat(indent + 1);
        let subject = self.expr(Kind::Int, 1);
        let mut s = format!("match {subject} {{\n");
        for (pattern, bind) in self.patterns() {
            self.ctx.scopes.push(Vec::new());
            let guard = if pattern == "_" {
                String::new()
            } else {
                self.guard(bind.as_deref())
            };
            let body = self.arm_stmt(indent + 1, depth - 1);
            self.ctx.scopes.pop();
            s += &format!("{pad}{pattern}{guard} => {body}\n");
        }
        s += &"  ".repeat(indent);
        s.push('}');
        s
    }

    /// Patterns for a `match` on an int, the last one usually `_`; with
    /// the name each binds.
    fn patterns(&mut self) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        for _ in 0..1 + self.rng.below(3) {
            out.push(match self.rng.below(4) {
                0 | 1 => (self.small_int(), None),
                2 => (format!("{} | {}", self.small_int(), self.small_int()), None),
                _ => {
                    let n = self.fresh("n");
                    (n.clone(), Some(n))
                }
            });
        }
        if self.rng.chance(98) {
            out.push(("_".into(), None));
        }
        out
    }

    /// An optional guard; a name bound by the pattern is declared first.
    fn guard(&mut self, bind: Option<&str>) -> String {
        if let Some(n) = bind {
            self.declare(n, Kind::Int, false);
        }
        if !self.rng.chance(if bind.is_some() { 70 } else { 20 }) {
            return String::new();
        }
        let c = match (bind, self.rng.below(3)) {
            (Some(n), 0 | 1) => format!("{n} > {}", self.small_int()),
            _ => self.cond(1),
        };
        format!(" if {c}")
    }

    fn loop_stmt(&mut self, indent: usize, depth: u32) -> String {
        let pad = "  ".repeat(indent);
        let n = 1 + self.rng.below(2);
        self.ctx.loops += 1;
        let s = match self.rng.below(5) {
            0 => {
                let k = self.rng.below(4);
                format!("repeat {k} {}", self.block(indent, depth - 1, n, &[]))
            }
            1 | 2 => {
                let var = self.fresh("i");
                let coll = match self.rng.below(3) {
                    0 => {
                        let lists = self.list_vars();
                        match lists.first() {
                            Some(l) => l.clone(),
                            None => "[1, 2]".into(),
                        }
                    }
                    _ => format!("1..{}", self.rng.below(4)),
                };
                self.ctx.scopes.push(Vec::new());
                self.declare(&var, Kind::Int, false);
                let body = self.block(indent, depth - 1, n, &[]);
                self.ctx.scopes.pop();
                format!("for {var} in {coll} {body}")
            }
            3 => {
                // The counter can be read but not assigned, so the loop ends.
                let c = self.fresh("c");
                let k = 1 + self.rng.below(3);
                self.declare(&c, Kind::Int, false);
                let body = self.block(indent, depth - 1, n, &[format!("{c} += 1")]);
                format!("var {c} = 0\n{pad}while {c} < {k} {body}")
            }
            _ => {
                let c = self.fresh("c");
                let k = 1 + self.rng.below(3);
                self.declare(&c, Kind::Int, false);
                let first = [format!("if {c} >= {k} {{ break }}"), format!("{c} += 1")];
                let body = self.block(indent, depth - 1, n, &first);
                format!("var {c} = 0\n{pad}loop {body}")
            }
        };
        self.ctx.loops -= 1;
        s
    }

    fn list_vars(&self) -> Vec<String> {
        self.visible()
            .into_iter()
            .filter(|v| matches!(v.kind, Kind::List(_)))
            .map(|v| v.name)
            .collect()
    }

    fn observe_stmt(&mut self) -> Option<String> {
        if !self.ctx.observe {
            return None;
        }
        let ints = self.vars_of(Kind::Int);
        Some(match self.rng.below(12) {
            0 if !ints.is_empty() => {
                let v = self.rng.pick(&ints).clone();
                let from = match self.rng.below(3) {
                    0 => self.rng.pick(DICE).to_string(),
                    _ => format!("one_of([{v}, {}])", self.expr(Kind::Int, 1)),
                };
                format!("observe {v} from {from}")
            }
            // Evidence that rules the worlds that get here out.
            1 if self.rng.chance(30) => "observe false".into(),
            2 if self.rng.chance(30) => "observe 0%".into(),
            _ => format!("observe {}", self.evidence(2)),
        })
    }

    /// A condition for `observe`, usually one that can hold.
    fn evidence(&mut self, depth: u32) -> String {
        let d = depth.saturating_sub(1);
        match self.rng.below(20) {
            0..=7 => self.rng.pick(&["1%", "10%", "25%", "50%", "90%"]).to_string(),
            8..=13 => {
                let op = *self.rng.pick(&["<=", ">=", "!="]);
                format!("({} {op} {})", self.expr(Kind::DInt, d), self.rng.below(4))
            }
            14..=16 => format!("bernoulli({})", self.rng.pick(&["10%", "50%", "90%"])),
            17 => format!("(if {} {{ 90% }} else {{ 30% }})", self.cond(d)),
            _ => self.cond(depth),
        }
    }

    /// `break`, `continue` or `return`, under a condition.
    fn flow_stmt(&mut self) -> Option<String> {
        let mut options = Vec::new();
        if self.ctx.loops > 0 {
            options.extend(["break", "continue"]);
        }
        if self.ctx.ret.is_some() {
            options.push("return");
        }
        if options.is_empty() {
            return None;
        }
        let word = *self.rng.pick(&options);
        let body = match (word, self.ctx.ret) {
            ("return", Some(k)) => format!("return {}", self.expr(k, 1)),
            _ => word.to_string(),
        };
        Some(format!("if {} {{ {body} }}", self.cond(1)))
    }

    fn call_stmt(&mut self) -> Option<String> {
        if let Some(call) = self.recursive_call(None, 1) {
            return Some(call);
        }
        let f = self.callable(None)?;
        Some(self.call(&f, 1))
    }

    /// Sometimes, in a recursive function, a call to itself.
    fn recursive_call(&mut self, kind: Option<Kind>, d: u32) -> Option<String> {
        let (f, k) = self.ctx.recursive.clone()?;
        if !self.rng.chance(40) || !kind.is_none_or(|want| f.ret.fits(want)) {
            return None;
        }
        let mut args = vec![format!("({k} - 1)")];
        args.extend(f.params[1..].iter().map(|p| self.expr(*p, d)));
        Some(format!("{}({})", f.name, args.join(", ")))
    }

    /// Something to report: mostly the model's variables.
    fn reported(&mut self) -> String {
        let vars = self.visible();
        if !vars.is_empty() && self.rng.chance(70) {
            let v = self.rng.pick(&vars).clone();
            if self.rng.chance(50) {
                return v.name;
            }
            return self.expr(v.kind, 2);
        }
        let kind = self.some_kind();
        self.expr(kind, 2)
    }

    fn report_stmt(&mut self) -> String {
        self.reports += 1;
        let label = format!("r{}", self.reports);
        match self.rng.below(10) {
            0..=4 => format!("report {} as \"{label}\"", self.reported()),
            5 => {
                let c = self.cond(2);
                format!("if {c} {{\n  report {} as \"{label}\"\n}}", self.reported())
            }
            6 | 7 => {
                // Once per key: the key is the loop's variable.
                let var = self.fresh("i");
                let hi = 1 + self.rng.below(3);
                self.ctx.loops += 1;
                self.ctx.scopes.push(Vec::new());
                self.declare(&var, Kind::Int, false);
                let before = if self.rng.chance(50) {
                    format!("{}\n", self.stmt(1, 0))
                } else {
                    String::new()
                };
                let value = self.reported();
                self.ctx.scopes.pop();
                self.ctx.loops -= 1;
                // A key that isn't the loop variable counts every visit.
                let key = if self.rng.chance(25) {
                    format!("({var} mod 2)")
                } else {
                    var.clone()
                };
                format!("for {var} in 1..{hi} {{\n{before}  report {value} by {key} as \"{label}\"\n}}")
            }
            8 => {
                // The key isn't a loop variable: every visit counts.
                let k = 1 + self.rng.below(2);
                let value = self.reported();
                format!("repeat {k} {{\n  report {value} by 0 as \"{label}\"\n}}")
            }
            _ => self.stmt(0, 1),
        }
    }

    // ── Expressions ──────────────────────────────────────────────────────

    /// A condition: a fact, a probability or a distribution of facts.
    fn cond(&mut self, depth: u32) -> String {
        match self.rng.below(3) {
            0 => self.expr(Kind::Bool, depth),
            1 => self.expr(Kind::Prob, depth),
            _ => self.expr(Kind::DBool, depth),
        }
    }

    fn leaf(&mut self, kind: Kind) -> String {
        let vars = self.vars_of(kind);
        if !vars.is_empty() && self.rng.chance(60) {
            return self.rng.pick(&vars).clone();
        }
        match kind {
            Kind::Int => self.small_int(),
            Kind::Bool => self.rng.pick(&["true", "false"]).to_string(),
            Kind::Prob => self.prob(),
            Kind::DInt => {
                let lists = self.list_vars();
                if !lists.is_empty() && self.rng.chance(10) {
                    return format!("one_of({})", self.rng.pick(&lists));
                }
                self.rng.pick(DICE).to_string()
            }
            Kind::DBool => match self.rng.below(10) {
                0 => "bernoulli(0%)".into(),
                1 => "bernoulli(100%)".into(),
                _ => format!("bernoulli({})", self.prob()),
            },
            Kind::List(n) => {
                let items: Vec<String> = (0..n).map(|_| self.small_int()).collect();
                format!("[{}]", items.join(", "))
            }
        }
    }

    fn expr(&mut self, kind: Kind, depth: u32) -> String {
        if depth == 0 || self.rng.chance(30) {
            return self.leaf(kind);
        }
        let d = depth - 1;
        match kind {
            Kind::Int => match self.rng.below(14) {
                0 => format!("({} + {})", self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                1 => format!("({} - {})", self.expr(Kind::Int, d), self.expr(Kind::Int, d)),
                2 => format!("({} * {})", self.expr(Kind::Int, d), self.rng.below(4)),
                3 => format!("({} mod {})", self.expr(Kind::Int, d), 2 + self.rng.below(2)),
                4 => {
                    let f = *self.rng.pick(&["min", "max"]);
                    format!("{f}({}, {})", self.expr(Kind::Int, d), self.expr(Kind::Int, d))
                }
                5 => match self.rng.below(3) {
                    0 => format!("abs({})", self.expr(Kind::Int, d)),
                    1 => format!("({} div {})", self.expr(Kind::Int, d), 1 + self.rng.below(3)),
                    _ => format!("({} ^ {})", self.expr(Kind::Int, d), self.rng.below(3)),
                },
                6 => {
                    let n = 1 + self.rng.below(3);
                    let i = self.rng.below(n);
                    format!("{}[{i}]", self.expr(Kind::List(n), d))
                }
                7 => {
                    let n = 1 + self.rng.below(3);
                    format!("len({})", self.expr(Kind::List(n), d))
                }
                _ => self.compound(kind, d),
            },
            Kind::Bool => match self.rng.below(12) {
                0..=2 => {
                    let op = *self.rng.pick(COMPARE);
                    format!("({} {op} {})", self.expr(Kind::Int, d), self.expr(Kind::Int, d))
                }
                3 => format!("not {}", self.expr(Kind::Bool, d)),
                4 => format!("({} and {})", self.expr(Kind::Bool, d), self.expr(Kind::Bool, d)),
                5 => format!("({} or {})", self.expr(Kind::Bool, d), self.expr(Kind::Bool, d)),
                6 => {
                    let op = *self.rng.pick(&["in", "not in"]);
                    format!("({} {op} [1, 3, 5])", self.expr(Kind::Int, d))
                }
                _ => self.compound(kind, d),
            },
            Kind::Prob => match self.rng.below(4) {
                0 | 1 => format!("P({})", self.expr(Kind::DBool, d)),
                _ => self.compound(kind, d),
            },
            Kind::DInt => match self.rng.below(12) {
                0 => format!("({} + {})", self.expr(Kind::DInt, d), self.expr(Kind::DInt, d)),
                1 => format!("({} - {})", self.expr(Kind::DInt, d), self.expr(Kind::Int, d)),
                2 => format!("({} * {})", self.expr(Kind::DInt, d), self.rng.below(3)),
                3 => format!("max({}, {})", self.expr(Kind::DInt, d), self.expr(Kind::Int, d)),
                4 => format!("({} mod {})", self.expr(Kind::DInt, d), 2 + self.rng.below(2)),
                5 => {
                    // Occasionally a distribution among the choices: it's mixed in.
                    let n = 1 + self.rng.below(3);
                    let items: Vec<String> = (0..n)
                        .map(|_| {
                            let k = if self.rng.chance(15) { Kind::DInt } else { Kind::Int };
                            self.expr(k, d)
                        })
                        .collect();
                    format!("one_of([{}])", items.join(", "))
                }
                6 | 7 => self.simulate(Kind::DInt, d),
                8 => self.expr(Kind::Int, depth),
                _ => self.compound(kind, d),
            },
            Kind::DBool => match self.rng.below(12) {
                0 | 1 => {
                    let op = *self.rng.pick(COMPARE);
                    format!("({} {op} {})", self.expr(Kind::DInt, d), self.expr(Kind::Int, d))
                }
                2 => format!("bernoulli({})", self.expr(Kind::Prob, d)),
                3 => format!("not {}", self.expr(Kind::DBool, d)),
                4 => format!("({} and {})", self.expr(Kind::Bool, d), self.expr(Kind::DBool, d)),
                5 => format!("({} or {})", self.expr(Kind::DBool, d), self.expr(Kind::Bool, d)),
                // Two uncertain operands: an error, rarely on purpose.
                6 if self.rng.chance(10) => {
                    format!("({} and {})", self.expr(Kind::DBool, d), self.expr(Kind::DBool, d))
                }
                7 | 8 => self.simulate(Kind::DBool, d),
                9 => self.expr(Kind::Bool, depth),
                _ => self.compound(kind, d),
            },
            Kind::List(n) => match self.rng.below(4) {
                0 => {
                    let items: Vec<String> = (0..n).map(|_| self.expr(Kind::Int, d)).collect();
                    format!("[{}]", items.join(", "))
                }
                1 => self.if_value(kind, d),
                2 => self.block_value(kind, d),
                _ => self.leaf(kind),
            },
        }
    }

    /// Branching, blocks and calls, which work for every kind.
    fn compound(&mut self, kind: Kind, d: u32) -> String {
        match self.rng.below(6) {
            0 => self.if_value(kind, d),
            1 => self.chance_value(kind, d),
            2 => self.match_value(kind, d),
            3 | 4 => self.block_value(kind, d),
            _ => {
                if let Some(call) = self.recursive_call(Some(kind), d) {
                    return call;
                }
                match self.callable(Some(kind)) {
                    Some(f) => self.call(&f, d),
                    None => self.leaf(kind),
                }
            }
        }
    }

    fn if_value(&mut self, kind: Kind, d: u32) -> String {
        let c = self.cond(d);
        let a = self.expr(kind, d);
        if self.rng.chance(20) {
            let c2 = self.cond(d);
            let b = self.expr(kind, d);
            let e = self.expr(kind, d);
            return format!("(if {c} {{ {a} }} else if {c2} {{ {b} }} else {{ {e} }})");
        }
        let b = self.expr(kind, d);
        format!("(if {c} {{ {a} }} else {{ {b} }})")
    }

    fn chance_value(&mut self, kind: Kind, d: u32) -> String {
        let (ws, total) = self.weights();
        let mut arms: Vec<String> = ws
            .into_iter()
            .map(|w| format!("{w} => {}", self.expr(kind, d)))
            .collect();
        // Without `else`, weights that leave something over are an error;
        // that happens rarely, on purpose.
        if total != 100 && !self.rng.chance(1) || self.rng.chance(10) {
            arms.push(format!("else => {}", self.expr(kind, d)));
        }
        format!("(chance {{ {} }})", arms.join(", "))
    }

    fn match_value(&mut self, kind: Kind, d: u32) -> String {
        let subject = self.expr(Kind::Int, d);
        let mut arms = Vec::new();
        for (pattern, bind) in self.patterns() {
            self.ctx.scopes.push(Vec::new());
            let guard = if pattern == "_" {
                String::new()
            } else {
                self.guard(bind.as_deref())
            };
            let body = self.expr(kind, d);
            self.ctx.scopes.pop();
            arms.push(format!("{pattern}{guard} => {body}"));
        }
        format!("(match {subject} {{ {} }})", arms.join(", "))
    }

    /// `{ statements; value }`: the statements may draw, observe, and assign
    /// variables that other operands read.
    fn block_value(&mut self, kind: Kind, d: u32) -> String {
        self.ctx.scopes.push(Vec::new());
        let mut parts = Vec::new();
        for _ in 0..1 + self.rng.below(2) {
            parts.push(self.inline_stmt(d));
        }
        parts.push(self.expr(kind, d));
        self.ctx.scopes.pop();
        format!("({{ {} }})", parts.join("; "))
    }

    /// A statement that fits on one line.
    fn inline_stmt(&mut self, d: u32) -> String {
        loop {
            let s = match self.rng.below(6) {
                0 => {
                    let name = self.fresh("t");
                    let e = self.expr(Kind::DInt, d);
                    self.declare(&name, Kind::Int, false);
                    Some(format!("let {name} ~ {e}"))
                }
                4 if self.ctx.observe && self.rng.chance(50) => Some(format!("observe {}", self.evidence(d))),
                1 => {
                    let name = self.fresh("t");
                    let e = self.expr(Kind::Int, d);
                    self.declare(&name, Kind::Int, true);
                    Some(format!("var {name} = {e}"))
                }
                2 | 3 => self.assign_stmt(),
                4 => None,
                _ => {
                    let k = self.rng.below(3);
                    let targets: Vec<Var> = self.assignable().into_iter().filter(|v| v.kind == Kind::Int).collect();
                    if targets.is_empty() {
                        None
                    } else {
                        let v = self.rng.pick(&targets).name.clone();
                        Some(format!("repeat {k} {{ {v} += {} }}", self.expr(Kind::Int, 0)))
                    }
                }
            };
            if let Some(s) = s {
                return s;
            }
        }
    }

    /// `simulate { … }`: a separate model. Everything around it can be
    /// read but not assigned, and its observations stay inside.
    fn simulate(&mut self, kind: Kind, d: u32) -> String {
        let inner = Ctx {
            scopes: vec![Vec::new()],
            outer: self.visible(),
            ret: None,
            loops: 0,
            observe: true,
            callable: self.ctx.callable,
            recursive: self.ctx.recursive.clone(),
        };
        let saved = std::mem::replace(&mut self.ctx, inner);
        let mut parts = Vec::new();
        for _ in 0..1 + self.rng.below(2) {
            parts.push(self.inline_stmt(d));
        }
        if self.rng.chance(50) {
            parts.push(format!("observe {}", self.evidence(d)));
        }
        parts.push(self.expr(kind, d));
        self.ctx = saved;
        format!("(simulate {{ {} }})", parts.join("; "))
    }

    /// A function that can be called here and returns something that fits
    /// `kind` (any, without a kind).
    fn callable(&mut self, kind: Option<Kind>) -> Option<Func> {
        let candidates: Vec<Func> = self.funcs[..self.ctx.callable]
            .iter()
            .filter(|f| (self.ctx.observe || !f.observes) && kind.is_none_or(|k| f.ret.fits(k)))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return None;
        }
        Some(self.rng.pick(&candidates).clone())
    }

    fn call(&mut self, f: &Func, d: u32) -> String {
        let args: Vec<String> = f.params.iter().map(|k| self.expr(*k, d)).collect();
        format!("{}({})", f.name, args.join(", "))
    }
}
