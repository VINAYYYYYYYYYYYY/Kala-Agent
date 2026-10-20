//! Typed parameters for the wing domain package.

use std::sync::LazyLock;

use kala_contracts::WireValue;
use kala_json::CanonicalValue;

use crate::error::ParameterError;
use crate::input::{as_object, coerce_enum, coerce_float, coerce_int_in_range, get, reject_extra};
use crate::parameters::ParameterModel;
use crate::spec::{ParameterDefault, ParameterSpec, parameter_schema_digest};

/// Length unit.
pub const LENGTH_UNIT: &str = "m";
/// Area unit.
pub const AREA_UNIT: &str = "m^2";
/// Volume unit.
pub const VOLUME_UNIT: &str = "m^3";
/// Angle unit.
pub const ANGLE_UNIT: &str = "deg";
/// Ratio unit.
pub const RATIO_UNIT: &str = "1";
/// Station unit.
pub const STATION_UNIT: &str = "count";
/// Span-load unit.
pub const SPAN_LOAD_UNIT: &str = "kN/m";
/// Root-moment unit.
pub const ROOT_MOMENT_UNIT: &str = "kN*m";
/// Mass unit.
pub const MASS_UNIT: &str = "kg";
/// Per-radian unit.
pub const PER_RADIAN_UNIT: &str = "1/rad";

/// Non-certified finding.
pub const NON_CERTIFIED_FINDING: &str = "Geometric and aerodynamic surrogates are not certified analysis: they are not a lifting-line or panel solution, not CFD, and they establish no structural margin, stability, control, or airworthiness conclusion.";
/// Taper finding.
pub const TAPER_FINDING: &str = "Taper ratio and sweep are declared geometry only; no lifting surface method is applied to them.";

/// Declared load case names. Each is a surrogate, not a flight condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WingLoadCase {
    /// Reference surface.
    ReferenceSurface,
    /// Symmetric cruise.
    SymmetricCruise,
}

impl WingLoadCase {
    /// The declared string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReferenceSurface => "reference-surface",
            Self::SymmetricCruise => "symmetric-cruise",
        }
    }

    /// Parse an exact declared value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "reference-surface" => Some(Self::ReferenceSurface),
            "symmetric-cruise" => Some(Self::SymmetricCruise),
            _ => None,
        }
    }
}

/// Wing planform, load, and bound parameters with units and ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct WingParameters {
    span_m: f64,
    root_chord_m: f64,
    tip_chord_m: f64,
    sweep_deg: f64,
    dihedral_deg: f64,
    thickness_ratio: f64,
    spanwise_stations: i64,
    load_case: WingLoadCase,
    span_load_kn_m: f64,
    maximum_aspect_ratio: f64,
    maximum_root_bending_moment_kn_m: f64,
    maximum_mass_kg: f64,
}

impl WingParameters {
    /// Wing span.
    pub fn span_m(&self) -> f64 {
        self.span_m
    }
    /// Root chord.
    pub fn root_chord_m(&self) -> f64 {
        self.root_chord_m
    }
    /// Tip chord.
    pub fn tip_chord_m(&self) -> f64 {
        self.tip_chord_m
    }
    /// Quarter-chord sweep.
    pub fn sweep_deg(&self) -> f64 {
        self.sweep_deg
    }
    /// Dihedral.
    pub fn dihedral_deg(&self) -> f64 {
        self.dihedral_deg
    }
    /// Maximum thickness ratio.
    pub fn thickness_ratio(&self) -> f64 {
        self.thickness_ratio
    }
    /// Spanwise stations.
    pub fn spanwise_stations(&self) -> i64 {
        self.spanwise_stations
    }
    /// Declared surrogate load case.
    pub fn load_case(&self) -> WingLoadCase {
        self.load_case
    }
    /// Distributed span load.
    pub fn span_load_kn_m(&self) -> f64 {
        self.span_load_kn_m
    }
    /// Hard maximum aspect ratio.
    pub fn maximum_aspect_ratio(&self) -> f64 {
        self.maximum_aspect_ratio
    }
    /// Hard maximum root bending moment.
    pub fn maximum_root_bending_moment_kn_m(&self) -> f64 {
        self.maximum_root_bending_moment_kn_m
    }
    /// Hard maximum reference mass.
    pub fn maximum_mass_kg(&self) -> f64 {
        self.maximum_mass_kg
    }
}

