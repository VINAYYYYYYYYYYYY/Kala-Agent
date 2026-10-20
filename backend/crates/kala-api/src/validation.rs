//! Pydantic-compatible validation primitives.
//!
//! The Python source validates its request and response models with Pydantic
//! v2, and FastAPI renders the failures of a rejected body as a list of
//! entries, each with a machine readable type, a location, a message, the
//! offending input and an optional context. This module is the vocabulary of
//! those failures, so a native model can refuse a value with the same kind,
//! the same location, the same message and the same context the Python source
//! reports.
//!
//! The vocabulary was taken from the live Python source, not from memory:
//!
//! * a field that is not declared is `extra_forbidden`, "Extra inputs are not
//!   permitted";
//! * a field that is missing is `missing`, "Field required", and the recorded
//!   input is the whole object that was validated;
//! * a value of the wrong type is `string_type`, `int_type`, `float_type`,
//!   `bool_type`, `tuple_type`, `dict_type` or `model_type`;
//! * a bounded string is `string_too_short` or `string_too_long`, with the
//!   bound in the context;
//! * a patterned string is `string_pattern_mismatch`, with the pattern in the
//!   context;
//! * an enum value is `enum`, with the accepted values in the context;
//! * a field validator that raises is `value_error`, with the raised text
//!   after the "Value error, " prefix.
//!
//! The errors of a model are reported in the order the model declares its
//! fields, and the errors of the undeclared fields follow, in the order the
//! input carries them. That is the order Pydantic reports, and the native
//! models keep it.

use kala_json::{CanonicalObject, CanonicalValue};
use num_bigint::BigInt;

/// One step of the path to a rejected value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A field of a model.
    Field(String),
    /// An element of a sequence.
    Index(usize),
}

impl Location {
    /// The rendering FastAPI uses inside a location.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::Field(name) => name.clone(),
            Self::Index(index) => index.to_string(),
        }
    }
}

/// The machine readable kinds Pydantic reports.
pub mod kind {
    /// A field the model does not declare.
    pub const EXTRA_FORBIDDEN: &str = "extra_forbidden";
    /// A field the model requires and the input does not carry.
    pub const MISSING: &str = "missing";
    /// A value that is not the model it is declared as.
    pub const MODEL_TYPE: &str = "model_type";
    /// A value that is not a string.
    pub const STRING_TYPE: &str = "string_type";
    /// A string that carries a code point that is not a character.
    pub const STRING_UNICODE: &str = "string_unicode";
    /// A string that is shorter than the bound.
    pub const STRING_TOO_SHORT: &str = "string_too_short";
    /// A string that is longer than the bound.
    pub const STRING_TOO_LONG: &str = "string_too_long";
    /// A string that does not match the pattern.
    pub const STRING_PATTERN_MISMATCH: &str = "string_pattern_mismatch";
    /// A value that is not an integer.
    pub const INT_TYPE: &str = "int_type";
    /// A string that is not an integer.
    pub const INT_PARSING: &str = "int_parsing";
    /// A number with a fractional part where an integer is declared.
    pub const INT_FROM_FLOAT: &str = "int_from_float";
    /// A value that is not a number.
    pub const FLOAT_TYPE: &str = "float_type";
    /// A string that is not a number.
    pub const FLOAT_PARSING: &str = "float_parsing";
    /// A value that is not a boolean.
    pub const BOOL_TYPE: &str = "bool_type";
    /// A value that cannot be read as a boolean.
    pub const BOOL_PARSING: &str = "bool_parsing";
    /// A value that is not one of the accepted enum values.
    pub const ENUM: &str = "enum";
    /// A value that is not a sequence.
    pub const TUPLE_TYPE: &str = "tuple_type";
    /// A value that is not a mapping.
    pub const DICT_TYPE: &str = "dict_type";
    /// A field validator that refused the value.
    pub const VALUE_ERROR: &str = "value_error";
    /// A body that is not an object, refused by the body layer of the
    /// framework and not by the model.
    pub const MODEL_ATTRIBUTES_TYPE: &str = "model_attributes_type";
    /// A value that is not a timestamp.
    pub const DATETIME_TYPE: &str = "datetime_type";
    /// A timestamp that cannot be read.
    pub const DATETIME_PARSING: &str = "datetime_from_date_parsing";
    /// A timestamp that carries no offset.
    pub const TIMEZONE_AWARE: &str = "timezone_aware";
}

/// A context value of a failure.
///
/// Pydantic attaches the bound or the expectation that was violated, and
/// FastAPI renders it under the `ctx` key of the failure.
#[derive(Clone, Debug, PartialEq)]
pub enum Context {
    /// An integer bound.
    Integer(i64),
    /// A text bound or expectation.
    Text(String),
}

