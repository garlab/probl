//! Turns source text into tokens.
//!
//! Besides the usual work, the lexer decides which line breaks end a statement
//! (see [`filter_newlines`]), so the parser only ever sees significant ones.

use crate::diagnostic::Diagnostic;
use crate::span::Span;
use crate::token::{StrPart, Tok, Token, keyword};

/// Lex `src`, whose first byte sits at offset `base` of the enclosing file
/// (non-zero when lexing an interpolated expression inside a string).
pub fn lex(src: &str, base: u32) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer {
        src,
        bytes: src.as_bytes(),
        pos: 0,
        base,
        tokens: Vec::new(),
        diags: Vec::new(),
    };
    lexer.run();
    (filter_newlines(lexer.tokens), lexer.diags)
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    base: u32,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn span(&self, lo: usize, hi: usize) -> Span {
        Span::new(lo, hi).shifted(self.base)
    }

    fn push(&mut self, tok: Tok, lo: usize) {
        let span = self.span(lo, self.pos);
        self.tokens.push(Token { tok, span });
    }

    fn error(&mut self, lo: usize, hi: usize, message: impl Into<String>) -> &mut Diagnostic {
        let span = self.span(lo, hi);
        self.diags.push(Diagnostic::error(span, message));
        self.diags.last_mut().unwrap()
    }

    fn peek(&self, ahead: usize) -> u8 {
        self.bytes.get(self.pos + ahead).copied().unwrap_or(0)
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            match c {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' => {
                    let lo = self.pos;
                    self.pos += 1;
                    self.push(Tok::Newline, lo);
                }
                b'#' => {
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                b'0'..=b'9' => self.number(),
                b'"' => self.string(),
                c if c.is_ascii_alphabetic() || c == b'_' => self.word(),
                _ => self.punct(),
            }
        }
        let end = self.bytes.len();
        self.tokens.push(Token {
            tok: Tok::Eof,
            span: self.span(end, end),
        });
    }

    fn digits(&mut self) {
        while self.peek(0).is_ascii_digit() || (self.peek(0) == b'_' && self.peek(1).is_ascii_digit()) {
            self.pos += 1;
        }
    }

    fn number(&mut self) {
        let lo = self.pos;
        self.digits();
        let mut is_float = false;
        if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            self.pos += 1;
            self.digits();
            is_float = true;
        }
        if matches!(self.peek(0), b'e' | b'E')
            && (self.peek(1).is_ascii_digit() || (matches!(self.peek(1), b'+' | b'-') && self.peek(2).is_ascii_digit()))
        {
            self.pos += 2;
            self.digits();
            is_float = true;
        }
        let text: String = self.src[lo..self.pos].chars().filter(|&c| c != '_').collect();

        // Dice: `2d6`.
        if !is_float && self.peek(0) == b'd' && self.peek(1).is_ascii_digit() {
            self.pos += 1;
            let sides_lo = self.pos;
            self.digits();
            let sides_text: String = self.src[sides_lo..self.pos].chars().filter(|&c| c != '_').collect();
            if self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' {
                self.word_tail();
                self.error(lo, self.pos, "invalid dice literal")
                    .help("dice are written like `2d6`: a count, `d`, and a number of sides");
                return;
            }
            self.dice(lo, Some(&text), &sides_text);
            return;
        }

        // Percentages: `30%`, `12.5%`.
        if self.peek(0) == b'%' {
            self.pos += 1;
            match text.parse::<f64>() {
                Ok(v) => self.push(Tok::Percent(v / 100.0), lo),
                Err(_) => {
                    self.error(lo, self.pos, "invalid percentage");
                }
            }
            return;
        }

        if self.peek(0).is_ascii_alphabetic() || self.peek(0) == b'_' {
            self.word_tail();
            self.error(lo, self.pos, "invalid number")
                .help("names can't start with a digit");
            return;
        }

        if is_float {
            match text.parse::<f64>() {
                Ok(v) => self.push(Tok::Float(v), lo),
                Err(_) => {
                    self.error(lo, self.pos, "invalid number");
                }
            }
        } else {
            match text.parse::<i64>() {
                Ok(v) => self.push(Tok::Int(v), lo),
                Err(_) => {
                    self.error(lo, self.pos, "integer too large")
                        .note("integers must lie between -9223372036854775808 and 9223372036854775807");
                }
            }
        }
    }

    fn dice(&mut self, lo: usize, count: Option<&str>, sides: &str) {
        let count = match count {
            None => Some(1),
            Some(text) => text.parse::<u32>().ok(),
        };
        let sides = sides.parse::<u32>().ok();
        match (count, sides) {
            (Some(count), Some(sides)) if count >= 1 && sides >= 1 => {
                self.push(Tok::Dice { count, sides }, lo);
            }
            (Some(0), _) => {
                self.error(lo, self.pos, "a dice roll needs at least one die");
            }
            (_, Some(0)) => {
                self.error(lo, self.pos, "a die needs at least one side");
            }
            _ => {
                self.error(lo, self.pos, "dice literal too large");
            }
        }
    }

    fn word_tail(&mut self) {
        while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' {
            self.pos += 1;
        }
    }

    fn word(&mut self) {
        let lo = self.pos;
        self.word_tail();
        let text = &self.src[lo..self.pos];
        if text == "_" {
            self.push(Tok::Underscore, lo);
        } else if let Some(sides) = text
            .strip_prefix('d')
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        {
            let sides = sides.to_string();
            self.dice(lo, None, &sides);
        } else if let Some(tok) = keyword(text) {
            self.push(tok, lo);
        } else {
            let text = text.to_string();
            self.push(Tok::Ident(text), lo);
        }
    }

    fn string(&mut self) {
        let lo = self.pos;
        self.pos += 1; // opening quote
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            let Some(c) = self.src[self.pos..].chars().next() else {
                self.error(lo, self.pos, "unterminated string");
                return;
            };
            match c {
                '"' => {
                    self.pos += 1;
                    break;
                }
                '\n' => {
                    self.error(lo, self.pos, "unterminated string")
                        .help("strings must end on the line where they start");
                    return;
                }
                '\\' => {
                    let esc_lo = self.pos;
                    self.pos += 1;
                    let Some(e) = self.src[self.pos..].chars().next() else {
                        self.error(lo, self.pos, "unterminated string");
                        return;
                    };
                    self.pos += e.len_utf8();
                    match e {
                        'n' => text.push('\n'),
                        't' => text.push('\t'),
                        'r' => text.push('\r'),
                        '0' => text.push('\0'),
                        '\\' | '"' | '{' | '}' => text.push(e),
                        _ => {
                            self.error(esc_lo, self.pos, format!("unknown escape `\\{e}`"))
                                .help("the escapes are \\n \\t \\r \\0 \\\\ \\\" \\{ and \\}");
                        }
                    }
                }
                '{' => {
                    let open = self.pos;
                    self.pos += 1;
                    let start = self.pos;
                    let Some(end) = self.interpolation_end() else {
                        self.error(open, self.pos, "unclosed `{` in string")
                            .help("write `\\{` for a literal brace");
                        return;
                    };
                    if !text.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut text)));
                    }
                    let src = self.src[start..end].to_string();
                    if src.trim().is_empty() {
                        self.error(open, end + 1, "empty `{}` in string")
                            .help("put an expression inside, or write `\\{` for a literal brace");
                    }
                    parts.push(StrPart::Expr {
                        src,
                        offset: self.base + start as u32,
                    });
                    self.pos = end + 1;
                }
                '}' => {
                    let at = self.pos;
                    self.pos += 1;
                    self.error(at, self.pos, "unmatched `}` in string")
                        .help("write `\\}` for a literal brace");
                }
                c => {
                    text.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        if !text.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(text));
        }
        self.push(Tok::Str(parts), lo);
    }

    /// Find the `}` closing an interpolation that starts at `self.pos`.
    fn interpolation_end(&self) -> Option<usize> {
        let mut depth = 1;
        let mut i = self.pos;
        let mut in_string = false;
        while i < self.bytes.len() {
            let b = self.bytes[i];
            if b == b'\n' {
                return None;
            }
            if in_string {
                match b {
                    b'\\' => i += 1,
                    b'"' => in_string = false,
                    _ => {}
                }
            } else {
                match b {
                    b'"' => in_string = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i);
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        None
    }

    fn punct(&mut self) {
        let lo = self.pos;
        let (tok, len) = match (self.peek(0), self.peek(1), self.peek(2)) {
            (b'.', b'.', b'<') => (Tok::DotDotLt, 3),
            (b'.', b'.', _) => (Tok::DotDot, 2),
            (b'-', b'>', _) => (Tok::Arrow, 2),
            (b'=', b'>', _) => (Tok::FatArrow, 2),
            (b'=', b'=', _) => (Tok::EqEq, 2),
            (b'!', b'=', _) => (Tok::NotEq, 2),
            (b'<', b'=', _) => (Tok::Le, 2),
            (b'>', b'=', _) => (Tok::Ge, 2),
            (b'+', b'=', _) => (Tok::PlusAssign, 2),
            (b'-', b'=', _) => (Tok::MinusAssign, 2),
            (b'*', b'=', _) => (Tok::StarAssign, 2),
            (b'/', b'=', _) => (Tok::SlashAssign, 2),
            (b'(', _, _) => (Tok::LParen, 1),
            (b')', _, _) => (Tok::RParen, 1),
            (b'[', _, _) => (Tok::LBracket, 1),
            (b']', _, _) => (Tok::RBracket, 1),
            (b'{', _, _) => (Tok::LBrace, 1),
            (b'}', _, _) => (Tok::RBrace, 1),
            (b',', _, _) => (Tok::Comma, 1),
            (b'.', _, _) => (Tok::Dot, 1),
            (b':', _, _) => (Tok::Colon, 1),
            (b';', _, _) => (Tok::Semi, 1),
            (b'@', _, _) => (Tok::At, 1),
            (b'|', b'|', _) => return self.unexpected(2, "use `or` instead of `||`"),
            (b'|', _, _) => (Tok::Pipe, 1),
            (b'~', _, _) => (Tok::Tilde, 1),
            (b'=', _, _) => (Tok::Assign, 1),
            (b'<', _, _) => (Tok::Lt, 1),
            (b'>', _, _) => (Tok::Gt, 1),
            (b'+', _, _) => (Tok::Plus, 1),
            (b'-', _, _) => (Tok::Minus, 1),
            (b'*', _, _) => (Tok::Star, 1),
            (b'/', _, _) => (Tok::Slash, 1),
            (b'^', _, _) => (Tok::Caret, 1),
            (b'&', b'&', _) => return self.unexpected(2, "use `and` instead of `&&`"),
            (b'!', _, _) => return self.unexpected(1, "use `not` for negation"),
            (b'%', _, _) => {
                return self.unexpected(
                    1,
                    "`%` only marks percentages, as in `30%`; for the remainder of a division use `mod`",
                );
            }
            _ => {
                let c = self.src[self.pos..].chars().next().unwrap();
                self.pos += c.len_utf8();
                self.error(lo, self.pos, format!("unexpected character `{c}`"));
                return;
            }
        };
        self.pos += len;
        self.push(tok, lo);
    }

    fn unexpected(&mut self, len: usize, help: &str) {
        let lo = self.pos;
        self.pos += len;
        let text = self.src[lo..self.pos].to_string();
        self.error(lo, self.pos, format!("unexpected `{text}`")).help(help);
    }
}

/// Drop the line breaks that don't end a statement.
///
/// A line break is kept only where a statement could end. It is dropped:
/// - inside `( )` and `[ ]` (but not inside a `{ }` block nested in them);
/// - after a token that can't end an expression, such as a binary operator,
///   `,`, `=>` or an opening bracket;
/// - before a line starting with `.` (method chains);
/// - when it repeats a previous line break, or starts the file.
pub fn filter_newlines(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut stack: Vec<Tok> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        match token.tok {
            Tok::LParen | Tok::LBracket | Tok::LBrace => stack.push(token.tok.clone()),
            Tok::RParen | Tok::RBracket | Tok::RBrace => {
                stack.pop();
            }
            Tok::Newline => {
                let in_parens = matches!(stack.last(), Some(Tok::LParen | Tok::LBracket));
                let after_continuation = out.last().is_none_or(|prev| continues_line(&prev.tok));
                let next = tokens[i + 1..].iter().find(|t| t.tok != Tok::Newline);
                let before_continuation = next.is_some_and(|t| t.tok == Tok::Dot);
                if in_parens || after_continuation || before_continuation {
                    continue;
                }
            }
            _ => {}
        }
        out.push(token.clone());
    }
    out
}

