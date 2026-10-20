//! The simulator parameters and the closed form reference calculation.
//!
//! The Python module parses the arguments of a call into a
//! `SimulatorParameters` record and computes the metrics of a prismatic bar
//! from it. Both are deterministic closed form arithmetic: the module is an
//! arithmetic reference, not an engineering validation.

use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use num_traits::ToPrimitive;

use crate::simulator::error::SimulatorError;

/// The operations the simulator supports.
pub const OPERATIONS: [&str; 2] = ["beam_volume", "bar_summary"];

/// The number of decimals the reported metrics are rounded to.
const ROUNDING_DIGITS: usize = 6;

/// The largest reported volume, in cubic millimetres.
pub const MAX_REPORTED_VOLUME_MM3: f64 = 1_250_000_000.0;

/// The largest reported mass, in kilogrammes.
pub const MAX_REPORTED_MASS_KG: f64 = 27_000.0;

/// The largest reported slenderness ratio.
pub const MAX_REPORTED_SLENDERNESS: f64 = 5_000.0;

/// The dimension names, their defaults and their limits.
///
/// The order is the order of the Python mapping: the first failure is
/// reported for the first name in this list.
const DIMENSIONS: [(&str, f64, f64, f64); 4] = [
    ("density_kg_m3", 2_700.0, 100.0, 20_000.0),
    ("height_mm", 10.0, 1.0, 500.0),
    ("length_mm", 120.0, 10.0, 5_000.0),
    ("width_mm", 20.0, 1.0, 500.0),
];

/// The parsed arguments of a simulator call.
#[derive(Clone, Debug, PartialEq)]
pub struct SimulatorParameters {
    operation: String,
    length_mm: f64,
    width_mm: f64,
    height_mm: f64,
    density_kg_m3: f64,
    write_report: bool,
}

impl SimulatorParameters {
    /// Build the parameters of a call.
    ///
    /// The Python implementation exposes the same constructor through its
    /// dataclass. The bounds are enforced by [`parse_parameters`], not here.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation: impl Into<String>,
        length_mm: f64,
        width_mm: f64,
        height_mm: f64,
        density_kg_m3: f64,
        write_report: bool,
    ) -> Self {
        Self {
            operation: operation.into(),
            length_mm,
            width_mm,
            height_mm,
            density_kg_m3,
            write_report,
        }
    }

    /// The requested operation.
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// The length of the bar, in millimetres.
    pub fn length_mm(&self) -> f64 {
        self.length_mm
    }

    /// The width of the bar, in millimetres.
    pub fn width_mm(&self) -> f64 {
        self.width_mm
    }

    /// The height of the bar, in millimetres.
    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }

    /// The density of the material, in kilogrammes per cubic metre.
    pub fn density_kg_m3(&self) -> f64 {
        self.density_kg_m3
    }

    /// Whether the call asked for a report to be written.
    pub fn write_report(&self) -> bool {
        self.write_report
    }

    /// The canonical payload of the parameters.
    pub fn to_payload(&self) -> CanonicalValue {
        let mut payload = CanonicalObject::new();
        payload.insert(
            JsonText::new("density_kg_m3"),
            CanonicalValue::Float(self.density_kg_m3),
        );
        payload.insert(
            JsonText::new("height_mm"),
            CanonicalValue::Float(self.height_mm),
        );
        payload.insert(
            JsonText::new("length_mm"),
            CanonicalValue::Float(self.length_mm),
        );
        payload.insert(
            JsonText::new("operation"),
            CanonicalValue::text(&self.operation),
        );
        payload.insert(
            JsonText::new("width_mm"),
            CanonicalValue::Float(self.width_mm),
        );
        payload.insert(
            JsonText::new("write_report"),
            CanonicalValue::Bool(self.write_report),
        );
        CanonicalValue::Object(payload)
    }
}