impl Context {
    /// The JSON rendering of the context value.
    #[must_use]
    pub fn to_value(&self) -> CanonicalValue {
        match self {
            Self::Integer(number) => CanonicalValue::from(*number),
            Self::Text(text) => CanonicalValue::from(text.as_str()),
        }
    }
}

/// One rejected value.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    /// The machine readable kind of the failure.
    pub kind: String,
    /// The path to the rejected value, most specific part last.
    pub location: Vec<Location>,
    /// The human readable message, in the Python source's wording.
    pub message: String,
    /// The rejected value.
    pub input: CanonicalValue,
    /// The context of the failure, empty when the failure carries none.
    pub context: Vec<(String, Context)>,
}

impl Issue {
    /// Build a failure without a context.
    #[must_use]
    pub fn new(
        kind: &str,
        location: Vec<Location>,
        message: impl Into<String>,
        input: &CanonicalValue,
    ) -> Self {
        Self {
            kind: kind.to_owned(),
            location,
            message: message.into(),
            input: input.clone(),
            context: Vec::new(),
        }
    }

    /// Build a failure with a context.
    #[must_use]
    pub fn with_context(
        kind: &str,
        location: Vec<Location>,
        message: impl Into<String>,
        input: &CanonicalValue,
        context: Vec<(String, Context)>,
    ) -> Self {
        Self {
            kind: kind.to_owned(),
            location,
            message: message.into(),
            input: input.clone(),
            context,
        }
    }

    /// Put the failure under one more step of the path.
    #[must_use]
    pub fn under(mut self, parent: &Location) -> Self {
        self.location.insert(0, parent.clone());
        self
    }

    /// The location FastAPI renders, with the field names and the indices.
    #[must_use]
    pub fn location_text(&self) -> Vec<String> {
        self.location.iter().map(Location::as_text).collect()
    }
}

/// Every failure of one validation.
pub type Failures = Vec<Issue>;

/// How a model reads its input.
///
/// Pydantic reports a value that is not a mapping differently in the two
/// modes: `model_validate` refuses it as a `model_type`, and the body layer
/// of FastAPI, which reads a body with `from_attributes=True`, refuses it as
/// a `model_attributes_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    /// `model_validate`: the input is a mapping of the fields of the model.
    Mapping,
    /// `model_validate(..., from_attributes=True)`: the body layer of the
    /// framework reads a body this way.
    Attributes,
}

/// The reader of one nested model.
pub type ModelReader<T> = fn(&CanonicalValue, &[Location], Reading) -> Result<T, Failures>;

/// The fields of one model, read in the order the model declares them.
///
/// The reader collects the failures instead of stopping at the first one, so a
/// rejected body reports every problem it carries, exactly as Pydantic does.
pub struct Fields<'a> {
    object: &'a CanonicalObject,
    failures: Failures,
    path: Vec<Location>,
    strip: bool,
    reading: Reading,
}

impl<'a> Fields<'a> {
    /// Read the fields of one model.
    ///
    /// `strip` mirrors the `str_strip_whitespace` configuration of the model:
    /// the request models strip the surrounding whitespace of their string
    /// fields before the bounds and the pattern are checked, and the response
    /// models do not.
    ///
    /// `path` is the path of the model itself, which the nested models carry
    /// so that a failure of a nested field is reported where it happened.
    #[must_use]
    pub fn new(object: &'a CanonicalObject, strip: bool) -> Self {
        Self::new_at(object, strip, &[])
    }

    /// Read the fields of one model at a path.
    #[must_use]
    pub fn new_at(object: &'a CanonicalObject, strip: bool, path: &[Location]) -> Self {
        Self::new_reading_at(object, strip, path, Reading::Mapping)
    }

    /// Read the fields of one model at a path, in one reading mode.
    #[must_use]
    pub fn new_reading_at(
        object: &'a CanonicalObject,
        strip: bool,
        path: &[Location],
        reading: Reading,
    ) -> Self {
        Self {
            object,
            failures: Vec::new(),
            path: path.to_vec(),
            strip,
            reading,
        }
    }

    /// The failures collected so far.
    #[must_use]
    pub fn failures(&self) -> &[Issue] {
        &self.failures
    }

    /// Record one failure.
    pub fn push_failure(&mut self, issue: Issue) {
        self.failures.push(issue);
    }

