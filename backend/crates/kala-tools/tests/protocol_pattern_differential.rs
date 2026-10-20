//! Live Python differential tests for the schema pattern layer.
//!
//! The pattern layer is the part of the protocol that is not delegated to an
//! accepted crate: the Rust port owns its own matcher so that it can
//! reproduce CPython's `re` semantics exactly. This test therefore compares
//! the admission analysis *and* the matching against the live Python module,
//! over a corpus that covers every construct of the admitted subset and every
//! construct the admission analysis has to refuse.

mod support;

use kala_json::{CanonicalValue, JsonText};
use kala_tools::protocol::{MAX_PATTERN_INPUT_LENGTH, validate_against_schema};
use support::{assert_same, error_result, ok_result, run_oracle};

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

fn array(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::Array(values)
}

/// The patterns the Python security suite refuses.
const AMBIGUOUS_REPETITION: [&str; 22] = [
    r"(a|aa)+$",
    r"(a|a)+$",
    r"(a|ab)*$",
    r"(|a)*",
    r"^(?:foo|bar)+$",
    r"(?:a|ab){1,64}",
    r"(a*)(a*)$",
    r"a*a*a*a*a*b",
    r"a*a*a*a*a*a*b$",
    r"[a-z]*[a-z]*[a-z]*q",
    r"\w+\s?\w+",
    r"^\w+@\w+\.\w+$",
    r"[-+]?[0-9]{1,3}",
    r"(a+)+$",
    r"(a*)+$",
    r"(?:a{1,64})*",
    r"^(a|a?)+$",
    r"(x+x+)+y",
    r"(.+)*z",
    r"(\w+\s?)*$",
    r"^(\s+)+x",
    r"(a+)*(b+)*",
];

/// The constructs outside the admitted subset.
const UNSUPPORTED_CONSTRUCTS: [&str; 19] = [
    r"(a)\1",
    r"(?P<n>a)(?P=n)",
    r"(?=x)y",
    r"(?<=x)y",
    r"(?!x)y",
    r"(?<!x)y",
    r"(a)?(?(1)b|c)",
    r"(?i)meter",
    r"(?im-sx:meter)",
    r"(?>a+)b",
    r"a*+",
    r"a++",
    r".*",
    r".+",
    r".{2,}",
    r"^.*$",
    r"(?#comment)",
    r"(?-i)x",
    r"(?P<>a)",
];

/// The repetitions beyond the backtracking budget.
const OVER_BUDGET_REPETITION: [&str; 8] = [
    r"a{0,16}b{0,16}c{0,16}d{0,16}",
    r"a{0,32}b{0,32}c{0,32}d{0,32}",
    r"(?:ab){0,64}(?:cd){0,64}",
    r"a{0,2048}",
    r"a{0,1001}",
    r"a{100000}",
    r"a{0,}",
    "",
];

/// The admitted subset, with a sample that must match.
const ADMITTED_SUBSET: [(&str, &str); 15] = [
    (r"^bar-[0-9]{2}$", "bar-42"),
    (r"^[A-Za-z0-9._-]{1,64}$", "report.v1_final"),
    (r"^/[\w./-]{0,200}$", "/a/b-c.json"),
    (r"^\d{4}-\d{2}-\d{2}$", "2026-05-06"),
    (r"^(?:abc)+$", "abcabcabc"),
    (r"^(ab)+$", "abab"),
    (r"^(?:ab){2,3}$", "ababab"),
    (r"^(a+)$", "aaa"),
    (r"^[a-z]+$", "abc"),
    (r"^a?b$", "b"),
    (r"^[0-9]{1,3}$", "102"),
    (r"^\(\)$", "()"),
    (r"a|bc", "bc"),
    (r"^[^,]{0,10}$", "no commas"),
    (r"^\w+\s\w+$", "two words"),
];

/// The patterns that exercise the matcher itself.
const MATCHER_PATTERNS: [&str; 48] = [
    r"^abc$",
    r"abc",
    r"a.c",
    r"^a.c$",
    r"a[bc]d",
    r"a[^bc]d",
    r"a[b-d]e",
    r"a\]b",
    r"a\\b",
    r"a\.b",
    r"a\*b",
    r"^\d\d$",
    r"^\D\d$",
    r"^\s\d$",
    r"^\S\d$",
    r"^\w\d$",
    r"^\W\d$",
    r"^\bword\b$",
    r"^\Bord$",
    r"^a$",
    r"a$",
    r"^a\n$",
    r"^\Aa$",
    r"a\Z",
    r"(a)(b)",
    r"(?:a)(b)",
    r"(?P<name>a)b",
    r"a{2}",
    r"a{2,}",
    r"a{2,4}",
    r"a{0,2}",
    r"a*",
    r"a+",
    r"a?",
    r"(a|b)+",
    r"a|b",
    r"[[:alpha:]]",
    r"[\w.-]+",
    r"^[^,]*$",
    r"[-a]",
    r"[a-]",
    r"\x41",
    r"\u0041",
    r"\101",
    r"é",
    r"^\w+$",
    r"^.$",
    r"^\S+$",
];

