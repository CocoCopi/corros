//! The Corros lexer: turns source text into a stream of [`Token`]s.
//!
//! Handles numbers (integers, floats, exponents), strings with escapes,
//! comments (`//` and `/* */`), identifiers, keywords, and all operators.

use crate::error::{CompileError, CompileResult};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Number(f64),
    Str(String),
    Identifier(String),
    // Keywords (Corros's own vocabulary)
    Forge,
    Craft,
    When,
    Else,
    Whilst,
    Each,
    In,
    Return,
    Break,
    Onward,
    True,
    False,
    Nil,
    Adopt,
    // Punctuation & operators
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Semicolon,
    Colon,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    BangEqual,
    Equal,
    EqualEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    And,
    Or,
    PlusEqual,
    MinusEqual,
    StarEqual,
    SlashEqual,
    PercentEqual,
    Power,
    PowerEqual,
    DotDot,
    DotDotEqual,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub line: u32,
    pub column: u32,
    pub file: String,
}

impl Token {
    pub fn eof(file: &str, line: u32, column: u32) -> Token {
        Token {
            kind: TokenKind::Eof,
            lexeme: String::new(),
            line,
            column,
            file: file.to_string(),
        }
    }
}

pub fn lex(source: &str, file: &str) -> CompileResult<Vec<Token>> {
    Lexer::new(source, file).scan()
}

