//! Tokens produced by the lexer.

use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(probl_number::Integer),
    Float(f64),
    /// A percentage, already divided by 100: `30%` is `Percent(0.3)`.
    Percent(f64),
    /// `2d6` is `Dice { count: 2, sides: 6 }`; `d6` has a count of 1.
    Dice {
        count: u32,
        sides: u32,
    },
    Str(Vec<StrPart>),
    /// Identifiers, including the contextual keywords `as`, `by`, `from` and `to`.
    Ident(String),

    // Keywords
    And,
    Break,
    Catch,
    Chance,
    Continue,
    Div,
    Else,
    Enum,
    False,
    Fn,
    For,
    If,
    Import,
    In,
    Let,
    Loop,
    Match,
    Mod,
    Not,
    Observe,
    Score,
    Or,
    Repeat,
    Report,
    Return,
    Simulate,
    True,
    Try,
    Type,
    Typeof,
    Var,
    While,
    With,

    // Punctuation
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Dot,
    Colon,
    Semi,
    At,
    Pipe,
    Underscore,
    /// `->`
    Arrow,
    /// `=>`
    FatArrow,
    Tilde,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    DotDot,
    /// `..<`
    DotDotLt,

    Newline,
    Eof,
}

/// A piece of a string literal: plain text, or the source of an interpolated `{expression}`.
#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Lit(String),
    Expr { src: String, offset: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Whether `text` can be a name in a program: ASCII letters, digits and
/// `_`, not starting with a digit, and neither a keyword nor a die like `d6`.
pub fn is_name(text: &str) -> bool {
    let is_die = text
        .strip_prefix('d')
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    text.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && text != "_"
        && !is_die
        && keyword(text).is_none()
}

pub fn keyword(word: &str) -> Option<Tok> {
    Some(match word {
        "and" => Tok::And,
        "break" => Tok::Break,
        "catch" => Tok::Catch,
        "chance" => Tok::Chance,
        "continue" => Tok::Continue,
        "div" => Tok::Div,
        "else" => Tok::Else,
        "enum" => Tok::Enum,
        "false" => Tok::False,
        "fn" => Tok::Fn,
        "for" => Tok::For,
        "if" => Tok::If,
        "import" => Tok::Import,
        "in" => Tok::In,
        "let" => Tok::Let,
        "loop" => Tok::Loop,
        "match" => Tok::Match,
        "mod" => Tok::Mod,
        "not" => Tok::Not,
        "observe" => Tok::Observe,
        "score" => Tok::Score,
        "or" => Tok::Or,
        "repeat" => Tok::Repeat,
        "report" => Tok::Report,
        "return" => Tok::Return,
        "simulate" => Tok::Simulate,
        "true" => Tok::True,
        "try" => Tok::Try,
        "type" => Tok::Type,
        "typeof" => Tok::Typeof,
        "var" => Tok::Var,
        "while" => Tok::While,
        "with" => Tok::With,
        _ => return None,
    })
}

impl Tok {
    /// How the token is described in error messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Int(v) => format!("number `{v}`"),
            Tok::Float(v) => format!("number `{v}`"),
            Tok::Percent(v) => format!("percentage `{}%`", v * 100.0),
            Tok::Dice { count, sides } => format!("dice `{count}d{sides}`"),
            Tok::Str(_) => "a string".to_string(),
            Tok::Ident(name) => format!("`{name}`"),
            Tok::Newline => "the end of the line".to_string(),
            Tok::Eof => "the end of the file".to_string(),
            other => format!("`{}`", other.text()),
        }
    }

    /// Source text of keywords and punctuation.
    pub fn text(&self) -> &'static str {
        match self {
            Tok::And => "and",
            Tok::Break => "break",
            Tok::Catch => "catch",
            Tok::Chance => "chance",
            Tok::Continue => "continue",
            Tok::Div => "div",
            Tok::Else => "else",
            Tok::Enum => "enum",
            Tok::False => "false",
            Tok::Fn => "fn",
            Tok::For => "for",
            Tok::If => "if",
            Tok::Import => "import",
            Tok::In => "in",
            Tok::Let => "let",
            Tok::Loop => "loop",
            Tok::Match => "match",
            Tok::Mod => "mod",
            Tok::Not => "not",
            Tok::Observe => "observe",
            Tok::Score => "score",
            Tok::Or => "or",
            Tok::Repeat => "repeat",
            Tok::Report => "report",
            Tok::Return => "return",
            Tok::Simulate => "simulate",
            Tok::True => "true",
            Tok::Try => "try",
            Tok::Type => "type",
            Tok::Typeof => "typeof",
            Tok::Var => "var",
            Tok::While => "while",
            Tok::With => "with",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::Comma => ",",
            Tok::Dot => ".",
            Tok::Colon => ":",
            Tok::Semi => ";",
            Tok::At => "@",
            Tok::Pipe => "|",
            Tok::Underscore => "_",
            Tok::Arrow => "->",
            Tok::FatArrow => "=>",
            Tok::Tilde => "~",
            Tok::Assign => "=",
            Tok::PlusAssign => "+=",
            Tok::MinusAssign => "-=",
            Tok::StarAssign => "*=",
            Tok::SlashAssign => "/=",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Caret => "^",
            Tok::DotDot => "..",
            Tok::DotDotLt => "..<",
            Tok::Int(_)
            | Tok::Float(_)
            | Tok::Percent(_)
            | Tok::Dice { .. }
            | Tok::Str(_)
            | Tok::Ident(_)
            | Tok::Newline
            | Tok::Eof => "",
        }
    }

    pub fn is_ident(&self, word: &str) -> bool {
        matches!(self, Tok::Ident(name) if name == word)
    }
}