fn samples() -> Vec<CanonicalValue> {
    vec![
        text(""),
        text("a"),
        text("b"),
        text("ab"),
        text("abc"),
        text("abcabc"),
        text("abab"),
        text("ababab"),
        text("aaa"),
        text("ABC"),
        text("123"),
        text("2026-05-06"),
        text("bar-42"),
        text("bar-4"),
        text("/a/b-c.json"),
        text("report.v1_final"),
        text("()"),
        text("no commas"),
        text("two words"),
        text("a\nb"),
        text("a\n"),
        text("\n"),
        text("a.b"),
        text("a*b"),
        text("a\\b"),
        text("a]b"),
        text("a[b"),
        text("a-b"),
        text("a_b"),
        text("word"),
        text("wordX"),
        text("ord"),
        text("é"),
        text("½"),
        text("١"),
        text(" "),
        text("\t"),
        text(" x "),
        text("A"),
        text("aXb"),
        text("aXbXc"),
        text("!"),
        text("a1b2"),
        text("a_b.c-d"),
        text(&"x".repeat(64)),
        text(&"a".repeat(64)),
    ]
}

fn corpus() -> Vec<CanonicalValue> {
    let mut cases: Vec<CanonicalValue> = Vec::new();
    let values = samples();
    let mut push = |pattern: &str, maximum: Option<i64>| {
        let mut entries = vec![("pattern", text(pattern))];
        if let Some(maximum) = maximum {
            entries.push(("maxLength", CanonicalValue::Integer(maximum.into())));
        }
        entries.push(("values", array(values.clone())));
        cases.push(object(entries));
    };
    for pattern in AMBIGUOUS_REPETITION {
        push(pattern, Some(MAX_PATTERN_INPUT_LENGTH as i64));
    }
    for pattern in UNSUPPORTED_CONSTRUCTS {
        push(pattern, Some(MAX_PATTERN_INPUT_LENGTH as i64));
    }
    for pattern in OVER_BUDGET_REPETITION {
        push(pattern, Some(MAX_PATTERN_INPUT_LENGTH as i64));
    }
    for (pattern, _) in ADMITTED_SUBSET {
        push(pattern, Some(MAX_PATTERN_INPUT_LENGTH as i64));
    }
    for pattern in MATCHER_PATTERNS {
        push(pattern, Some(MAX_PATTERN_INPUT_LENGTH as i64));
    }
    // Patterns without a length bound, and patterns with a bound that is too
    // small or too large.
    push("^a$", None);
    push("^a$", Some(0));
    push("^a$", Some(MAX_PATTERN_INPUT_LENGTH as i64 + 1));
    push("^a$", Some(4_096));
    push("^a$", Some(8));
    cases
}

fn native_case(case: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = case else {
        panic!("case is not an object")
    };
    let pattern = entries
        .get(&JsonText::new("pattern"))
        .and_then(|value| value.as_text())
        .expect("pattern")
        .to_string_lossy();
    let maximum = entries
        .get(&JsonText::new("maxLength"))
        .and_then(|value| value.as_integer())
        .map(|value| value.to_string().parse::<i128>().expect("maxLength"));
    let mut schema = CanonicalObjectBuilder::new();
    schema.set("type", text("string"));
    if let Some(maximum) = maximum {
        schema.set("maxLength", CanonicalValue::Integer(maximum.into()));
    }
    schema.set("pattern", text(&pattern));
    let schema = schema.build();
    let CanonicalValue::Array(values) = entries.get(&JsonText::new("values")).expect("values")
    else {
        panic!("values is not an array")
    };
    let mut outcomes = Vec::with_capacity(values.len());
    for value in values {
        let outcome = match validate_against_schema(value, &schema, "$", 0) {
            Ok(()) => ok_result(CanonicalValue::Null),
            Err(error) => error_result(&error),
        };
        outcomes.push(outcome);
    }
    object(vec![("values", array(outcomes))])
}

/// A small ordered object builder for the schema of one case.
struct CanonicalObjectBuilder {
    entries: Vec<(String, CanonicalValue)>,
}

impl CanonicalObjectBuilder {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn set(&mut self, key: &str, value: CanonicalValue) {
        self.entries.push((key.to_owned(), value));
    }

    fn build(self) -> CanonicalValue {
        let mut entries = kala_json::CanonicalObject::new();
        for (key, value) in self.entries {
            entries.insert(JsonText::new(&key), value);
        }
        CanonicalValue::Object(entries)
    }
}