/// Parse the arguments of a simulator call.
///
/// The value must be a JSON object. Every dimension is optional and falls
/// back to its default; a dimension that is supplied must be a finite number
/// inside its bounds.
pub fn parse_parameters(arguments: &CanonicalValue) -> Result<SimulatorParameters, SimulatorError> {
    let CanonicalValue::Object(arguments) = arguments else {
        return Err(SimulatorError::input("arguments must be a JSON object"));
    };
    let operation = match arguments.get(&JsonText::new("operation")) {
        None => "beam_volume".to_owned(),
        Some(CanonicalValue::Text(text)) => text.to_string_lossy(),
        Some(_) => {
            return Err(SimulatorError::input(format!(
                "operation must be one of {}",
                OPERATIONS.join(", ")
            )));
        }
    };
    if !OPERATIONS.contains(&operation.as_str()) {
        return Err(SimulatorError::input(format!(
            "operation must be one of {}",
            OPERATIONS.join(", ")
        )));
    }
    let mut values: Vec<(&str, f64)> = Vec::with_capacity(DIMENSIONS.len());
    for (name, default, low, high) in DIMENSIONS {
        let supplied = match arguments.get(&JsonText::new(name)) {
            None => default,
            Some(CanonicalValue::Integer(number)) => number
                .to_f64()
                .ok_or_else(|| SimulatorError::overflow("int too large to convert to float"))?,
            Some(CanonicalValue::Float(number)) => *number,
            Some(_) => return Err(SimulatorError::input(format!("{name} must be a number"))),
        };
        if !supplied.is_finite() {
            return Err(SimulatorError::input(format!("{name} must be finite")));
        }
        if !(low..=high).contains(&supplied) {
            return Err(SimulatorError::input(format!(
                "{name} must be between {low:?} and {high:?}"
            )));
        }
        values.push((name, supplied));
    }
    let lookup = |name: &str| {
        values
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, value)| *value)
            .unwrap_or_default()
    };
    let write_report = match arguments.get(&JsonText::new("write_report")) {
        None => false,
        Some(CanonicalValue::Bool(value)) => *value,
        Some(_) => return Err(SimulatorError::input("write_report must be a boolean")),
    };
    Ok(SimulatorParameters {
        operation,
        length_mm: lookup("length_mm"),
        width_mm: lookup("width_mm"),
        height_mm: lookup("height_mm"),
        density_kg_m3: lookup("density_kg_m3"),
        write_report,
    })
}

/// Compute the metrics of a prismatic bar.
///
/// The calculation is the closed form reference of the Python module: the
/// volume, the mass, the second moment of area, the surface area and the
/// slenderness ratio, each rounded to six decimals.
pub fn compute_metrics(parameters: &SimulatorParameters) -> CanonicalValue {
    let volume = parameters.length_mm * parameters.width_mm * parameters.height_mm;
    let mass = volume * parameters.density_kg_m3 * 1e-9;
    let second_moment =
        (parameters.width_mm * parameters.height_mm.powf(3.0) / 12.0) * parameters.length_mm;
    let surface_area = 2.0
        * (parameters.length_mm * parameters.width_mm
            + parameters.length_mm * parameters.height_mm
            + parameters.width_mm * parameters.height_mm);
    let slenderness = parameters.length_mm / parameters.height_mm.max(1.0);
    let mut metrics = CanonicalObject::new();
    metrics.insert(JsonText::new("mass_kg"), CanonicalValue::Float(round(mass)));
    metrics.insert(
        JsonText::new("second_moment_mm4"),
        CanonicalValue::Float(round(second_moment)),
    );
    metrics.insert(
        JsonText::new("slenderness_ratio"),
        CanonicalValue::Float(round(slenderness)),
    );
    metrics.insert(
        JsonText::new("surface_area_mm2"),
        CanonicalValue::Float(round(surface_area)),
    );
    metrics.insert(
        JsonText::new("volume_mm3"),
        CanonicalValue::Float(round(volume)),
    );
    metrics.insert(
        JsonText::new("within_limits"),
        CanonicalValue::Bool(within_limits(volume, mass, slenderness)),
    );
    CanonicalValue::Object(metrics)
}

