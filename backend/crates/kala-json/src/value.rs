use std::collections::BTreeMap;
use std::fmt;

use num_bigint::BigInt;

use crate::error::CanonicalJsonError;
use crate::text::JsonText;

/// A JSON object keyed in Python code-point order.
pub type CanonicalObject = BTreeMap<JsonText, CanonicalValue>;

/// A typed JSON value with the domains required for byte-compatible hashing.
///
/// This is deliberately not `serde_json::Value`: that type cannot preserve
/// arbitrary-precision integers by default, cannot represent lone surrogates,
/// and does not format floats like CPython.
///
/// The enum shape is unchanged, but the standard `Debug`, `Clone`, `PartialEq`
/// and `Drop` implementations are replaced with iterative ones. A persisted
/// canonical object may be nested tens of thousands of levels deep; the
/// compiler-generated recursive implementations would overflow a normal
/// thread stack while cloning, comparing, printing or dropping such a value
/// even after the parser itself became stack-safe. The iterative
/// implementations below use explicit heap stacks and preserve the ordinary
/// observable semantics (the same `Debug` text, equality, clone and drop
/// behaviour for shallow values).
pub enum CanonicalValue {
    /// JSON `null`.
    Null,
    /// JSON `true`/`false`.
    Bool(bool),
    /// An arbitrary-precision integer.
    Integer(BigInt),
    /// A finite IEEE-754 binary64 number.
    Float(f64),
    /// A JSON string with code-point fidelity.
    Text(JsonText),
    /// A JSON array.
    Array(Vec<CanonicalValue>),
    /// A JSON object.
    Object(CanonicalObject),
}

impl Clone for CanonicalValue {
    fn clone(&self) -> Self {
        // Post-order traversal with an explicit work stack. Completed clones
        // are collected in `results`; a container task consumes exactly the
        // results produced by its children. Each value is copied once.
        enum Work<'a> {
            Visit(&'a CanonicalValue),
            Array(usize),
            Object(Vec<&'a JsonText>),
        }

        let mut work = vec![Work::Visit(self)];
        let mut results: Vec<CanonicalValue> = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Work::Visit(value) => match value {
                    Self::Null => results.push(Self::Null),
                    Self::Bool(value) => results.push(Self::Bool(*value)),
                    Self::Integer(value) => results.push(Self::Integer(value.clone())),
                    Self::Float(value) => results.push(Self::Float(*value)),
                    Self::Text(value) => results.push(Self::Text(value.clone())),
                    Self::Array(values) => {
                        work.push(Work::Array(values.len()));
                        for item in values.iter().rev() {
                            work.push(Work::Visit(item));
                        }
                    }
                    Self::Object(entries) => {
                        work.push(Work::Object(entries.keys().collect()));
                        for item in entries.values().rev() {
                            work.push(Work::Visit(item));
                        }
                    }
                },
                Work::Array(len) => {
                    let start = results.len() - len;
                    let values = results.split_off(start);
                    results.push(Self::Array(values));
                }
                Work::Object(keys) => {
                    let start = results.len() - keys.len();
                    let values = results.split_off(start);
                    let mut entries = CanonicalObject::new();
                    for (key, value) in keys.into_iter().zip(values) {
                        entries.insert(key.clone(), value);
                    }
                    results.push(Self::Object(entries));
                }
            }
        }
        results.pop().expect("a clone produces exactly one value")
    }
}

impl PartialEq for CanonicalValue {
    fn eq(&self, other: &Self) -> bool {
        // Compare pairs in explicit-stack order rather than recursing through
        // child values. Comparison is commutative and order-insensitive for
        // the final result, so the stack order does not affect semantics.
        let mut stack = vec![(self, other)];
        while let Some((left, right)) = stack.pop() {
            match (left, right) {
                (Self::Null, Self::Null) => {}
                (Self::Bool(left), Self::Bool(right)) if left == right => {}
                (Self::Integer(left), Self::Integer(right)) if left == right => {}
                (Self::Float(left), Self::Float(right)) if left == right => {}
                (Self::Text(left), Self::Text(right)) if left == right => {}
                (Self::Array(left), Self::Array(right)) if left.len() == right.len() => {
                    for (left, right) in left.iter().zip(right) {
                        stack.push((left, right));
                    }
                }
                (Self::Object(left), Self::Object(right)) if left.len() == right.len() => {
                    for ((left_key, left_value), (right_key, right_value)) in left.iter().zip(right)
                    {
                        if left_key != right_key {
                            return false;
                        }
                        stack.push((left_value, right_value));
                    }
                }
                _ => return false,
            }
        }
        true
    }
}