/// A deterministic pseudo random number generator.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn pick<'a, T>(&mut self, values: &'a [T]) -> &'a T {
        &values[(self.next() as usize) % values.len()]
    }
}

/// Build a random pattern from the constructs of the admitted subset.
fn random_pattern(random: &mut Lcg, depth: usize) -> String {
    let atoms: [&str; 16] = [
        "a", "b", "c", "1", "-", "_", "/", ".", "é", " ", "x", "y", "z", "0", ":", "%",
    ];
    let classes: [&str; 10] = [
        "[abc]",
        "[^abc]",
        "[a-z]",
        "[0-9]",
        "[\\w]",
        "[\\d]",
        "[\\s]",
        "[.-]",
        "[a-zA-Z0-9_-]",
        "[\\w.-]",
    ];
    let escapes: [&str; 8] = ["\\.", "\\\\", "\\+", "\\[", "\\]", "\\-", "\\n", "\\t"];
    let mut out = String::new();
    let length = 1 + random.below(6) as usize;
    for _ in 0..length {
        match random.below(10) {
            0 | 1 => out.push_str(random.pick(&atoms)),
            2 | 3 => out.push_str(random.pick(&classes)),
            4 => out.push_str(random.pick(&escapes)),
            5 => out.push('^'),
            6 => out.push('$'),
            7 if depth < 2 => {
                out.push_str("(?:");
                out.push_str(&random_pattern(random, depth + 1));
                out.push(')');
            }
            8 if depth < 2 => {
                out.push('(');
                out.push_str(&random_pattern(random, depth + 1));
                out.push(')');
            }
            _ => out.push_str(random.pick(&atoms)),
        }
        match random.below(8) {
            0 => out.push('*'),
            1 => out.push('+'),
            2 => out.push('?'),
            3 => out.push_str("{2}"),
            4 => out.push_str("{1,3}"),
            5 => out.push_str("{0,2}"),
            _ => {}
        }
    }
    if out.len() > 120 {
        out.truncate(120);
    }
    out
}

/// Random subject strings for the randomised pattern corpus.
fn random_values(random: &mut Lcg) -> Vec<CanonicalValue> {
    let alphabet = ['a', 'b', 'c', '1', '-', '_', '/', '.', ' ', 'é'];
    let mut values = Vec::with_capacity(24);
    for _ in 0..24 {
        let length = random.below(8) as usize;
        let mut out = String::new();
        for _ in 0..length {
            out.push(*random.pick(&alphabet));
        }
        values.push(text(&out));
    }
    values
}

fn randomized_corpus() -> Vec<CanonicalValue> {
    let mut random = Lcg::new(0x5041_5454_4E52_3101);
    let mut cases = Vec::with_capacity(300);
    for _ in 0..300 {
        let pattern = random_pattern(&mut random, 0);
        let values = random_values(&mut random);
        cases.push(object(vec![
            ("pattern", text(&pattern)),
            ("maxLength", CanonicalValue::Integer(64.into())),
            ("values", array(values)),
        ]));
    }
    cases
}

#[test]
fn the_pattern_corpus_sizes_are_stable() {
    assert_eq!(corpus().len(), 117);
    assert_eq!(randomized_corpus().len(), 300);
    assert_eq!(samples().len(), 46);
    assert_eq!(random_values(&mut Lcg::new(1)).len(), 24);
}

#[test]
fn pattern_admission_and_matching_match_live_python() {
    let corpus = corpus();
    let oracle = run_oracle("pattern", &CanonicalValue::Array(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (index, (case, expected)) in corpus.iter().zip(oracle).enumerate() {
        let native = native_case(case);
        let CanonicalValue::Object(entries) = case else {
            panic!("case is not an object")
        };
        let pattern = entries
            .get(&JsonText::new("pattern"))
            .and_then(|value| value.as_text())
            .expect("pattern")
            .to_string_lossy();
        assert_same(
            &native,
            &expected,
            &format!("pattern corpus case {index} ({pattern:?})"),
        );
    }
}

#[test]
fn randomised_patterns_match_live_python() {
    let corpus = randomized_corpus();
    let oracle = run_oracle("pattern", &CanonicalValue::Array(corpus.clone()), None);
    assert_eq!(oracle.len(), corpus.len());
    for (index, (case, expected)) in corpus.iter().zip(oracle).enumerate() {
        let native = native_case(case);
        let CanonicalValue::Object(entries) = case else {
            panic!("case is not an object")
        };
        let pattern = entries
            .get(&JsonText::new("pattern"))
            .and_then(|value| value.as_text())
            .expect("pattern")
            .to_string_lossy();
        assert_same(
            &native,
            &expected,
            &format!("randomised pattern case {index} ({pattern:?})"),
        );
    }
}