/// Tokens after which a line break can't end the statement.
fn continues_line(tok: &Tok) -> bool {
    matches!(
        tok,
        Tok::Newline
            | Tok::Semi
            | Tok::Comma
            | Tok::Dot
            | Tok::Colon
            | Tok::LParen
            | Tok::LBracket
            | Tok::LBrace
            | Tok::Arrow
            | Tok::FatArrow
            | Tok::Tilde
            | Tok::Assign
            | Tok::PlusAssign
            | Tok::MinusAssign
            | Tok::StarAssign
            | Tok::SlashAssign
            | Tok::EqEq
            | Tok::NotEq
            | Tok::Lt
            | Tok::Le
            | Tok::Gt
            | Tok::Ge
            | Tok::Plus
            | Tok::Minus
            | Tok::Star
            | Tok::Slash
            | Tok::Caret
            | Tok::DotDot
            | Tok::DotDotLt
            | Tok::And
            | Tok::Or
            | Tok::Not
            | Tok::In
            | Tok::Div
            | Tok::Mod
            | Tok::Pipe
            | Tok::At
            | Tok::With
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let (tokens, diags) = lex(src, 0);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        tokens.into_iter().map(|t| t.tok).collect()
    }

    fn errors(src: &str) -> Vec<String> {
        lex(src, 0).1.into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn numbers_percentages_and_dice() {
        assert_eq!(
            toks("42 1_000 3.5 2.5e-3 1e-12 30% 12.5% d6 2d6 10d10"),
            vec![
                Tok::Int(42),
                Tok::Int(1000),
                Tok::Float(3.5),
                Tok::Float(2.5e-3),
                Tok::Float(1e-12),
                Tok::Percent(0.3),
                Tok::Percent(0.125),
                Tok::Dice { count: 1, sides: 6 },
                Tok::Dice { count: 2, sides: 6 },
                Tok::Dice { count: 10, sides: 10 },
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn ranges_are_not_floats() {
        assert_eq!(
            toks("1..6 0..<n"),
            vec![
                Tok::Int(1),
                Tok::DotDot,
                Tok::Int(6),
                Tok::Int(0),
                Tok::DotDotLt,
                Tok::Ident("n".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn names_that_look_like_dice() {
        assert_eq!(
            toks("d6x dx d _d6"),
            vec![
                Tok::Ident("d6x".into()),
                Tok::Ident("dx".into()),
                Tok::Ident("d".into()),
                Tok::Ident("_d6".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn keywords_and_contextual_words() {
        assert_eq!(
            toks("let x ~ 5 to 10"),
            vec![
                Tok::Let,
                Tok::Ident("x".into()),
                Tok::Tilde,
                Tok::Int(5),
                Tok::Ident("to".into()),
                Tok::Int(10),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn strings_with_interpolation() {
        assert_eq!(
            toks(r#""hp: {h + 1}!" "a\{b\}" """#),
            vec![
                Tok::Str(vec![
                    StrPart::Lit("hp: ".into()),
                    StrPart::Expr {
                        src: "h + 1".into(),
                        offset: 6
                    },
                    StrPart::Lit("!".into()),
                ]),
                Tok::Str(vec![StrPart::Lit("a{b}".into())]),
                Tok::Str(vec![StrPart::Lit(String::new())]),
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            toks("a # comment\nb"),
            vec![Tok::Ident("a".into()), Tok::Newline, Tok::Ident("b".into()), Tok::Eof]
        );
    }

    #[test]
    fn line_breaks_inside_brackets_and_after_operators_are_dropped() {
        assert_eq!(
            toks("f(1,\n2)\nx = a +\n b\n\n[1\n]"),
            vec![
                Tok::Ident("f".into()),
                Tok::LParen,
                Tok::Int(1),
                Tok::Comma,
                Tok::Int(2),
                Tok::RParen,
                Tok::Newline,
                Tok::Ident("x".into()),
                Tok::Assign,
                Tok::Ident("a".into()),
                Tok::Plus,
                Tok::Ident("b".into()),
                Tok::Newline,
                Tok::LBracket,
                Tok::Int(1),
                Tok::RBracket,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn line_breaks_inside_blocks_nested_in_parens_are_kept() {
        assert_eq!(
            toks("f({\na\nb\n})"),
            vec![
                Tok::Ident("f".into()),
                Tok::LParen,
                Tok::LBrace,
                Tok::Ident("a".into()),
                Tok::Newline,
                Tok::Ident("b".into()),
                Tok::Newline,
                Tok::RBrace,
                Tok::RParen,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn line_breaks_before_method_chains_are_dropped() {
        assert_eq!(
            toks("xs\n.len()"),
            vec![
                Tok::Ident("xs".into()),
                Tok::Dot,
                Tok::Ident("len".into()),
                Tok::LParen,
                Tok::RParen,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn helpful_errors() {
        assert_eq!(errors("a % b"), vec!["unexpected `%`"]);
        assert_eq!(errors("a && b"), vec!["unexpected `&&`"]);
        assert_eq!(errors("\"abc"), vec!["unterminated string"]);
        assert_eq!(errors("2d6x"), vec!["invalid dice literal"]);
        assert_eq!(errors("0d6"), vec!["a dice roll needs at least one die"]);
        assert_eq!(errors("9999999999999999999"), vec!["integer too large"]);
    }
}