struct Lexer<'a> {
    chars: Vec<char>,
    file: &'a str,
    pos: usize,
    /// Character index where the current token started.
    token_start: usize,
    line: u32,
    line_start: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str, file: &'a str) -> Self {
        Lexer {
            chars: source.chars().collect(),
            file,
            pos: 0,
            token_start: 0,
            line: 1,
            line_start: 0,
        }
    }

    fn scan(mut self) -> CompileResult<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments()?;
            if self.at_end() {
                tokens.push(Token::eof(self.file, self.line, self.column()));
                break;
            }
            self.token_start = self.pos;
            let kind = self.scan_token()?;
            let lexeme: String = self.chars[self.token_start..self.pos].iter().collect();
            tokens.push(Token {
                kind,
                lexeme,
                line: self.line,
                column: self.column_at(self.token_start),
                file: self.file.to_string(),
            });
        }
        Ok(tokens)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if let Some(c) = c {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.line_start = self.pos;
            }
        }
        c
    }

    fn column(&self) -> u32 {
        self.column_at(self.pos)
    }

    fn column_at(&self, pos: usize) -> u32 {
        (pos - self.line_start + 1) as u32
    }

    fn err(&self, message: impl Into<String>) -> CompileError {
        CompileError::new(message, self.file, self.line, self.column())
    }

    fn skip_whitespace_and_comments(&mut self) -> CompileResult<()> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.advance();
                }
                Some('/') if self.peek_next() == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                Some('/') if self.peek_next() == Some('*') => {
                    self.advance();
                    self.advance();
                    let mut closed = false;
                    while !self.at_end() {
                        if self.peek() == Some('*') && self.peek_next() == Some('/') {
                            self.advance();
                            self.advance();
                            closed = true;
                            break;
                        }
                        self.advance();
                    }
                    if !closed {
                        return Err(self.err("unterminated block comment"));
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn scan_token(&mut self) -> CompileResult<TokenKind> {
        let c = self.peek().expect("scan_token called at end");
        match c {
            c if c.is_ascii_digit() => self.number(),
            c if c.is_alphabetic() || c == '_' || c == '$' => self.identifier(),
            '"' => self.string(),
            '(' => {
                self.advance();
                Ok(TokenKind::LParen)
            }
            ')' => {
                self.advance();
                Ok(TokenKind::RParen)
            }
            '{' => {
                self.advance();
                Ok(TokenKind::LBrace)
            }
            '}' => {
                self.advance();
                Ok(TokenKind::RBrace)
            }
            '[' => {
                self.advance();
                Ok(TokenKind::LBracket)
            }
            ']' => {
                self.advance();
                Ok(TokenKind::RBracket)
            }
            ',' => {
                self.advance();
                Ok(TokenKind::Comma)
            }
            ';' => {
                self.advance();
                Ok(TokenKind::Semicolon)
            }
            ':' => {
                self.advance();
                Ok(TokenKind::Colon)
            }
            '+' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::PlusEqual
                    }
                    _ => TokenKind::Plus,
                })
            }
            '-' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::MinusEqual
                    }
                    _ => TokenKind::Minus,
                })
            }
            '*' => {
                self.advance();
                Ok(match self.peek() {
                    Some('*') => {
                        self.advance();
                        match self.peek() {
                            Some('=') => {
                                self.advance();
                                TokenKind::PowerEqual
                            }
                            _ => TokenKind::Power,
                        }
                    }
                    Some('=') => {
                        self.advance();
                        TokenKind::StarEqual
                    }
                    _ => TokenKind::Star,
                })
            }
            '/' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::SlashEqual
                    }
                    _ => TokenKind::Slash,
                })
            }
            '%' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::PercentEqual
                    }
                    _ => TokenKind::Percent,
                })
            }
            '!' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::BangEqual
                    }
                    _ => TokenKind::Bang,
                })
            }
            '=' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::EqualEqual
                    }
                    _ => TokenKind::Equal,
                })
            }
            '>' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::GreaterEqual
                    }
                    _ => TokenKind::Greater,
                })
            }
            '<' => {
                self.advance();
                Ok(match self.peek() {
                    Some('=') => {
                        self.advance();
                        TokenKind::LessEqual
                    }
                    _ => TokenKind::Less,
                })
            }
            '&' => {
                self.advance();
                if self.peek() == Some('&') {
                    self.advance();
                    Ok(TokenKind::And)
                } else {
                    Err(self.err("expected '&' (use '&&' for logical and)"))
                }
            }
            '|' => {
                self.advance();
                if self.peek() == Some('|') {
                    self.advance();
                    Ok(TokenKind::Or)
                } else {
                    Err(self.err("expected '|' (use '||' for logical or)"))
                }
            }
            '.' => {
                self.advance();
                if self.peek() == Some('.') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Ok(TokenKind::DotDotEqual)
                    } else {
                        Ok(TokenKind::DotDot)
                    }
                } else {
                    Ok(TokenKind::Dot)
                }
            }
            _ => Err(self.err(format!("unexpected character '{}'", c))),
        }
    }

    fn number(&mut self) -> CompileResult<TokenKind> {
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.advance();
        }
        // Fraction part: only consume the '.' if a digit follows, so that
        // `1..5` lexes as `1`, `..`, `5` rather than `1.` `.5`.
        if self.peek() == Some('.') && matches!(self.peek_next(), Some(c) if c.is_ascii_digit()) {
            self.advance();
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.advance();
            }
        }
        // Exponent part.
        if matches!(self.peek(), Some('e') | Some('E')) {
            let mut ahead = self.pos + 1;
            if matches!(self.chars.get(ahead), Some('+') | Some('-')) {
                ahead += 1;
            }
            if matches!(self.chars.get(ahead), Some(c) if c.is_ascii_digit()) {
                self.pos = ahead;
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.advance();
                }
            }
        }
        let text: String = self.chars[self.token_start..self.pos].iter().collect();
        let value = text
            .parse::<f64>()
            .map_err(|_| self.err(format!("invalid number '{}'", text)))?;
        Ok(TokenKind::Number(value))
    }

    fn identifier(&mut self) -> CompileResult<TokenKind> {
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_' || c == '$') {
            self.advance();
        }
        let text: String = self.chars[self.token_start..self.pos].iter().collect();
        Ok(match text.as_str() {
            "forge" => TokenKind::Forge,
            "craft" => TokenKind::Craft,
            "when" => TokenKind::When,
            "else" => TokenKind::Else,
            "whilst" => TokenKind::Whilst,
            "each" => TokenKind::Each,
            "in" => TokenKind::In,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "onward" => TokenKind::Onward,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "nil" => TokenKind::Nil,
            "adopt" => TokenKind::Adopt,
            _ => TokenKind::Identifier(text),
        })
    }

    fn string(&mut self) -> CompileResult<TokenKind> {
        self.advance(); // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    let escaped = self
                        .advance()
                        .ok_or_else(|| self.err("unterminated string"))?;
                    match escaped {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '\\' => out.push('\\'),
                        '"' => out.push('"'),
                        '\'' => out.push('\''),
                        '0' => out.push('\0'),
                        'e' => out.push('\x1b'),
                        c => {
                            return Err(self.err(format!("invalid escape sequence '\\{}'", c)));
                        }
                    }
                }
                Some(c) => {
                    out.push(c);
                    self.advance();
                }
            }
        }
        Ok(TokenKind::Str(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source, "test.cor")
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn basic_tokens() {
        let toks = kinds("forge x = 1 + 2.5;");
        assert_eq!(
            toks,
            vec![
                TokenKind::Forge,
                TokenKind::Identifier("x".into()),
                TokenKind::Equal,
                TokenKind::Number(1.0),
                TokenKind::Plus,
                TokenKind::Number(2.5),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn operators() {
        let toks = kinds("a == b != c <= d >= e && f || g");
        assert!(toks.contains(&TokenKind::EqualEqual));
        assert!(toks.contains(&TokenKind::BangEqual));
        assert!(toks.contains(&TokenKind::LessEqual));
        assert!(toks.contains(&TokenKind::GreaterEqual));
        assert!(toks.contains(&TokenKind::And));
        assert!(toks.contains(&TokenKind::Or));
    }

    #[test]
    fn ranges_and_power() {
        let toks = kinds("0..10 ..= 5 ** 2 **= 3");
        assert!(toks.contains(&TokenKind::DotDot));
        assert!(toks.contains(&TokenKind::DotDotEqual));
        assert!(toks.contains(&TokenKind::Power));
        assert!(toks.contains(&TokenKind::PowerEqual));
    }

    #[test]
    fn dot_vs_dotdot() {
        let toks = kinds("1.5 1..5 1.5.2");
        assert!(matches!(toks[0], TokenKind::Number(1.5)));
        assert!(matches!(toks[1], TokenKind::Number(1.0)));
        assert!(matches!(toks[2], TokenKind::DotDot));
        assert!(matches!(toks[3], TokenKind::Number(5.0)));
    }

    #[test]
    fn strings_and_escapes() {
        let toks = kinds(r#""hello\nworld""#);
        assert_eq!(toks[0], TokenKind::Str("hello\nworld".into()));
    }

    #[test]
    fn comments_are_skipped() {
        let toks = kinds("// line comment\nforge a = 1 /* block */ + 2;");
        assert_eq!(toks.len(), 8); // forge a = 1 + 2 ; eof
        assert!(toks.contains(&TokenKind::Forge));
    }

    #[test]
    fn corros_keywords() {
        let toks = kinds("forge craft when whilst each onward adopt");
        assert!(toks.contains(&TokenKind::Forge));
        assert!(toks.contains(&TokenKind::Craft));
        assert!(toks.contains(&TokenKind::When));
        assert!(toks.contains(&TokenKind::Whilst));
        assert!(toks.contains(&TokenKind::Each));
        assert!(toks.contains(&TokenKind::Onward));
        assert!(toks.contains(&TokenKind::Adopt));
    }

    #[test]
    fn unexpected_character() {
        let err = lex("let @ = 1", "test.cor").unwrap_err();
        assert!(err.message.contains("unexpected character"));
        assert_eq!(err.line, 1);
    }

    #[test]
    fn dollar_is_a_valid_identifier_char() {
        let toks = kinds("$method = 1; forge $$ = 2;");
        assert!(toks.contains(&TokenKind::Identifier("$method".to_string())));
        assert!(toks.contains(&TokenKind::Identifier("$$".to_string())));
    }

    #[test]
    fn unterminated_string() {
        let err = lex("\"oops", "test.cor").unwrap_err();
        assert!(err.message.contains("unterminated string"));
    }
}
