//! Where a program's names are declared and used, recorded as the program is
//! lowered, so they're resolved exactly as the compiler resolves them. It's
//! what an editor needs to go to a name's definition, describe it, and
//! complete names (docs/playground-plan.md).

use probl_syntax::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefKind {
    Variable,
    Parameter,
    Function,
    Record,
    Field,
    Enum,
    Variant,
}

impl DefKind {
    pub fn name(self) -> &'static str {
        match self {
            DefKind::Variable => "variable",
            DefKind::Parameter => "parameter",
            DefKind::Function => "function",
            DefKind::Record => "record",
            DefKind::Field => "field",
            DefKind::Enum => "enum",
            DefKind::Variant => "variant",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub name: String,
    pub kind: DefKind,
    /// Declared with `var`, so it can be assigned.
    pub mutable: bool,
    /// The name where it's declared.
    pub span: Span,
    /// Where it can be used by name: from its declaration to the end of its
    /// block, or all of the program for functions, types and variants.
    pub scope: Span,
    /// A top-level variable, which functions can also use, wherever they're
    /// written.
    pub global: bool,
    /// For a field or a variant: the record or enum it belongs to.
    pub owner: Option<String>,
    /// For a field: its type, as written.
    pub ty: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Symbols {
    pub definitions: Vec<Definition>,
    /// Every use of a name that was resolved: where it is, and the index of
    /// its definition.
    pub references: Vec<(Span, usize)>,
    /// The named functions, from `fn` to their closing brace: top-level
    /// variables can be used anywhere in them.
    pub functions: Vec<Span>,
}

impl Symbols {
    /// The definition of the name at `offset`, whether it's used or declared
    /// there.
    pub fn at(&self, offset: u32) -> Option<usize> {
        let inside = |s: Span| s.lo <= offset && offset <= s.hi;
        self.references
            .iter()
            .find(|(span, _)| inside(*span))
            .map(|&(_, d)| d)
            .or_else(|| self.definitions.iter().position(|d| inside(d.span)))
    }

    /// The definitions that can be used by name at `offset`, nearest first.
    pub fn visible(&self, offset: u32) -> Vec<usize> {
        let in_function = self.functions.iter().any(|f| f.lo <= offset && offset <= f.hi);
        let mut found: Vec<usize> = (0..self.definitions.len())
            .filter(|&i| {
                let d = &self.definitions[i];
                !matches!(d.kind, DefKind::Field)
                    && ((d.scope.lo <= offset && offset <= d.scope.hi) || (d.global && in_function))
            })
            .collect();
        // Inner scopes first, and later declarations before earlier ones.
        found.sort_by_key(|&i| std::cmp::Reverse(self.definitions[i].scope.lo));
        found
    }
}