/// Whether the reported metrics stay inside the reported limits.
pub fn within_limits(volume_mm3: f64, mass_kg: f64, slenderness_ratio: f64) -> bool {
    volume_mm3 <= MAX_REPORTED_VOLUME_MM3
        && mass_kg <= MAX_REPORTED_MASS_KG
        && slenderness_ratio <= MAX_REPORTED_SLENDERNESS
}

/// The canonical payload of a report.
///
/// The report is self describing: it names the tool, the call, the inputs
/// and the metrics, and it carries the disclaimer of the module.
pub fn report_payload(
    parameters: &SimulatorParameters,
    metrics: &CanonicalValue,
    call_id: &str,
) -> CanonicalValue {
    let mut payload = CanonicalObject::new();
    payload.insert(JsonText::new("call_id"), CanonicalValue::text(call_id));
    payload.insert(
        JsonText::new("disclaimer"),
        CanonicalValue::text("closed-form arithmetic reference; not an engineering validation"),
    );
    payload.insert(JsonText::new("inputs"), parameters.to_payload());
    payload.insert(JsonText::new("metrics"), metrics.clone());
    payload.insert(
        JsonText::new("tool_id"),
        CanonicalValue::text(crate::simulator::SIMULATOR_TOOL_ID),
    );
    payload.insert(
        JsonText::new("tool_version"),
        CanonicalValue::text(crate::simulator::SIMULATOR_TOOL_VERSION),
    );
    CanonicalValue::Object(payload)
}

/// Round a value to six decimals, exactly like CPython's `round(value, 6)`.
///
/// CPython rounds the exact value of the double to the nearest multiple of
/// `10**-6`, with a tie going to the even choice, and returns the double
/// that is nearest to that decimal. The Rust formatter rounds the exact value
/// of the double to the same number of decimals with the same rule, so the
/// decimal text is identical and the parsed double is identical.
fn round(value: f64) -> f64 {
    let text = format!("{value:.precision$}", precision = ROUNDING_DIGITS);
    text.parse().unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters() -> SimulatorParameters {
        parse_parameters(&CanonicalValue::Object(CanonicalObject::new())).expect("parameters")
    }

    #[test]
    fn matches_the_closed_form_reference() {
        let metrics = compute_metrics(&parameters());
        let CanonicalValue::Object(metrics) = &metrics else {
            panic!("metrics are an object");
        };
        assert_eq!(
            metrics.get(&JsonText::new("volume_mm3")),
            Some(&CanonicalValue::Float(24_000.0))
        );
        assert_eq!(
            metrics.get(&JsonText::new("mass_kg")),
            Some(&CanonicalValue::Float(0.0648))
        );
        assert_eq!(
            metrics.get(&JsonText::new("surface_area_mm2")),
            Some(&CanonicalValue::Float(7_600.0))
        );
        assert_eq!(
            metrics.get(&JsonText::new("slenderness_ratio")),
            Some(&CanonicalValue::Float(12.0))
        );
        assert_eq!(
            metrics.get(&JsonText::new("second_moment_mm4")),
            Some(&CanonicalValue::Float(200_000.0))
        );
        assert_eq!(
            metrics.get(&JsonText::new("within_limits")),
            Some(&CanonicalValue::Bool(true))
        );
    }

    #[test]
    fn floors_the_slenderness_divisor() {
        let parameters = SimulatorParameters::new("beam_volume", 120.0, 20.0, 0.0, 2_700.0, false);
        let metrics = compute_metrics(&parameters);
        let CanonicalValue::Object(ref metrics) = metrics else {
            panic!("metrics are an object");
        };
        assert_eq!(
            metrics.get(&JsonText::new("slenderness_ratio")),
            Some(&CanonicalValue::Float(120.0))
        );
    }

    #[test]
    fn rounds_a_tie_to_the_even_choice() {
        // 0.0078125 is exactly representable and has seven decimals, so the
        // seventh decimal is a tie. CPython rounds it to the even choice.
        assert_eq!(round(0.0078125), 0.007812);
        assert_eq!(round(0.0234375), 0.023438);
    }
}
