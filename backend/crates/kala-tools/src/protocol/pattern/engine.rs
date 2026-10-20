//! A small backtracking matcher with CPython `re` semantics.
//!
//! The Python protocol module compiles a schema pattern with the standard
//! library `re` module and then runs `compiled.search(value)`. The Rust port
//! owns its own matcher so that it can reproduce CPython's exact behaviour on
//! the whole admitted pattern subset:
//!
//! * CPython's `str` based classes (`\w`, `\d`, `\s`) rather than the Unicode
//!   `Alphabetic`/`White_Space` properties used by other engines,
//! * `$` matching at the end of the subject *or* just before a final newline,
//! * `\Z` matching only at the very end,
//! * leftmost-first (Perl style) alternation preference,
//! * the CPython escape, character class and repetition syntax rules,
//! * matching over Unicode code points, so a lone surrogate is an ordinary
//!   character exactly as it is in CPython.
//!
//! The matcher is an explicit-stack backtracking VM: it never recurses, so a
//! hostile pattern cannot overflow the thread stack. The pattern admission
//! analysis in the parent module bounds the pattern length, the subject length
//! and the number of ways a repetition can split the subject, so the work is
//! bounded before the matcher runs.

use super::unicode::{ALNUM_RANGES, DECIMAL_RANGES, SPACE_RANGES};

/// The maximum number of matcher steps before a match is abandoned.
///
/// The admission analysis bounds the real work far below this limit; the
/// counter is a backstop that keeps a pathological pattern from hanging the
/// validator.
const MAX_STEPS: u64 = 16_000_000;

/// CPython's `MAXREPEAT`.
const MAX_REPEAT: u64 = 4_294_967_295;

/// The pattern is not a valid regular expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PatternError;

/// A shorthand character class.
#[derive(Clone, Copy, Debug)]
enum ClassKind {
    Digit,
    NotDigit,
    Space,
    NotSpace,
    Word,
    NotWord,
}

/// One member of a character class.
#[derive(Clone, Copy, Debug)]
enum ClassItem {
    Literal(u32),
    Range(u32, u32),
    Kind(ClassKind),
}

/// A character class, possibly negated.
#[derive(Clone, Debug)]
struct CharClass {
    negated: bool,
    items: Vec<ClassItem>,
}

impl CharClass {
    fn kind(kind: ClassKind) -> Self {
        Self {
            negated: false,
            items: vec![ClassItem::Kind(kind)],
        }
    }

    fn matches(&self, code_point: u32) -> bool {
        let mut hit = false;
        for item in &self.items {
            let matched = match item {
                ClassItem::Literal(value) => *value == code_point,
                ClassItem::Range(low, high) => code_point >= *low && code_point <= *high,
                ClassItem::Kind(kind) => match kind {
                    ClassKind::Digit => is_decimal(code_point),
                    ClassKind::NotDigit => !is_decimal(code_point),
                    ClassKind::Space => is_space(code_point),
                    ClassKind::NotSpace => !is_space(code_point),
                    ClassKind::Word => is_word(code_point),
                    ClassKind::NotWord => !is_word(code_point),
                },
            };
            if matched {
                hit = true;
                break;
            }
        }
        hit != self.negated
    }
}

/// The parsed shape of a pattern.
#[derive(Clone, Debug)]
enum Node {
    Literal(u32),
    Any,
    Class(CharClass),
    Start,
    End,
    StartText,
    EndText,
    WordBoundary,
    NotWordBoundary,
    Concat(Vec<Node>),
    Alternate(Vec<Node>),
    Repeat {
        node: Box<Node>,
        minimum: u32,
        maximum: Option<u32>,
        greedy: bool,
    },
}

/// One instruction of the compiled program.
#[derive(Clone, Copy, Debug)]
enum Inst {
    Literal(u32),
    Any,
    Class(usize),
    Start,
    End,
    StartText,
    EndText,
    WordBoundary,
    NotWordBoundary,
    Split(usize, usize),
    Jump(usize),
    Match,
}

