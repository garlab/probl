//! The built-in functions: names, arities and how they treat distributions.

/// How a built-in treats arguments that are distributions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifting {
    /// Applied to every outcome: `abs(d6 - 4)` is a distribution.
    Lift,
    /// Receives distributions as they are (`mean`, `P`, `print`, …).
    Raw,
}

macro_rules! builtins {
    ($( $(#[$attr:meta])* $variant:ident = $name:literal, $min:literal ..= $max:expr, $lifting:ident, $public:literal; )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Builtin { $($(#[$attr])* $variant),* }

        impl Builtin {
            pub const ALL: &'static [Builtin] = &[$(Builtin::$variant),*];

            pub fn name(self) -> &'static str {
                match self { $(Builtin::$variant => $name),* }
            }

            /// Minimum and maximum number of arguments (`usize::MAX` for any number).
            pub fn arity(self) -> (usize, usize) {
                match self { $(Builtin::$variant => ($min, $max)),* }
            }

            pub fn lifting(self) -> Lifting {
                match self { $(Builtin::$variant => Lifting::$lifting),* }
            }

            /// Whether programs can call it by name (internal helpers can't).
            pub fn is_public(self) -> bool {
                match self { $(Builtin::$variant => $public),* }
            }
        }
    };
}

const ANY: usize = usize::MAX;

builtins! {
    // Math
    Min = "min", 1..=ANY, Lift, true;
    Max = "max", 1..=ANY, Lift, true;
    Abs = "abs", 1..=1, Lift, true;
    Floor = "floor", 1..=1, Lift, true;
    Ceil = "ceil", 1..=1, Lift, true;
    Trunc = "trunc", 1..=1, Lift, true;
    Round = "round", 1..=2, Lift, true;
    Sqrt = "sqrt", 1..=1, Lift, true;
    Cbrt = "cbrt", 1..=1, Lift, true;
    Exp = "exp", 1..=1, Lift, true;
    Exp2 = "exp2", 1..=1, Lift, true;
    Ln = "ln", 1..=1, Lift, true;
    Log10 = "log10", 1..=1, Lift, true;
    Log2 = "log2", 1..=1, Lift, true;
    Log1p = "log1p", 1..=1, Lift, true;
    Expm1 = "expm1", 1..=1, Lift, true;
    Sin = "sin", 1..=1, Lift, true;
    Cos = "cos", 1..=1, Lift, true;
    Tan = "tan", 1..=1, Lift, true;
    Asin = "asin", 1..=1, Lift, true;
    Acos = "acos", 1..=1, Lift, true;
    Atan = "atan", 1..=1, Lift, true;
    Atan2 = "atan2", 2..=2, Lift, true;
    Hypot = "hypot", 2..=2, Lift, true;
    Sinh = "sinh", 1..=1, Lift, true;
    Cosh = "cosh", 1..=1, Lift, true;
    Tanh = "tanh", 1..=1, Lift, true;
    Asinh = "asinh", 1..=1, Lift, true;
    Acosh = "acosh", 1..=1, Lift, true;
    Atanh = "atanh", 1..=1, Lift, true;
    BitLength = "bit_length", 1..=1, Lift, true;
    BitAnd = "bit_and", 2..=2, Lift, true;
    BitOr = "bit_or", 2..=2, Lift, true;
    BitXor = "bit_xor", 2..=2, Lift, true;
    BitNot = "bit_not", 1..=1, Lift, true;
    BitCount = "bit_count", 1..=1, Lift, true;
    ILog2 = "ilog2", 1..=1, Lift, true;
    Choose = "choose", 2..=2, Lift, true;
    Factorial = "factorial", 1..=1, Lift, true;
    Gcd = "gcd", 2..=2, Lift, true;
    Lcm = "lcm", 2..=2, Lift, true;
    EulerPhi = "euler_phi", 1..=1, Lift, true;
    LnGamma = "ln_gamma", 1..=1, Lift, true;
    Erf = "erf", 1..=1, Lift, true;
    Erfc = "erfc", 1..=1, Lift, true;
    Complex = "complex", 1..=2, Lift, true;
    Real = "real", 1..=1, Lift, true;
    Imag = "imag", 1..=1, Lift, true;
    Conj = "conj", 1..=1, Lift, true;
    Abs2 = "abs2", 1..=1, Lift, true;
    Arg = "arg", 1..=1, Lift, true;
    Cis = "cis", 1..=1, Lift, true;
    Clamp = "clamp", 3..=3, Lift, true;
    // Text
    Str = "str", 1..=1, Lift, true;
    Upper = "upper", 1..=1, Lift, true;
    Lower = "lower", 1..=1, Lift, true;
    Trim = "trim", 1..=2, Lift, true;
    TrimStart = "trim_start", 1..=2, Lift, true;
    TrimEnd = "trim_end", 1..=2, Lift, true;
    StartsWith = "starts_with", 2..=2, Lift, true;
    EndsWith = "ends_with", 2..=2, Lift, true;
    Chars = "chars", 1..=1, Lift, true;
    Split = "split", 2..=2, Lift, true;
    Join = "join", 2..=2, Lift, true;
    Print = "print", 0..=ANY, Raw, true;
    // Collections
    Len = "len", 1..=1, Lift, true;
    Slice = "slice", 2..=3, Lift, true;
    Sum = "sum", 1..=1, Lift, true;
    Count = "count", 1..=2, Lift, true;
    Map = "map", 2..=2, Lift, true;
    Filter = "filter", 2..=2, Lift, true;
    Reduce = "reduce", 3..=3, Lift, true;
    Sort = "sort", 1..=1, Lift, true;
    SortDesc = "sort_desc", 1..=1, Lift, true;
    Reverse = "reverse", 1..=1, Lift, true;
    Keys = "keys", 1..=1, Lift, true;
    Values = "values", 1..=1, Lift, true;
    Get = "get", 2..=3, Lift, true;
    Contains = "contains", 2..=2, Lift, true;
    Highest = "highest", 1..=2, Lift, true;
    Lowest = "lowest", 1..=2, Lift, true;
    Enumerate = "enumerate", 1..=1, Lift, true;
    Zip = "zip", 2..=2, Lift, true;
    // Mutating methods; lowering turns `xs.push(v)` into `xs = push(xs, v)`.
    Push = "push", 2..=2, Lift, true;
    Insert = "insert", 3..=3, Lift, true;
    Remove = "remove", 2..=2, Lift, true;
    Pop = "pop", 1..=1, Lift, true;
    Take = "take", 1..=1, Raw, true;
    // Distributions
    Bernoulli = "bernoulli", 1..=1, Lift, true;
    OneOf = "one_of", 1..=1, Lift, true;
    Binomial = "binomial", 2..=2, Lift, true;
    Poisson = "poisson", 1..=1, Lift, true;
    Geometric = "geometric", 1..=1, Lift, true;
    Roll = "roll", 2..=2, Raw, true;
    Bag = "bag", 1..=1, Lift, true;
    Normal = "normal", 2..=2, Lift, true;
    Lognormal = "lognormal", 2..=2, Lift, true;
    Uniform = "uniform", 2..=2, Lift, true;
    Beta = "beta", 2..=2, Lift, true;
    Gamma = "gamma", 2..=2, Lift, true;
    Exponential = "exponential", 1..=1, Lift, true;
    Triangular = "triangular", 3..=3, Lift, true;
    Pert = "pert", 3..=3, Lift, true;
    NormalRange = "normal_range", 2..=2, Lift, true;
    Mixture = "mixture", 1..=1, Raw, true;
    Truncate = "truncate", 3..=3, Raw, true;
    Bins = "bins", 2..=2, Raw, true;
    /// `a to b`.
    To = "to", 2..=2, Lift, false;
    // Questions about distributions
    P = "P", 1..=1, Raw, true;
    Mean = "mean", 1..=1, Raw, true;
    Sd = "sd", 1..=1, Raw, true;
    Variance = "variance", 1..=1, Raw, true;
    Median = "median", 1..=1, Raw, true;
    Quantile = "quantile", 2..=2, Raw, true;
    Support = "support", 1..=1, Raw, true;
    Cdf = "cdf", 2..=2, Raw, true;
    Pmf = "pmf", 2..=2, Raw, true;
    Pdf = "pdf", 2..=2, Raw, true;
    // Probability helpers
    Odds = "odds", 1..=1, Lift, true;
    Logit = "logit", 1..=1, Lift, true;
    InvLogit = "inv_logit", 1..=1, Lift, true;
    // Dates
    Date = "date", 1..=3, Lift, true;
    RunDate = "$run_date", 0..=0, Raw, false;
    Days = "days", 1..=1, Lift, true;
    Weeks = "weeks", 1..=1, Lift, true;
    AddWorkdays = "add_workdays", 2..=3, Lift, true;
    IsWorkday = "is_workday", 1..=2, Lift, true;
    AddMonths = "add_months", 2..=2, Lift, true;
    AddYears = "add_years", 2..=2, Lift, true;
    StartOfMonth = "start_of_month", 1..=1, Lift, true;
    EndOfMonth = "end_of_month", 1..=1, Lift, true;
    Year = "year", 1..=1, Lift, true;
    Month = "month", 1..=1, Lift, true;
    Day = "day", 1..=1, Lift, true;
    Weekday = "weekday", 1..=1, Lift, true;
    // Internal helpers used by the lowering pass
    /// The prefix `typeof` operator, which inspects its operand without lifting.
    Typeof = "$typeof", 1..=1, Raw, false;
    /// Checks a `for` loop's collection and turns it into something indexable.
    IterItems = "$iter_items", 1..=1, Raw, false;
    /// Checks a `repeat` count.
    RepeatCount = "$repeat_count", 1..=1, Raw, false;
    /// 100% if the value is certainly false, else 0% (for `and` with hoisted parts).
    IsFalse = "$is_false", 1..=1, Raw, false;
    /// 100% if the value is certainly true, else 0% (for `or` with hoisted parts).
    IsTrue = "$is_true", 1..=1, Raw, false;
    /// 100% if the value is a list of the given length (list patterns).
    IsListOfLen = "$is_list_of_len", 2..=2, Raw, false;
    /// Checks that a `match` subject is a settled value, not a distribution.
    Settled = "$settled", 1..=1, Raw, false;
    /// The last element of a list (for `pop`).
    Last = "$last", 1..=1, Raw, false;
    /// A list without its last element (for `pop`).
    DropLast = "$drop_last", 1..=1, Raw, false;
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Builtin> {
        Builtin::ALL.iter().copied().find(|b| b.is_public() && b.name() == name)
    }

    /// Methods that change the collection they're called on.
    pub fn is_mutating(self) -> bool {
        matches!(self, Builtin::Push | Builtin::Insert | Builtin::Remove | Builtin::Pop)
    }
}

/// A named value. A program's own variables, variants and functions hide
/// it, so `let e = 5` still works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constant {
    Pi,
    E,
    EulerGamma,
    Today,
}

impl Constant {
    pub const ALL: &'static [Constant] = &[Constant::Pi, Constant::E, Constant::EulerGamma, Constant::Today];

    pub fn name(self) -> &'static str {
        match self {
            Constant::Pi => "pi",
            Constant::E => "e",
            Constant::EulerGamma => "euler_gamma",
            Constant::Today => "today",
        }
    }

    /// Static numeric values; `today` is supplied by the execution context.
    pub fn value(self) -> Option<f64> {
        Some(match self {
            Constant::Pi => std::f64::consts::PI,
            Constant::E => std::f64::consts::E,
            // The Euler–Mascheroni constant: `std::f64::consts::EGAMMA` isn't stable yet.
            Constant::EulerGamma => 0.577_215_664_901_532_9,
            Constant::Today => return None,
        })
    }

    pub fn from_name(name: &str) -> Option<Constant> {
        Constant::ALL.iter().copied().find(|c| c.name() == name)
    }
}