impl Default for WingParameters {
    fn default() -> Self {
        Self::from_value(&CanonicalValue::Object(Default::default()))
            .expect("the declared defaults satisfy every invariant")
    }
}

impl ParameterModel for WingParameters {
    const MODEL_NAME: &'static str = "WingParameters";
    const DOMAIN_NAME: &'static str = "wing";
    const FIELD_NAMES: &'static [&'static str] = &[
        "span_m",
        "root_chord_m",
        "tip_chord_m",
        "sweep_deg",
        "dihedral_deg",
        "thickness_ratio",
        "spanwise_stations",
        "load_case",
        "span_load_kn_m",
        "maximum_aspect_ratio",
        "maximum_root_bending_moment_kn_m",
        "maximum_mass_kg",
    ];

    fn from_value(value: &CanonicalValue) -> Result<Self, ParameterError> {
        let object = as_object(value, Self::MODEL_NAME)?;
        reject_extra(object, Self::FIELD_NAMES)?;

        let span_m = match get(object, "span_m") {
            Some(value) => coerce_float("span_m", value, 1.0, 120.0)?,
            None => 12.0,
        };
        let root_chord_m = match get(object, "root_chord_m") {
            Some(value) => coerce_float("root_chord_m", value, 0.05, 12.0)?,
            None => 1.8,
        };
        let tip_chord_m = match get(object, "tip_chord_m") {
            Some(value) => coerce_float("tip_chord_m", value, 0.01, 12.0)?,
            None => 0.5,
        };
        let sweep_deg = match get(object, "sweep_deg") {
            Some(value) => coerce_float("sweep_deg", value, -10.0, 75.0)?,
            None => 22.0,
        };
        let dihedral_deg = match get(object, "dihedral_deg") {
            Some(value) => coerce_float("dihedral_deg", value, -20.0, 45.0)?,
            None => 5.0,
        };
        let thickness_ratio = match get(object, "thickness_ratio") {
            Some(value) => coerce_float("thickness_ratio", value, 0.03, 0.25)?,
            None => 0.12,
        };
        let spanwise_stations = match get(object, "spanwise_stations") {
            Some(value) => coerce_int_in_range("spanwise_stations", value, 2, 64)?,
            None => 8,
        };
        let load_case = match get(object, "load_case") {
            Some(value) => coerce_enum(
                "load_case",
                value,
                &[
                    ("reference-surface", WingLoadCase::ReferenceSurface),
                    ("symmetric-cruise", WingLoadCase::SymmetricCruise),
                ],
            )?,
            None => WingLoadCase::ReferenceSurface,
        };
        let span_load_kn_m = match get(object, "span_load_kn_m") {
            Some(value) => coerce_float("span_load_kn_m", value, 0.0, 5_000.0)?,
            None => 4.0,
        };
        let maximum_aspect_ratio = match get(object, "maximum_aspect_ratio") {
            Some(value) => coerce_float("maximum_aspect_ratio", value, 0.5, 60.0)?,
            None => 20.0,
        };
        let maximum_root_bending_moment_kn_m = match get(object, "maximum_root_bending_moment_kn_m")
        {
            Some(value) => coerce_float("maximum_root_bending_moment_kn_m", value, 1.0, 5_000.0)?,
            None => 150.0,
        };
        let maximum_mass_kg = match get(object, "maximum_mass_kg") {
            Some(value) => coerce_float("maximum_mass_kg", value, 5.0, 5_000.0)?,
            None => 400.0,
        };

        let parameters = Self {
            span_m,
            root_chord_m,
            tip_chord_m,
            sweep_deg,
            dihedral_deg,
            thickness_ratio,
            spanwise_stations,
            load_case,
            span_load_kn_m,
            maximum_aspect_ratio,
            maximum_root_bending_moment_kn_m,
            maximum_mass_kg,
        };
        parameters.validate_planform()?;
        Ok(parameters)
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("span_m", WireValue::Float(self.span_m)),
            ("root_chord_m", WireValue::Float(self.root_chord_m)),
            ("tip_chord_m", WireValue::Float(self.tip_chord_m)),
            ("sweep_deg", WireValue::Float(self.sweep_deg)),
            ("dihedral_deg", WireValue::Float(self.dihedral_deg)),
            ("thickness_ratio", WireValue::Float(self.thickness_ratio)),
            (
                "spanwise_stations",
                WireValue::Integer(self.spanwise_stations.into()),
            ),
            ("load_case", WireValue::Text(self.load_case.as_str().into())),
            ("span_load_kn_m", WireValue::Float(self.span_load_kn_m)),
            (
                "maximum_aspect_ratio",
                WireValue::Float(self.maximum_aspect_ratio),
            ),
            (
                "maximum_root_bending_moment_kn_m",
                WireValue::Float(self.maximum_root_bending_moment_kn_m),
            ),
            ("maximum_mass_kg", WireValue::Float(self.maximum_mass_kg)),
        ])
    }

    fn schema_digest() -> String {
        WING_SCHEMA_DIGEST.clone()
    }
}