/// A compiled pattern.
#[derive(Debug)]
pub(crate) struct Pattern {
    program: Vec<Inst>,
    classes: Vec<CharClass>,
}

impl Pattern {
    /// Compile a pattern from its code points.
    pub(crate) fn compile(chars: &[u32]) -> Result<Self, PatternError> {
        let mut parser = Parser {
            chars,
            pos: 0,
            names: Vec::new(),
        };
        let node = parser.parse_alternation()?;
        if parser.pos != chars.len() {
            return Err(PatternError);
        }
        let mut compiler = Compiler {
            program: Vec::new(),
            classes: Vec::new(),
        };
        compiler.emit_node(&node);
        compiler.program.push(Inst::Match);
        Ok(Self {
            program: compiler.program,
            classes: compiler.classes,
        })
    }

    /// Search the subject, returning the leftmost match span.
    pub(crate) fn search(&self, subject: &[u32]) -> Option<(usize, usize)> {
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let mut start = 0usize;
        let mut steps = 0u64;
        loop {
            let mut pc = 0usize;
            let mut position = start;
            stack.clear();
            loop {
                steps += 1;
                if steps > MAX_STEPS {
                    return None;
                }
                match self.program[pc] {
                    Inst::Match => return Some((start, position)),
                    Inst::Literal(expected) => {
                        if position < subject.len() && subject[position] == expected {
                            pc += 1;
                            position += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::Any => {
                        if position < subject.len() && subject[position] != u32::from(b'\n') {
                            pc += 1;
                            position += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::Class(index) => {
                        if position < subject.len()
                            && self.classes[index].matches(subject[position])
                        {
                            pc += 1;
                            position += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::Start => {
                        if position == 0 {
                            pc += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::End => {
                        if is_dollar_position(subject, position) {
                            pc += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::StartText => {
                        if position == 0 {
                            pc += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::EndText => {
                        if position == subject.len() {
                            pc += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::WordBoundary | Inst::NotWordBoundary => {
                        let at_boundary = is_word_boundary(subject, position);
                        let wanted = matches!(self.program[pc], Inst::WordBoundary);
                        if at_boundary == wanted {
                            pc += 1;
                        } else if let Some((next_pc, next_position)) = stack.pop() {
                            pc = next_pc;
                            position = next_position;
                        } else {
                            break;
                        }
                    }
                    Inst::Split(first, second) => {
                        stack.push((second, position));
                        pc = first;
                    }
                    Inst::Jump(target) => {
                        pc = target;
                    }
                }
            }
            if start >= subject.len() {
                return None;
            }
            start += 1;
        }
    }
}

/// Whether `$` matches at `position`.
///
/// CPython matches `$` at the end of the subject and immediately before a
/// newline that ends the subject.
fn is_dollar_position(subject: &[u32], position: usize) -> bool {
    if position == subject.len() {
        return true;
    }
    position + 1 == subject.len() && subject[position] == u32::from(b'\n')
}

/// Whether a word boundary sits at `position`.
fn is_word_boundary(subject: &[u32], position: usize) -> bool {
    let before = position > 0 && is_word(subject[position - 1]);
    let after = position < subject.len() && is_word(subject[position]);
    before != after
}

/// CPython's `str.isalnum() or '_'`.
fn is_word(code_point: u32) -> bool {
    code_point == u32::from(b'_') || in_ranges(ALNUM_RANGES, code_point)
}

/// CPython's `str.isdecimal()`.
pub(crate) fn is_decimal(code_point: u32) -> bool {
    in_ranges(DECIMAL_RANGES, code_point)
}

/// CPython's `str.isspace()`.
pub(crate) fn is_space(code_point: u32) -> bool {
    in_ranges(SPACE_RANGES, code_point)
}

/// Whether a code point falls inside one of the sorted ranges.
fn in_ranges(ranges: &[(u32, u32)], code_point: u32) -> bool {
    let mut low = 0usize;
    let mut high = ranges.len();
    while low < high {
        let middle = (low + high) / 2;
        let (start, end) = ranges[middle];
        if code_point < start {
            high = middle;
        } else if code_point > end {
            low = middle + 1;
        } else {
            return true;
        }
    }
    false
}

/// The pattern parser.
struct Parser<'a> {
    chars: &'a [u32],
    pos: usize,
    names: Vec<String>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u32> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u32> {
        self.chars.get(self.pos + offset).copied()
    }

    fn parse_alternation(&mut self) -> Result<Node, PatternError> {
        let mut branches = vec![self.parse_concat()?];
        while self.peek() == Some(u32::from(b'|')) {
            self.pos += 1;
            branches.push(self.parse_concat()?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().expect("one branch"))
        } else {
            Ok(Node::Alternate(branches))
        }
    }

    fn parse_concat(&mut self) -> Result<Node, PatternError> {
        let mut items: Vec<Node> = Vec::new();
        while let Some(code_point) = self.peek() {
            if code_point == u32::from(b'|') || code_point == u32::from(b')') {
                break;
            }
            let atom = self.parse_atom()?;
            items.push(self.parse_quantifier(atom)?);
        }
        Ok(Node::Concat(items))
    }

    fn parse_atom(&mut self) -> Result<Node, PatternError> {
        let code_point = self.peek().ok_or(PatternError)?;
        match code_point {
            value if value == u32::from(b'(') => self.parse_group(),
            value if value == u32::from(b'[') => self.parse_class(),
            value if value == u32::from(b'.') => {
                self.pos += 1;
                Ok(Node::Any)
            }
            value if value == u32::from(b'^') => {
                self.pos += 1;
                Ok(Node::Start)
            }
            value if value == u32::from(b'$') => {
                self.pos += 1;
                Ok(Node::End)
            }
            value if value == u32::from(b'\\') => self.parse_escape(),
            value
                if value == u32::from(b'*')
                    || value == u32::from(b'+')
                    || value == u32::from(b'?') =>
            {
                Err(PatternError)
            }
            value => {
                self.pos += 1;
                Ok(Node::Literal(value))
            }
        }
    }

    fn parse_group(&mut self) -> Result<Node, PatternError> {
        self.pos += 1;
        if self.peek() == Some(u32::from(b'?')) {
            self.pos += 1;
            match self.peek() {
                Some(value) if value == u32::from(b':') => {
                    self.pos += 1;
                }
                Some(value) if value == u32::from(b'P') => {
                    self.pos += 1;
                    if self.peek() != Some(u32::from(b'<')) {
                        return Err(PatternError);
                    }
                    self.pos += 1;
                    let name = self.parse_group_name()?;
                    if self.names.contains(&name) {
                        return Err(PatternError);
                    }
                    self.names.push(name);
                }
                _ => return Err(PatternError),
            }
        }
        let inner = self.parse_alternation()?;
        if self.peek() != Some(u32::from(b')')) {
            return Err(PatternError);
        }
        self.pos += 1;
        Ok(inner)
    }

    /// Parse a `(?P<name>` group name.
    ///
    /// CPython requires the name to be a valid Python identifier and rejects a
    /// duplicate name.
    fn parse_group_name(&mut self) -> Result<String, PatternError> {
        let start = self.pos;
        while let Some(code_point) = self.peek() {
            if code_point == u32::from(b'>') {
                break;
            }
            self.pos += 1;
        }
        if self.peek() != Some(u32::from(b'>')) {
            return Err(PatternError);
        }
        let name: String = self.chars[start..self.pos]
            .iter()
            .map(|code_point| char::from_u32(*code_point).unwrap_or('\u{FFFD}'))
            .collect();
        if name.is_empty() || !is_python_identifier(&name) {
            return Err(PatternError);
        }
        self.pos += 1;
        Ok(name)
    }

    fn parse_escape(&mut self) -> Result<Node, PatternError> {
        self.pos += 1;
        let code_point = self.peek().ok_or(PatternError)?;
        self.pos += 1;
        match code_point {
            value if value == u32::from(b'd') => Ok(Node::Class(CharClass::kind(ClassKind::Digit))),
            value if value == u32::from(b'D') => {
                Ok(Node::Class(CharClass::kind(ClassKind::NotDigit)))
            }
            value if value == u32::from(b's') => Ok(Node::Class(CharClass::kind(ClassKind::Space))),
            value if value == u32::from(b'S') => {
                Ok(Node::Class(CharClass::kind(ClassKind::NotSpace)))
            }
            value if value == u32::from(b'w') => Ok(Node::Class(CharClass::kind(ClassKind::Word))),
            value if value == u32::from(b'W') => {
                Ok(Node::Class(CharClass::kind(ClassKind::NotWord)))
            }
            value if value == u32::from(b'b') => Ok(Node::WordBoundary),
            value if value == u32::from(b'B') => Ok(Node::NotWordBoundary),
            value if value == u32::from(b'A') => Ok(Node::StartText),
            value if value == u32::from(b'Z') => Ok(Node::EndText),
            value if value == u32::from(b'a') => Ok(Node::Literal(0x07)),
            value if value == u32::from(b'f') => Ok(Node::Literal(0x0C)),
            value if value == u32::from(b'n') => Ok(Node::Literal(0x0A)),
            value if value == u32::from(b'r') => Ok(Node::Literal(0x0D)),
            value if value == u32::from(b't') => Ok(Node::Literal(0x09)),
            value if value == u32::from(b'v') => Ok(Node::Literal(0x0B)),
            value if value == u32::from(b'x') => {
                let literal = self.parse_hex(2)?;
                Ok(Node::Literal(literal))
            }
            value if value == u32::from(b'u') => {
                let literal = self.parse_hex(4)?;
                Ok(Node::Literal(literal))
            }
            value if value == u32::from(b'U') => {
                let literal = self.parse_hex(8)?;
                if literal > 0x10_FFFF {
                    return Err(PatternError);
                }
                Ok(Node::Literal(literal))
            }
            value if value == u32::from(b'N') => {
                // CPython resolves `\N{...}` through the Unicode character
                // name database. The Rust port refuses the escape instead of
                // guessing a name; the refusal is reported as an invalid
                // regular expression.
                Err(PatternError)
            }
            value if value == u32::from(b'0') => {
                let literal = self.parse_octal(2, false)?;
                Ok(Node::Literal(literal))
            }
            value if is_digit_code_point(value) => Err(PatternError),
            value => {
                if is_ascii_letter_code_point(value) {
                    return Err(PatternError);
                }
                Ok(Node::Literal(value))
            }
        }
    }

    /// Parse exactly `count` hexadecimal digits.
    fn parse_hex(&mut self, count: usize) -> Result<u32, PatternError> {
        let mut value = 0u32;
        for _ in 0..count {
            let digit = self.peek().ok_or(PatternError)?;
            let digit = hex_value(digit).ok_or(PatternError)?;
            value = (value << 4) | digit;
            self.pos += 1;
        }
        Ok(value)
    }

    /// Parse an octal escape of at most `extra` further digits.
    ///
    /// `bounded` selects the character class rule, which refuses a value
    /// above `0o377`, over the pattern rule, which does not.
    fn parse_octal(&mut self, extra: usize, bounded: bool) -> Result<u32, PatternError> {
        let mut value = 0u32;
        let mut digits = 0usize;
        while digits < extra {
            let Some(digit) = self.peek() else { break };
            if !is_octal_digit(digit) {
                break;
            }
            value = value * 8 + (digit - u32::from(b'0'));
            digits += 1;
            self.pos += 1;
        }
        if bounded && value > 0o377 {
            return Err(PatternError);
        }
        Ok(value)
    }

    /// Parse a character class.
    fn parse_class(&mut self) -> Result<Node, PatternError> {
        self.pos += 1;
        let negated = self.peek() == Some(u32::from(b'^'));
        if negated {
            self.pos += 1;
        }
        let mut items: Vec<ClassItem> = Vec::new();
        loop {
            let Some(code_point) = self.peek() else {
                return Err(PatternError);
            };
            if code_point == u32::from(b']') && !items.is_empty() {
                self.pos += 1;
                break;
            }
            let first = self.parse_class_item()?;
            if self.peek() == Some(u32::from(b'-'))
                && self.peek_at(1).is_some()
                && self.peek_at(1) != Some(u32::from(b']'))
            {
                self.pos += 1;
                let second = self.parse_class_item()?;
                let (ClassItem::Literal(low), ClassItem::Literal(high)) = (first, second) else {
                    return Err(PatternError);
                };
                if high < low {
                    return Err(PatternError);
                }
                items.push(ClassItem::Range(low, high));
            } else {
                items.push(first);
            }
        }
        Ok(Node::Class(CharClass { negated, items }))
    }

    /// Parse one member of a character class.
    fn parse_class_item(&mut self) -> Result<ClassItem, PatternError> {
        let code_point = self.peek().ok_or(PatternError)?;
        if code_point != u32::from(b'\\') {
            self.pos += 1;
            return Ok(ClassItem::Literal(code_point));
        }
        self.pos += 1;
        let escaped = self.peek().ok_or(PatternError)?;
        self.pos += 1;
        match escaped {
            value if value == u32::from(b'd') => Ok(ClassItem::Kind(ClassKind::Digit)),
            value if value == u32::from(b'D') => Ok(ClassItem::Kind(ClassKind::NotDigit)),
            value if value == u32::from(b's') => Ok(ClassItem::Kind(ClassKind::Space)),
            value if value == u32::from(b'S') => Ok(ClassItem::Kind(ClassKind::NotSpace)),
            value if value == u32::from(b'w') => Ok(ClassItem::Kind(ClassKind::Word)),
            value if value == u32::from(b'W') => Ok(ClassItem::Kind(ClassKind::NotWord)),
            value if value == u32::from(b'b') => Ok(ClassItem::Literal(0x08)),
            value if value == u32::from(b'a') => Ok(ClassItem::Literal(0x07)),
            value if value == u32::from(b'f') => Ok(ClassItem::Literal(0x0C)),
            value if value == u32::from(b'n') => Ok(ClassItem::Literal(0x0A)),
            value if value == u32::from(b'r') => Ok(ClassItem::Literal(0x0D)),
            value if value == u32::from(b't') => Ok(ClassItem::Literal(0x09)),
            value if value == u32::from(b'v') => Ok(ClassItem::Literal(0x0B)),
            value if value == u32::from(b'\\') => Ok(ClassItem::Literal(u32::from(b'\\'))),
            value if value == u32::from(b'x') => {
                let literal = self.parse_hex(2)?;
                Ok(ClassItem::Literal(literal))
            }
            value if value == u32::from(b'u') => {
                let literal = self.parse_hex(4)?;
                Ok(ClassItem::Literal(literal))
            }
            value if value == u32::from(b'U') => {
                let literal = self.parse_hex(8)?;
                if literal > 0x10_FFFF {
                    return Err(PatternError);
                }
                Ok(ClassItem::Literal(literal))
            }
            value if value == u32::from(b'N') => Err(PatternError),
            value if is_octal_digit(value) => {
                self.pos -= 1;
                let literal = self.parse_octal(2, true)?;
                Ok(ClassItem::Literal(literal))
            }
            value if is_digit_code_point(value) => Err(PatternError),
            value => {
                if is_ascii_letter_code_point(value) {
                    return Err(PatternError);
                }
                Ok(ClassItem::Literal(value))
            }
        }
    }

    /// Apply a quantifier to an atom.
    fn parse_quantifier(&mut self, atom: Node) -> Result<Node, PatternError> {
        let Some(code_point) = self.peek() else {
            return Ok(atom);
        };
        let (minimum, maximum) = match code_point {
            value if value == u32::from(b'*') => {
                self.pos += 1;
                (0u32, None)
            }
            value if value == u32::from(b'+') => {
                self.pos += 1;
                (1u32, None)
            }
            value if value == u32::from(b'?') => {
                self.pos += 1;
                (0u32, Some(1u32))
            }
            value if value == u32::from(b'{') => match self.parse_repeat_spec()? {
                Some(bounds) => bounds,
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        let greedy = if self.peek() == Some(u32::from(b'?')) {
            self.pos += 1;
            false
        } else {
            true
        };
        if matches!(
            self.peek(),
            Some(value)
                if value == u32::from(b'*')
                    || value == u32::from(b'+')
                    || value == u32::from(b'?')
        ) {
            return Err(PatternError);
        }
        if self.peek() == Some(u32::from(b'{')) && self.parse_repeat_spec()?.is_some() {
            return Err(PatternError);
        }
        Ok(Node::Repeat {
            node: Box::new(atom),
            minimum,
            maximum,
            greedy,
        })
    }

    /// Parse a `{n}`, `{n,}` or `{n,m}` repetition.
    ///
    /// Returns `None` when the text is not a repetition, in which case CPython
    /// treats the brace as a literal character.
    fn parse_repeat_spec(&mut self) -> Result<Option<(u32, Option<u32>)>, PatternError> {
        let start = self.pos;
        if self.peek() != Some(u32::from(b'{')) {
            return Ok(None);
        }
        self.pos += 1;
        if self.peek() == Some(u32::from(b'}')) {
            self.pos = start;
            return Ok(None);
        }
        let minimum = self.parse_decimal();
        let maximum = if self.peek() == Some(u32::from(b',')) {
            self.pos += 1;
            self.parse_decimal()
        } else {
            minimum
        };
        if self.peek() != Some(u32::from(b'}')) {
            self.pos = start;
            return Ok(None);
        }
        self.pos += 1;
        let minimum = minimum.unwrap_or(0);
        if let Some(maximum) = maximum {
            if u64::from(minimum) >= MAX_REPEAT || u64::from(maximum) >= MAX_REPEAT {
                return Err(PatternError);
            }
            if maximum < minimum {
                return Err(PatternError);
            }
        }
        Ok(Some((minimum, maximum)))
    }

    /// Parse a run of decimal digits.
    fn parse_decimal(&mut self) -> Option<u32> {
        let start = self.pos;
        let mut value = 0u64;
        while let Some(code_point) = self.peek() {
            if !is_digit_code_point(code_point) {
                break;
            }
            value = value
                .saturating_mul(10)
                .saturating_add(u64::from(code_point - u32::from(b'0')));
            self.pos += 1;
            if value > u64::from(u32::MAX) {
                self.pos = start;
                return Some(u32::MAX);
            }
        }
        if self.pos == start {
            None
        } else {
            Some(value as u32)
        }
    }
}

/// Whether a code point is an ASCII decimal digit.
fn is_digit_code_point(code_point: u32) -> bool {
    (u32::from(b'0')..=u32::from(b'9')).contains(&code_point)
}

/// Whether a code point is an ASCII octal digit.
fn is_octal_digit(code_point: u32) -> bool {
    (u32::from(b'0')..=u32::from(b'7')).contains(&code_point)
}

/// Whether a code point is an ASCII letter.
fn is_ascii_letter_code_point(code_point: u32) -> bool {
    (u32::from(b'a')..=u32::from(b'z')).contains(&code_point)
        || (u32::from(b'A')..=u32::from(b'Z')).contains(&code_point)
}

/// The value of a hexadecimal digit.
fn hex_value(code_point: u32) -> Option<u32> {
    if (u32::from(b'0')..=u32::from(b'9')).contains(&code_point) {
        Some(code_point - u32::from(b'0'))
    } else if (u32::from(b'a')..=u32::from(b'f')).contains(&code_point) {
        Some(code_point - u32::from(b'a') + 10)
    } else if (u32::from(b'A')..=u32::from(b'F')).contains(&code_point) {
        Some(code_point - u32::from(b'A') + 10)
    } else {
        None
    }
}

/// Whether a name is a valid Python identifier.
///
/// CPython requires a group name to be an identifier. The check uses the
/// Unicode alphabetic and alphanumeric properties, which is what CPython's
/// `str.isidentifier()` uses for the characters that matter here.
fn is_python_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first == '_' || first.is_alphabetic()) {
        return false;
    }
    characters.all(|character| character == '_' || character.is_alphanumeric())
}

/// The program compiler.
struct Compiler {
    program: Vec<Inst>,
    classes: Vec<CharClass>,
}

impl Compiler {
    fn emit_node(&mut self, node: &Node) {
        match node {
            Node::Literal(value) => self.program.push(Inst::Literal(*value)),
            Node::Any => self.program.push(Inst::Any),
            Node::Class(class) => {
                self.classes.push(class.clone());
                self.program.push(Inst::Class(self.classes.len() - 1));
            }
            Node::Start => self.program.push(Inst::Start),
            Node::End => self.program.push(Inst::End),
            Node::StartText => self.program.push(Inst::StartText),
            Node::EndText => self.program.push(Inst::EndText),
            Node::WordBoundary => self.program.push(Inst::WordBoundary),
            Node::NotWordBoundary => self.program.push(Inst::NotWordBoundary),
            Node::Concat(items) => {
                for item in items {
                    self.emit_node(item);
                }
            }
            Node::Alternate(branches) => self.emit_alternate(branches),
            Node::Repeat {
                node,
                minimum,
                maximum,
                greedy,
            } => self.emit_repeat(node, *minimum, *maximum, *greedy),
        }
    }

    /// Emit an alternation.
    ///
    /// The first branch has the highest priority, matching CPython's
    /// leftmost-first semantics.
    fn emit_alternate(&mut self, branches: &[Node]) {
        let count = branches.len();
        let mut jumps: Vec<usize> = Vec::new();
        for (index, branch) in branches.iter().enumerate() {
            if index + 1 < count {
                let split = self.program.len();
                self.program.push(Inst::Split(0, 0));
                let body = self.program.len();
                self.emit_node(branch);
                jumps.push(self.program.len());
                self.program.push(Inst::Jump(0));
                let next = self.program.len();
                self.program[split] = Inst::Split(body, next);
            } else {
                self.emit_node(branch);
            }
        }
        let end = self.program.len();
        for jump in jumps {
            self.program[jump] = Inst::Jump(end);
        }
    }

    /// Emit a repetition.
    ///
    /// A bounded repetition is unrolled into mandatory copies followed by
    /// optional copies; an unbounded repetition becomes a loop. The admission
    /// analysis bounds the unrolled size.
    fn emit_repeat(&mut self, node: &Node, minimum: u32, maximum: Option<u32>, greedy: bool) {
        for _ in 0..minimum {
            self.emit_node(node);
        }
        match maximum {
            Some(maximum) => {
                let optional = maximum.saturating_sub(minimum);
                for _ in 0..optional {
                    let split = self.program.len();
                    self.program.push(Inst::Split(0, 0));
                    let body = self.program.len();
                    self.emit_node(node);
                    let next = self.program.len();
                    self.program[split] = if greedy {
                        Inst::Split(body, next)
                    } else {
                        Inst::Split(next, body)
                    };
                }
            }
            None => {
                let split = self.program.len();
                self.program.push(Inst::Split(0, 0));
                let body = self.program.len();
                self.emit_node(node);
                self.program.push(Inst::Jump(split));
                let next = self.program.len();
                self.program[split] = if greedy {
                    Inst::Split(body, next)
                } else {
                    Inst::Split(next, body)
                };
            }
        }
    }
}