    /// Finish the validation.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn finish(self) -> Result<(), Failures> {
        if self.failures.is_empty() {
            Ok(())
        } else {
            Err(self.failures)
        }
    }

    /// The value of one field, when the input carries it.
    ///
    /// A field the input carries as null is a value, not an absent field: the
    /// Python source reports a null as a type failure and not as a missing
    /// field.
    fn raw_value(&self, name: &str) -> Option<&'a CanonicalValue> {
        self.object.get(&kala_json::JsonText::new(name))
    }

    /// The location of one field.
    fn location(&self, name: &str) -> Vec<Location> {
        let mut path = self.path.clone();
        path.push(Location::Field(name.to_owned()));
        path
    }

    /// Record the failure of a missing field.
    fn missing(&mut self, name: &str) {
        self.failures.push(Issue::new(
            kind::MISSING,
            self.location(name),
            "Field required",
            &CanonicalValue::Object(self.object.clone()),
        ));
    }

    /// Record the failure of an undeclared field.
    fn extra(&mut self, name: &str, value: &CanonicalValue) {
        self.failures.push(Issue::new(
            kind::EXTRA_FORBIDDEN,
            self.location(name),
            "Extra inputs are not permitted",
            value,
        ));
    }

    /// Refuse every field the model does not declare.
    ///
    /// The undeclared fields are reported after the failures of the declared
    /// fields, in the order the value model keeps them: the keys of an object
    /// are sorted by code point.
    pub fn reject_extra(&mut self, known: &[&str]) {
        for (name, value) in self.object.iter() {
            let name = name.to_string_lossy();
            if !known.contains(&name.as_str()) {
                self.extra(&name, value);
            }
        }
    }

    /// Read one required string field.
    pub fn required_text(&mut self, name: &str) -> Option<String> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match self.read_text(name, value) {
                Ok(text) => Some(text),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional string field.
    pub fn optional_text(&mut self, name: &str) -> Option<Option<String>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_text(name, value) {
                Ok(text) => Some(Some(text)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required string field that may be null.
    ///
    /// A field declared as `str | None` without a default is required: the
    /// input must carry the key, and the value may be null.
    pub fn required_nullable_text(&mut self, name: &str) -> Option<Option<String>> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_text(name, value) {
                Ok(text) => Some(Some(text)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one string field with a default.
    pub fn default_text(&mut self, name: &str, default: &str) -> String {
        self.optional_text(name)
            .flatten()
            .unwrap_or_else(|| default.to_owned())
    }

    /// Read one required string field with bounds.
    pub fn required_text_bounded(
        &mut self,
        name: &str,
        minimum: Option<usize>,
        maximum: Option<usize>,
    ) -> Option<String> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match self.read_text(name, value) {
                Ok(text) => match self.check_length(name, &text, value, minimum, maximum) {
                    Ok(()) => Some(text),
                    Err(issue) => {
                        self.failures.push(*issue);
                        None
                    }
                },
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional string field with bounds.
    pub fn optional_text_bounded(
        &mut self,
        name: &str,
        minimum: Option<usize>,
        maximum: Option<usize>,
    ) -> Option<Option<String>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_text(name, value) {
                Ok(text) => match self.check_length(name, &text, value, minimum, maximum) {
                    Ok(()) => Some(Some(text)),
                    Err(issue) => {
                        self.failures.push(*issue);
                        None
                    }
                },
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one string field with bounds and a default.
    pub fn default_text_bounded(
        &mut self,
        name: &str,
        default: &str,
        minimum: Option<usize>,
        maximum: Option<usize>,
    ) -> String {
        self.optional_text_bounded(name, minimum, maximum)
            .flatten()
            .unwrap_or_else(|| default.to_owned())
    }

    /// Read one required string field with a pattern.
    pub fn required_text_pattern(&mut self, name: &str, pattern: &str) -> Option<String> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match self.read_text(name, value) {
                Ok(text) => match self.check_pattern(name, &text, value, pattern) {
                    Ok(()) => Some(text),
                    Err(issue) => {
                        self.failures.push(*issue);
                        None
                    }
                },
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional string field with a pattern.
    pub fn optional_text_pattern(&mut self, name: &str, pattern: &str) -> Option<Option<String>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_text(name, value) {
                Ok(text) => match self.check_pattern(name, &text, value, pattern) {
                    Ok(()) => Some(Some(text)),
                    Err(issue) => {
                        self.failures.push(*issue);
                        None
                    }
                },
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required string field with bounds and a pattern.
    ///
    /// The bounds are checked before the pattern, which is the order Pydantic
    /// checks them in.
    pub fn required_text_bounded_pattern(
        &mut self,
        name: &str,
        minimum: Option<usize>,
        maximum: Option<usize>,
        pattern: &str,
    ) -> Option<String> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match self.read_text(name, value) {
                Ok(text) => match self.check_length(name, &text, value, minimum, maximum) {
                    Ok(()) => match self.check_pattern(name, &text, value, pattern) {
                        Ok(()) => Some(text),
                        Err(issue) => {
                            self.failures.push(*issue);
                            None
                        }
                    },
                    Err(issue) => {
                        self.failures.push(*issue);
                        None
                    }
                },
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional string field with bounds and a pattern.
    pub fn optional_text_bounded_pattern(
        &mut self,
        name: &str,
        minimum: Option<usize>,
        maximum: Option<usize>,
        pattern: &str,
    ) -> Option<Option<String>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_text(name, value) {
                Ok(text) => match self.check_length(name, &text, value, minimum, maximum) {
                    Ok(()) => match self.check_pattern(name, &text, value, pattern) {
                        Ok(()) => Some(Some(text)),
                        Err(issue) => {
                            self.failures.push(*issue);
                            None
                        }
                    },
                    Err(issue) => {
                        self.failures.push(*issue);
                        None
                    }
                },
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one string field with bounds, a pattern and a default.
    pub fn default_text_bounded_pattern(
        &mut self,
        name: &str,
        default: &str,
        minimum: Option<usize>,
        maximum: Option<usize>,
        pattern: &str,
    ) -> String {
        self.optional_text_bounded_pattern(name, minimum, maximum, pattern)
            .flatten()
            .unwrap_or_else(|| default.to_owned())
    }

    /// Read one required integer field.
    pub fn required_integer(&mut self, name: &str) -> Option<BigInt> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match read_integer(value, &self.location(name)) {
                Ok(number) => Some(number),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional integer field.
    pub fn optional_integer(&mut self, name: &str) -> Option<Option<BigInt>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match read_integer(value, &self.location(name)) {
                Ok(number) => Some(Some(number)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required number field.
    pub fn required_number(&mut self, name: &str) -> Option<f64> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match read_number(value, &self.location(name)) {
                Ok(number) => Some(number),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required number field that may be null.
    pub fn required_nullable_number(&mut self, name: &str) -> Option<Option<f64>> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(CanonicalValue::Null) => Some(None),
            Some(value) => match read_number(value, &self.location(name)) {
                Ok(number) => Some(Some(number)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional number field.
    pub fn optional_number(&mut self, name: &str) -> Option<Option<f64>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match read_number(value, &self.location(name)) {
                Ok(number) => Some(Some(number)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required boolean field.
    pub fn required_bool(&mut self, name: &str) -> Option<bool> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match read_bool(value, &self.location(name)) {
                Ok(flag) => Some(flag),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional boolean field.
    pub fn optional_bool(&mut self, name: &str) -> Option<Option<bool>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match read_bool(value, &self.location(name)) {
                Ok(flag) => Some(Some(flag)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required enum field.
    ///
    /// The accepted values are the strings of the Python enum, in declaration
    /// order. The message is the one Pydantic renders: "Input should be
    /// 'first', 'second' or 'third'".
    pub fn required_enum(&mut self, name: &str, accepted: &[&str]) -> Option<String> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match self.read_enum(name, value, accepted) {
                Ok(text) => Some(text),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required enum field that may be null.
    pub fn required_nullable_enum(
        &mut self,
        name: &str,
        accepted: &[&str],
    ) -> Option<Option<String>> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_enum(name, value, accepted) {
                Ok(text) => Some(Some(text)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional enum field.
    pub fn optional_enum(&mut self, name: &str, accepted: &[&str]) -> Option<Option<String>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_enum(name, value, accepted) {
                Ok(text) => Some(Some(text)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required sequence of strings.
    pub fn required_text_tuple(&mut self, name: &str) -> Option<Vec<String>> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match self.read_text_tuple(name, value) {
                Ok(items) => Some(items),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional sequence of strings.
    pub fn optional_text_tuple(&mut self, name: &str) -> Option<Option<Vec<String>>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match self.read_text_tuple(name, value) {
                Ok(items) => Some(Some(items)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required sequence of models.
    pub fn required_model_tuple<T>(
        &mut self,
        name: &str,
        parse: impl Fn(&CanonicalValue, &[Location], Reading) -> Result<T, Failures>,
    ) -> Option<Vec<T>> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(CanonicalValue::Null) => {
                self.failures.push(Issue::new(
                    kind::TUPLE_TYPE,
                    self.location(name),
                    "Input should be a valid tuple",
                    &CanonicalValue::Null,
                ));
                None
            }
            Some(value) => self.read_model_tuple(name, value, parse),
        }
    }

    /// Read one model sequence with a default of an empty sequence.
    pub fn default_model_tuple<T>(
        &mut self,
        name: &str,
        parse: impl Fn(&CanonicalValue, &[Location], Reading) -> Result<T, Failures>,
    ) -> Vec<T> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Vec::new(),
            Some(value) => self
                .read_model_tuple(name, value, parse)
                .unwrap_or_default(),
        }
    }

    /// Read one required model.
    pub fn required_model<T>(
        &mut self,
        name: &str,
        parse: impl Fn(&CanonicalValue, &[Location], Reading) -> Result<T, Failures>,
    ) -> Option<T> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => {
                let location = self.location(name);
                match parse(value, &location, self.reading) {
                    Ok(parsed) => Some(parsed),
                    Err(failures) => {
                        self.failures.extend(failures);
                        None
                    }
                }
            }
        }
    }

    /// Read one required model that may be null.
    pub fn required_nullable_model<T>(
        &mut self,
        name: &str,
        parse: impl Fn(&CanonicalValue, &[Location], Reading) -> Result<T, Failures>,
    ) -> Option<Option<T>> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(CanonicalValue::Null) => Some(None),
            Some(value) => {
                let location = self.location(name);
                match parse(value, &location, self.reading) {
                    Ok(parsed) => Some(Some(parsed)),
                    Err(failures) => {
                        self.failures.extend(failures);
                        None
                    }
                }
            }
        }
    }

    /// Read one optional model.
    pub fn optional_model<T>(
        &mut self,
        name: &str,
        parse: impl Fn(&CanonicalValue, &[Location], Reading) -> Result<T, Failures>,
    ) -> Option<Option<T>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => {
                let location = self.location(name);
                match parse(value, &location, self.reading) {
                    Ok(parsed) => Some(Some(parsed)),
                    Err(failures) => {
                        self.failures.extend(failures);
                        None
                    }
                }
            }
        }
    }

    /// Read one required mapping of free values.
    pub fn required_object(&mut self, name: &str) -> Option<CanonicalObject> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match value {
                CanonicalValue::Object(object) => Some(object.clone()),
                other => {
                    self.failures.push(Issue::new(
                        kind::DICT_TYPE,
                        self.location(name),
                        "Input should be a valid dictionary",
                        other,
                    ));
                    None
                }
            },
        }
    }

    /// Read one optional mapping of free values.
    pub fn optional_object(&mut self, name: &str) -> Option<Option<CanonicalObject>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match value {
                CanonicalValue::Object(object) => Some(Some(object.clone())),
                other => {
                    self.failures.push(Issue::new(
                        kind::DICT_TYPE,
                        self.location(name),
                        "Input should be a valid dictionary",
                        other,
                    ));
                    None
                }
            },
        }
    }

    /// Read one required timestamp field.
    pub fn required_timestamp(&mut self, name: &str) -> Option<crate::datetime::Timestamp> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(value) => match crate::datetime::read_timestamp(value, &self.location(name)) {
                Ok(timestamp) => Some(timestamp),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one required timestamp field that may be null.
    pub fn required_nullable_timestamp(
        &mut self,
        name: &str,
    ) -> Option<Option<crate::datetime::Timestamp>> {
        match self.raw_value(name) {
            None => {
                self.missing(name);
                None
            }
            Some(CanonicalValue::Null) => Some(None),
            Some(value) => match crate::datetime::read_timestamp(value, &self.location(name)) {
                Ok(timestamp) => Some(Some(timestamp)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read one optional timestamp field.
    pub fn optional_timestamp(&mut self, name: &str) -> Option<Option<crate::datetime::Timestamp>> {
        match self.raw_value(name) {
            None | Some(CanonicalValue::Null) => Some(None),
            Some(value) => match crate::datetime::read_timestamp(value, &self.location(name)) {
                Ok(timestamp) => Some(Some(timestamp)),
                Err(issue) => {
                    self.failures.push(*issue);
                    None
                }
            },
        }
    }

    /// Read a sequence of models.
    fn read_model_tuple<T>(
        &mut self,
        name: &str,
        value: &CanonicalValue,
        parse: impl Fn(&CanonicalValue, &[Location], Reading) -> Result<T, Failures>,
    ) -> Option<Vec<T>> {
        let location = self.location(name);
        let CanonicalValue::Array(items) = value else {
            self.failures.push(Issue::new(
                kind::TUPLE_TYPE,
                location,
                "Input should be a valid tuple",
                value,
            ));
            return None;
        };
        let mut parsed = Vec::with_capacity(items.len());
        let mut failures: Failures = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let mut item_location = location.clone();
            item_location.push(Location::Index(index));
            match parse(item, &item_location, self.reading) {
                Ok(value) => parsed.push(value),
                Err(mut issues) => {
                    failures.append(&mut issues);
                }
            }
        }
        if failures.is_empty() {
            Some(parsed)
        } else {
            self.failures.extend(failures);
            None
        }
    }

    /// Read a string, applying the strip configuration of the model.
    fn read_text(&self, name: &str, value: &CanonicalValue) -> Result<String, Box<Issue>> {
        let CanonicalValue::Text(text) = value else {
            return Err(Box::new(Issue::new(
                kind::STRING_TYPE,
                self.location(name),
                "Input should be a valid string",
                value,
            )));
        };
        let text = text.to_utf8().map_err(|_| {
            Box::new(Issue::new(
                kind::STRING_UNICODE,
                self.location(name),
                "Input should be a valid string, unable to parse raw data as a unicode string",
                value,
            ))
        })?;
        Ok(if self.strip {
            crate::pytext::trim(&text).to_owned()
        } else {
            text
        })
    }

    /// Check the bounds of a string.
    fn check_length(
        &self,
        name: &str,
        text: &str,
        value: &CanonicalValue,
        minimum: Option<usize>,
        maximum: Option<usize>,
    ) -> Result<(), Box<Issue>> {
        let length = text.chars().count();
        if let Some(minimum) = minimum {
            if length < minimum {
                return Err(Box::new(Issue::with_context(
                    kind::STRING_TOO_SHORT,
                    self.location(name),
                    format!("String should have at least {minimum} characters"),
                    value,
                    vec![("min_length".to_owned(), Context::Integer(minimum as i64))],
                )));
            }
        }
        if let Some(maximum) = maximum {
            if length > maximum {
                return Err(Box::new(Issue::with_context(
                    kind::STRING_TOO_LONG,
                    self.location(name),
                    format!("String should have at most {maximum} characters"),
                    value,
                    vec![("max_length".to_owned(), Context::Integer(maximum as i64))],
                )));
            }
        }
        Ok(())
    }

    /// Check the pattern of a string.
    fn check_pattern(
        &self,
        name: &str,
        text: &str,
        value: &CanonicalValue,
        pattern: &str,
    ) -> Result<(), Box<Issue>> {
        if !crate::patterns::matches(pattern, text) {
            return Err(Box::new(Issue::with_context(
                kind::STRING_PATTERN_MISMATCH,
                self.location(name),
                format!("String should match pattern '{pattern}'"),
                value,
                vec![("pattern".to_owned(), Context::Text(pattern.to_owned()))],
            )));
        }
        Ok(())
    }

    /// Read an enum value.
    fn read_enum(
        &self,
        name: &str,
        value: &CanonicalValue,
        accepted: &[&str],
    ) -> Result<String, Box<Issue>> {
        let expected = enum_expectation(accepted);
        let text = match value {
            CanonicalValue::Text(text) => text.to_utf8().ok(),
            _ => None,
        };
        match text {
            Some(text) if accepted.contains(&text.as_str()) => Ok(text),
            _ => Err(Box::new(Issue::with_context(
                kind::ENUM,
                self.location(name),
                format!("Input should be {expected}"),
                value,
                vec![("expected".to_owned(), Context::Text(expected))],
            ))),
        }
    }

    /// Read a sequence of strings.
    fn read_text_tuple(
        &self,
        name: &str,
        value: &CanonicalValue,
    ) -> Result<Vec<String>, Box<Issue>> {
        let CanonicalValue::Array(items) = value else {
            return Err(Box::new(Issue::new(
                kind::TUPLE_TYPE,
                self.location(name),
                "Input should be a valid tuple",
                value,
            )));
        };
        let mut parsed = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let mut location = self.location(name);
            location.push(Location::Index(index));
            let CanonicalValue::Text(text) = item else {
                return Err(Box::new(Issue::new(
                    kind::STRING_TYPE,
                    location,
                    "Input should be a valid string",
                    item,
                )));
            };
            let text = text.to_utf8().map_err(|_| {
                Box::new(Issue::new(
                    kind::STRING_UNICODE,
                    location,
                    "Input should be a valid string, unable to parse raw data as a unicode string",
                    item,
                ))
            })?;
            parsed.push(text);
        }
        Ok(parsed)
    }
}

/// The rendering Pydantic uses for the accepted values of an enum.
///
/// One value is quoted, two are quoted and joined with "or", and more than two
/// are quoted, joined with ", " and closed with "or".
#[must_use]
pub fn enum_expectation(accepted: &[&str]) -> String {
    let quoted: Vec<String> = accepted.iter().map(|value| format!("'{value}'")).collect();
    match quoted.len() {
        0 => String::new(),
        1 => quoted[0].clone(),
        2 => format!("{} or {}", quoted[0], quoted[1]),
        _ => {
            let (last, leading) = quoted.split_last().expect("a non empty list");
            format!("{} or {last}", leading.join(", "))
        }
    }
}

/// Read an integer value the way Pydantic reads it.
///
/// An integer is accepted as it is, a boolean is read as one and zero, a
/// number without a fractional part is read as its exact integer value, and a
/// string is read with the rules of `int()` in CPython: the surrounding
/// whitespace is ignored, the underscores between the digits are ignored, and
/// a decimal spelling without a fractional part is accepted.
pub fn read_integer(value: &CanonicalValue, location: &[Location]) -> Result<BigInt, Box<Issue>> {
    match value {
        CanonicalValue::Integer(number) => Ok(number.clone()),
        CanonicalValue::Bool(flag) => Ok(BigInt::from(i64::from(*flag))),
        CanonicalValue::Float(number) => {
            if number.is_finite() && number.fract() == 0.0 {
                Ok(BigInt::parse_bytes(format!("{number:.0}").as_bytes(), 10)
                    .expect("an integral number renders as digits"))
            } else {
                Err(Box::new(Issue::new(
                    kind::INT_FROM_FLOAT,
                    location.to_vec(),
                    "Input should be a valid integer, got a number with a fractional part",
                    value,
                )))
            }
        }
        CanonicalValue::Text(text) => {
            let text = text.to_utf8().unwrap_or_default();
            match parse_python_int(&text) {
                Some(number) => Ok(number),
                None => Err(Box::new(Issue::new(
                    kind::INT_PARSING,
                    location.to_vec(),
                    "Input should be a valid integer, unable to parse string as an integer",
                    value,
                ))),
            }
        }
        _ => Err(Box::new(Issue::new(
            kind::INT_TYPE,
            location.to_vec(),
            "Input should be a valid integer",
            value,
        ))),
    }
}

/// Read a number the way Pydantic reads it.
///
/// A number is accepted as it is, an integer is read as its binary64 value,
/// and a string is read with the rules of `float()` in CPython.
pub fn read_number(value: &CanonicalValue, location: &[Location]) -> Result<f64, Box<Issue>> {
    match value {
        CanonicalValue::Float(number) => Ok(*number),
        CanonicalValue::Integer(number) => {
            Ok(number.to_string().parse::<f64>().unwrap_or(f64::INFINITY))
        }
        CanonicalValue::Text(text) => {
            let text = text.to_utf8().unwrap_or_default();
            match parse_python_float(&text) {
                Some(number) => Ok(number),
                None => Err(Box::new(Issue::new(
                    kind::FLOAT_PARSING,
                    location.to_vec(),
                    "Input should be a valid number, unable to parse string as a number",
                    value,
                ))),
            }
        }
        _ => Err(Box::new(Issue::new(
            kind::FLOAT_TYPE,
            location.to_vec(),
            "Input should be a valid number",
            value,
        ))),
    }
}

/// Read a boolean the way Pydantic reads it.
///
/// A boolean is accepted as it is, zero and one are read as false and true,
/// and the accepted spellings are the ones `bool()` accepts in Python.
pub fn read_bool(value: &CanonicalValue, location: &[Location]) -> Result<bool, Box<Issue>> {
    match value {
        CanonicalValue::Bool(flag) => Ok(*flag),
        CanonicalValue::Integer(number) => {
            if number.to_string() == "0" {
                Ok(false)
            } else if number.to_string() == "1" {
                Ok(true)
            } else {
                Err(Box::new(bool_parsing(value, location)))
            }
        }
        CanonicalValue::Text(text) => {
            let text = text.to_utf8().unwrap_or_default();
            let lowered = text.trim().to_ascii_lowercase();
            match lowered.as_str() {
                "1" | "on" | "t" | "true" | "y" | "yes" => Ok(true),
                "0" | "off" | "f" | "false" | "n" | "no" => Ok(false),
                _ => Err(Box::new(bool_parsing(value, location))),
            }
        }
        _ => Err(Box::new(Issue::new(
            kind::BOOL_TYPE,
            location.to_vec(),
            "Input should be a valid boolean",
            value,
        ))),
    }
}

/// The failure of a value that cannot be read as a boolean.
fn bool_parsing(value: &CanonicalValue, location: &[Location]) -> Issue {
    Issue::new(
        kind::BOOL_PARSING,
        location.to_vec(),
        "Input should be a valid boolean, unable to interpret input",
        value,
    )
}

/// Parse a string the way CPython's `int()` parses one.
fn parse_python_int(text: &str) -> Option<BigInt> {
    let trimmed = crate::pytext::trim(text);
    if trimmed.is_empty() {
        return None;
    }
    let (sign, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let digits = remove_python_underscores(digits)?;
    if digits.is_empty() {
        return None;
    }
    if let Some(number) = BigInt::parse_bytes(digits.as_bytes(), 10) {
        return Some(if sign == "-" { -number } else { number });
    }
    // A decimal spelling without a fractional part is an integer too.
    let (whole, fraction) = digits.split_once('.')?;
    if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let whole = remove_python_underscores(whole)?;
    if whole.is_empty() {
        return None;
    }
    let number = BigInt::parse_bytes(whole.as_bytes(), 10)?;
    Some(if sign == "-" { -number } else { number })
}

/// Parse a string the way CPython's `float()` parses one.
fn parse_python_float(text: &str) -> Option<f64> {
    let trimmed = crate::pytext::trim(text);
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = remove_python_underscores(trimmed)?;
    if cleaned.is_empty() {
        return None;
    }
    match cleaned.as_str() {
        "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity" => {
            return Some(if cleaned.starts_with('-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        "nan" | "+nan" | "-nan" => return Some(f64::NAN),
        _ => {}
    }
    cleaned.parse::<f64>().ok()
}

/// Remove the underscores CPython accepts between the digits.
fn remove_python_underscores(text: &str) -> Option<String> {
    if !text.contains('_') {
        return Some(text.to_owned());
    }
    let characters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (index, character) in characters.iter().enumerate() {
        if *character == '_' {
            let before = index
                .checked_sub(1)
                .and_then(|position| characters.get(position));
            let after = characters.get(index + 1);
            match (before, after) {
                (Some(before), Some(after))
                    if before.is_ascii_digit() && after.is_ascii_digit() => {}
                _ => return None,
            }
            continue;
        }
        out.push(*character);
    }
    Some(out)
}

/// The failure of a value that is not the model it is declared as.
#[must_use]
pub fn model_type_issue(model: &str, location: &[Location], value: &CanonicalValue) -> Issue {
    Issue::with_context(
        kind::MODEL_TYPE,
        location.to_vec(),
        format!("Input should be a valid dictionary or instance of {model}"),
        value,
        vec![("class_name".to_owned(), Context::Text(model.to_owned()))],
    )
}

/// The failure of a field validator that refused the value.
///
/// The context carries the raised text. The Python source carries the raised
/// exception itself, which the API never renders: it answers a sanitized
/// envelope and not the raw failure.
#[must_use]
pub fn value_error_issue(location: &[Location], value: &CanonicalValue, reason: &str) -> Issue {
    Issue::with_context(
        kind::VALUE_ERROR,
        location.to_vec(),
        format!("Value error, {reason}"),
        value,
        vec![("error".to_owned(), Context::Text(reason.to_owned()))],
    )
}

/// The failure of a body that is not an object.
///
/// The body layer of the framework refuses it before the model reads it.
#[must_use]
pub fn body_type_issue(location: &[Location], value: &CanonicalValue) -> Issue {
    Issue::new(
        kind::MODEL_ATTRIBUTES_TYPE,
        location.to_vec(),
        "Input should be a valid dictionary or object to extract fields from",
        value,
    )
}

/// The failure of a body that is not there.
#[must_use]
pub fn body_missing_issue() -> Issue {
    Issue::new(
        kind::MISSING,
        Vec::new(),
        "Field required",
        &CanonicalValue::Null,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(name: &str) -> Vec<Location> {
        vec![Location::Field(name.to_owned())]
    }

    #[test]
    fn the_enum_expectation_matches_pydantic() {
        assert_eq!(enum_expectation(&["building"]), "'building'");
        assert_eq!(
            enum_expectation(&["building", "mechanical"]),
            "'building' or 'mechanical'"
        );
        assert_eq!(
            enum_expectation(&["building", "mechanical", "wing"]),
            "'building', 'mechanical' or 'wing'"
        );
    }

    #[test]
    fn integers_are_read_the_way_pydantic_reads_them() {
        let value = CanonicalValue::from(7i64);
        assert_eq!(
            read_integer(&value, &location("count")).unwrap(),
            BigInt::from(7)
        );
        let value = CanonicalValue::from(true);
        assert_eq!(
            read_integer(&value, &location("count")).unwrap(),
            BigInt::from(1)
        );
        let value = CanonicalValue::from(" 1_000 ");
        assert_eq!(
            read_integer(&value, &location("count")).unwrap(),
            BigInt::from(1000)
        );
        let value = CanonicalValue::from("7.0");
        assert_eq!(
            read_integer(&value, &location("count")).unwrap(),
            BigInt::from(7)
        );
        let value = CanonicalValue::from("1e3");
        let issue = read_integer(&value, &location("count")).unwrap_err();
        assert_eq!(issue.kind, kind::INT_PARSING);
        let value = CanonicalValue::Float(1.5);
        let issue = read_integer(&value, &location("count")).unwrap_err();
        assert_eq!(issue.kind, kind::INT_FROM_FLOAT);
    }

    #[test]
    fn booleans_are_read_the_way_pydantic_reads_them() {
        for (input, expected) in [
            ("true", true),
            ("TRUE", true),
            ("yes", true),
            ("on", true),
            ("1", true),
            ("false", false),
            ("no", false),
            ("off", false),
            ("0", false),
        ] {
            let value = CanonicalValue::from(input);
            assert_eq!(
                read_bool(&value, &location("flag")).unwrap(),
                expected,
                "{input}"
            );
        }
        let value = CanonicalValue::from("maybe");
        let issue = read_bool(&value, &location("flag")).unwrap_err();
        assert_eq!(issue.kind, kind::BOOL_PARSING);
    }
}