impl WingParameters {
    fn validate_planform(&self) -> Result<(), ParameterError> {
        if self.tip_chord_m > self.root_chord_m {
            return Err(ParameterError::model(
                "tip_chord_m must not exceed root_chord_m",
            ));
        }
        if self.span_m < self.root_chord_m {
            return Err(ParameterError::model(
                "span_m must be at least the root chord",
            ));
        }
        Ok(())
    }
}

/// Declared parameter specifications.
pub static PARAMETER_SPECS: LazyLock<Vec<ParameterSpec>> = LazyLock::new(|| {
    vec![
        ParameterSpec::new(
            "span_m",
            LENGTH_UNIT,
            1.0,
            120.0,
            ParameterDefault::Float(12.0),
            "Wing span in metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "root_chord_m",
            LENGTH_UNIT,
            0.05,
            12.0,
            ParameterDefault::Float(1.8),
            "Root chord in metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "tip_chord_m",
            LENGTH_UNIT,
            0.01,
            12.0,
            ParameterDefault::Float(0.5),
            "Tip chord in metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "sweep_deg",
            ANGLE_UNIT,
            -10.0,
            75.0,
            ParameterDefault::Float(22.0),
            "Declared quarter-chord sweep in degrees.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "dihedral_deg",
            ANGLE_UNIT,
            -20.0,
            45.0,
            ParameterDefault::Float(5.0),
            "Declared dihedral in degrees.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "thickness_ratio",
            RATIO_UNIT,
            0.03,
            0.25,
            ParameterDefault::Float(0.12),
            "Declared maximum thickness ratio.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "spanwise_stations",
            STATION_UNIT,
            2.0,
            64.0,
            ParameterDefault::Float(8.0),
            "Spanwise stations used by the reference integration.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "load_case",
            "key",
            0.0,
            0.0,
            ParameterDefault::Text("reference-surface".to_owned()),
            "Declared surrogate load case name.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "span_load_kn_m",
            SPAN_LOAD_UNIT,
            0.0,
            5_000.0,
            ParameterDefault::Float(4.0),
            "Distributed span load in kilonewtons per metre.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "maximum_aspect_ratio",
            RATIO_UNIT,
            0.5,
            60.0,
            ParameterDefault::Float(20.0),
            "Hard maximum aspect ratio.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "maximum_root_bending_moment_kn_m",
            ROOT_MOMENT_UNIT,
            1.0,
            5_000.0,
            ParameterDefault::Float(150.0),
            "Hard maximum root bending moment in kilonewton metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "maximum_mass_kg",
            MASS_UNIT,
            5.0,
            5_000.0,
            ParameterDefault::Float(400.0),
            "Hard maximum reference mass in kilograms.",
        )
        .expect("declared specification"),
    ]
});

/// Parameter-schema digest.
pub static WING_SCHEMA_DIGEST: LazyLock<String> = LazyLock::new(|| {
    parameter_schema_digest(&PARAMETER_SPECS).expect("the declared schema is canonical")
});