impl fmt::Debug for CanonicalValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Reproduce the derived `Debug` text without recursive formatting.
        enum Task<'a> {
            Value(&'a CanonicalValue),
            Key(&'a JsonText),
            Text(&'static str),
        }

        let mut stack = vec![Task::Value(self)];
        while let Some(task) = stack.pop() {
            match task {
                Task::Text(text) => formatter.write_str(text)?,
                Task::Key(key) => write!(formatter, "{key:?}")?,
                Task::Value(value) => match value {
                    Self::Null => formatter.write_str("Null")?,
                    Self::Bool(value) => write!(formatter, "Bool({value:?})")?,
                    Self::Integer(value) => write!(formatter, "Integer({value:?})")?,
                    Self::Float(value) => write!(formatter, "Float({value:?})")?,
                    Self::Text(value) => write!(formatter, "Text({value:?})")?,
                    Self::Array(values) => {
                        formatter.write_str("Array([")?;
                        stack.push(Task::Text("])"));
                        for (index, item) in values.iter().enumerate().rev() {
                            stack.push(Task::Value(item));
                            if index > 0 {
                                stack.push(Task::Text(", "));
                            }
                        }
                    }
                    Self::Object(entries) => {
                        formatter.write_str("Object({")?;
                        stack.push(Task::Text("})"));
                        for (index, (key, item)) in entries.iter().enumerate().rev() {
                            stack.push(Task::Value(item));
                            stack.push(Task::Text(": "));
                            stack.push(Task::Key(key));
                            if index > 0 {
                                stack.push(Task::Text(", "));
                            }
                        }
                    }
                },
            }
        }
        Ok(())
    }
}

impl Drop for CanonicalValue {
    fn drop(&mut self) {
        // A recursive drop would overflow while unwinding a 50k-deep value.
        // Detach each container's children into a heap work list; every
        // detached container is empty by the time its own `Drop` runs, so the
        // implementation never recurses. `mem::take` is safe and moves no
        // fields out of a type with a `Drop` implementation.
        match self {
            Self::Array(items) if items.is_empty() => return,
            Self::Object(entries) if entries.is_empty() => return,
            Self::Array(_) | Self::Object(_) => {}
            _ => return,
        }

        let mut pending: Vec<CanonicalValue> = match self {
            Self::Array(items) => std::mem::take(items),
            Self::Object(entries) => std::mem::take(entries).into_values().collect(),
            _ => unreachable!("container checked above"),
        };
        while let Some(mut value) = pending.pop() {
            match &mut value {
                Self::Array(items) if !items.is_empty() => {
                    pending.append(&mut std::mem::take(items));
                }
                Self::Object(entries) if !entries.is_empty() => {
                    pending.extend(std::mem::take(entries).into_values());
                }
                _ => {}
            }
        }
    }
}

impl CanonicalValue {
    /// Construct a text value from a Rust string.
    pub fn text(value: impl Into<JsonText>) -> Self {
        Self::Text(value.into())
    }

    /// Construct an integer value from anything convertible to `BigInt`.
    pub fn integer(value: impl Into<BigInt>) -> Self {
        Self::Integer(value.into())
    }

    /// Construct a finite float value.
    pub fn float(value: f64) -> Result<Self, CanonicalJsonError> {
        if value.is_finite() {
            Ok(Self::Float(value))
        } else {
            Err(CanonicalJsonError::NonFinite)
        }
    }

    /// Construct an array value.
    pub fn array(values: impl IntoIterator<Item = CanonicalValue>) -> Self {
        Self::Array(values.into_iter().collect())
    }

    /// Construct an object from key/value pairs.
    pub fn object<K, V>(entries: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<JsonText>,
        V: Into<CanonicalValue>,
    {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }

    /// Return the string shape when this is a text value.
    pub fn as_text(&self) -> Option<&JsonText> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Return the integer shape when this is an integer value.
    pub fn as_integer(&self) -> Option<&BigInt> {
        match self {
            Self::Integer(value) => Some(value),
            _ => None,
        }
    }

    /// Return the float shape when this is a float value.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }
}

impl From<bool> for CanonicalValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for CanonicalValue {
    fn from(value: i64) -> Self {
        Self::Integer(BigInt::from(value))
    }
}

impl From<i32> for CanonicalValue {
    fn from(value: i32) -> Self {
        Self::Integer(BigInt::from(value))
    }
}

impl From<u64> for CanonicalValue {
    fn from(value: u64) -> Self {
        Self::Integer(BigInt::from(value))
    }
}

impl From<u32> for CanonicalValue {
    fn from(value: u32) -> Self {
        Self::Integer(BigInt::from(value))
    }
}

impl From<&str> for CanonicalValue {
    fn from(value: &str) -> Self {
        Self::Text(JsonText::new(value))
    }
}

impl From<String> for CanonicalValue {
    fn from(value: String) -> Self {
        Self::Text(JsonText::new(&value))
    }
}

impl From<BigInt> for CanonicalValue {
    fn from(value: BigInt) -> Self {
        Self::Integer(value)
    }
}

impl From<Vec<CanonicalValue>> for CanonicalValue {
    fn from(value: Vec<CanonicalValue>) -> Self {
        Self::Array(value)
    }
}

impl From<CanonicalObject> for CanonicalValue {
    fn from(value: CanonicalObject) -> Self {
        Self::Object(value)
    }
}
