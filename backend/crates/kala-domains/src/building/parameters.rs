//! Typed parameters for the building domain package.

use std::sync::LazyLock;

use kala_contracts::WireValue;
use kala_json::CanonicalValue;

use crate::error::ParameterError;
use crate::input::{as_object, coerce_enum, coerce_float, coerce_int_in_range, get, reject_extra};
use crate::parameters::ParameterModel;
use crate::spec::{ParameterDefault, ParameterSpec, parameter_schema_digest};

/// Metre unit.
pub const METRE_UNIT: &str = "m";
/// Square metre unit.
pub const SQUARE_METRE_UNIT: &str = "m^2";
/// Cubic metre unit.
pub const CUBIC_METRE_UNIT: &str = "m^3";
/// Conductivity unit.
pub const CONDUCTIVITY_UNIT: &str = "W/(m^2*K)";
/// Transmittance unit.
pub const TRANSMITTANCE_UNIT: &str = "W/K";
/// Temperature unit.
pub const TEMPERATURE_UNIT: &str = "degC";
/// Degree-day unit.
pub const DEGREE_DAY_UNIT: &str = "K.day";
/// Energy unit.
pub const ENERGY_UNIT: &str = "kWh";
/// Intensity unit.
pub const INTENSITY_UNIT: &str = "kWh/m^2";
/// Ratio unit.
pub const RATIO_UNIT: &str = "1";

/// Non-authoritative code version.
pub const NON_AUTHORITATIVE_CODE_VERSION: &str = "non-authoritative-placeholder";
/// Code authority finding.
pub const CODE_AUTHORITY_FINDING: &str = "Code version 'non-authoritative-placeholder' is a declared placeholder; this package determines no building code compliance, no occupancy classification, and no safety approval.";
/// Thermal surrogate finding.
pub const THERMAL_SURROGATE_FINDING: &str = "Transmission-only degree-day surrogate; it excludes solar gain, internal load, ventilation, and thermal mass, so it is not a code or energy-model calculation.";

/// Declared code references. Every value is a non-authoritative placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildingCodeVersion {
    /// The only declared placeholder.
    NonAuthoritativePlaceholder,
}

impl BuildingCodeVersion {
    /// The declared string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonAuthoritativePlaceholder => NON_AUTHORITATIVE_CODE_VERSION,
        }
    }

    /// Parse an exact declared value.
    pub fn parse(value: &str) -> Option<Self> {
        (value == NON_AUTHORITATIVE_CODE_VERSION).then_some(Self::NonAuthoritativePlaceholder)
    }
}

/// Storey, area, and thermal envelope parameters with units and ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingParameters {
    storey_count: i64,
    storey_height_m: f64,
    gross_floor_area_m2: f64,
    perimeter_m: f64,
    wall_conductivity_w_m2k: f64,
    window_conductivity_w_m2k: f64,
    roof_conductivity_w_m2k: f64,
    window_to_wall_ratio: f64,
    heating_degree_days_k: f64,
    design_indoor_temperature_c: f64,
    design_outdoor_temperature_c: f64,
    minimum_storey_area_m2: f64,
    maximum_height_m: f64,
    maximum_heating_intensity_kwh_m2: f64,
    code_version: BuildingCodeVersion,
}

impl BuildingParameters {
    /// Number of storeys.
    pub fn storey_count(&self) -> i64 {
        self.storey_count
    }
    /// Height of one storey.
    pub fn storey_height_m(&self) -> f64 {
        self.storey_height_m
    }
    /// Gross floor area of one storey.
    pub fn gross_floor_area_m2(&self) -> f64 {
        self.gross_floor_area_m2
    }
    /// Plate perimeter.
    pub fn perimeter_m(&self) -> f64 {
        self.perimeter_m
    }
    /// Opaque wall transmittance.
    pub fn wall_conductivity_w_m2k(&self) -> f64 {
        self.wall_conductivity_w_m2k
    }
    /// Window transmittance.
    pub fn window_conductivity_w_m2k(&self) -> f64 {
        self.window_conductivity_w_m2k
    }
    /// Roof transmittance.
    pub fn roof_conductivity_w_m2k(&self) -> f64 {
        self.roof_conductivity_w_m2k
    }
    /// Glazed fraction of the wall area.
    pub fn window_to_wall_ratio(&self) -> f64 {
        self.window_to_wall_ratio
    }
    /// Heating degree days.
    pub fn heating_degree_days_k(&self) -> f64 {
        self.heating_degree_days_k
    }
    /// Design indoor temperature.
    pub fn design_indoor_temperature_c(&self) -> f64 {
        self.design_indoor_temperature_c
    }
    /// Design outdoor temperature.
    pub fn design_outdoor_temperature_c(&self) -> f64 {
        self.design_outdoor_temperature_c
    }
    /// Placeholder minimum area per storey.
    pub fn minimum_storey_area_m2(&self) -> f64 {
        self.minimum_storey_area_m2
    }
    /// Placeholder maximum total height.
    pub fn maximum_height_m(&self) -> f64 {
        self.maximum_height_m
    }
    /// Placeholder maximum heating intensity.
    pub fn maximum_heating_intensity_kwh_m2(&self) -> f64 {
        self.maximum_heating_intensity_kwh_m2
    }
    /// Declared code reference.
    pub fn code_version(&self) -> BuildingCodeVersion {
        self.code_version
    }
}

impl Default for BuildingParameters {
    fn default() -> Self {
        Self::from_value(&CanonicalValue::Object(Default::default()))
            .expect("the declared defaults satisfy every invariant")
    }
}

impl ParameterModel for BuildingParameters {
    const MODEL_NAME: &'static str = "BuildingParameters";
    const DOMAIN_NAME: &'static str = "building";
    const FIELD_NAMES: &'static [&'static str] = &[
        "storey_count",
        "storey_height_m",
        "gross_floor_area_m2",
        "perimeter_m",
        "wall_conductivity_w_m2k",
        "window_conductivity_w_m2k",
        "roof_conductivity_w_m2k",
        "window_to_wall_ratio",
        "heating_degree_days_k",
        "design_indoor_temperature_c",
        "design_outdoor_temperature_c",
        "minimum_storey_area_m2",
        "maximum_height_m",
        "maximum_heating_intensity_kwh_m2",
        "code_version",
    ];

    fn from_value(value: &CanonicalValue) -> Result<Self, ParameterError> {
        let object = as_object(value, Self::MODEL_NAME)?;
        reject_extra(object, Self::FIELD_NAMES)?;

        let storey_count = match get(object, "storey_count") {
            Some(value) => coerce_int_in_range("storey_count", value, 1, 200)?,
            None => 4,
        };
        let storey_height_m = match get(object, "storey_height_m") {
            Some(value) => coerce_float("storey_height_m", value, 2.0, 6.0)?,
            None => 3.0,
        };
        let gross_floor_area_m2 = match get(object, "gross_floor_area_m2") {
            Some(value) => coerce_float("gross_floor_area_m2", value, 10.0, 100_000.0)?,
            None => 250.0,
        };
        let perimeter_m = match get(object, "perimeter_m") {
            Some(value) => coerce_float("perimeter_m", value, 5.0, 500.0)?,
            None => 32.0,
        };
        let wall_conductivity_w_m2k = match get(object, "wall_conductivity_w_m2k") {
            Some(value) => coerce_float("wall_conductivity_w_m2k", value, 0.0, 5.0)?,
            None => 0.30,
        };
        let window_conductivity_w_m2k = match get(object, "window_conductivity_w_m2k") {
            Some(value) => coerce_float("window_conductivity_w_m2k", value, 0.0, 6.0)?,
            None => 1.6,
        };
        let roof_conductivity_w_m2k = match get(object, "roof_conductivity_w_m2k") {
            Some(value) => coerce_float("roof_conductivity_w_m2k", value, 0.0, 5.0)?,
            None => 0.20,
        };
        let window_to_wall_ratio = match get(object, "window_to_wall_ratio") {
            Some(value) => coerce_float("window_to_wall_ratio", value, 0.0, 0.9)?,
            None => 0.30,
        };
        let heating_degree_days_k = match get(object, "heating_degree_days_k") {
            Some(value) => coerce_float("heating_degree_days_k", value, 1.0, 40_000.0)?,
            None => 2_500.0,
        };
        let design_indoor_temperature_c = match get(object, "design_indoor_temperature_c") {
            Some(value) => coerce_float("design_indoor_temperature_c", value, 10.0, 30.0)?,
            None => 20.0,
        };
        let design_outdoor_temperature_c = match get(object, "design_outdoor_temperature_c") {
            Some(value) => coerce_float("design_outdoor_temperature_c", value, -40.0, 20.0)?,
            None => -8.0,
        };
        let minimum_storey_area_m2 = match get(object, "minimum_storey_area_m2") {
            Some(value) => coerce_float("minimum_storey_area_m2", value, 1.0, 1_000.0)?,
            None => 20.0,
        };
        let maximum_height_m = match get(object, "maximum_height_m") {
            Some(value) => coerce_float("maximum_height_m", value, 10.0, 400.0)?,
            None => 200.0,
        };
        let maximum_heating_intensity_kwh_m2 = match get(object, "maximum_heating_intensity_kwh_m2")
        {
            Some(value) => coerce_float("maximum_heating_intensity_kwh_m2", value, 10.0, 2_000.0)?,
            None => 600.0,
        };
        let code_version = match get(object, "code_version") {
            Some(value) => coerce_enum(
                "code_version",
                value,
                &[(
                    NON_AUTHORITATIVE_CODE_VERSION,
                    BuildingCodeVersion::NonAuthoritativePlaceholder,
                )],
            )?,
            None => BuildingCodeVersion::NonAuthoritativePlaceholder,
        };

        let parameters = Self {
            storey_count,
            storey_height_m,
            gross_floor_area_m2,
            perimeter_m,
            wall_conductivity_w_m2k,
            window_conductivity_w_m2k,
            roof_conductivity_w_m2k,
            window_to_wall_ratio,
            heating_degree_days_k,
            design_indoor_temperature_c,
            design_outdoor_temperature_c,
            minimum_storey_area_m2,
            maximum_height_m,
            maximum_heating_intensity_kwh_m2,
            code_version,
        };
        parameters.validate_heating_case()?;
        Ok(parameters)
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("storey_count", WireValue::Integer(self.storey_count.into())),
            ("storey_height_m", WireValue::Float(self.storey_height_m)),
            (
                "gross_floor_area_m2",
                WireValue::Float(self.gross_floor_area_m2),
            ),
            ("perimeter_m", WireValue::Float(self.perimeter_m)),
            (
                "wall_conductivity_w_m2k",
                WireValue::Float(self.wall_conductivity_w_m2k),
            ),
            (
                "window_conductivity_w_m2k",
                WireValue::Float(self.window_conductivity_w_m2k),
            ),
            (
                "roof_conductivity_w_m2k",
                WireValue::Float(self.roof_conductivity_w_m2k),
            ),
            (
                "window_to_wall_ratio",
                WireValue::Float(self.window_to_wall_ratio),
            ),
            (
                "heating_degree_days_k",
                WireValue::Float(self.heating_degree_days_k),
            ),
            (
                "design_indoor_temperature_c",
                WireValue::Float(self.design_indoor_temperature_c),
            ),
            (
                "design_outdoor_temperature_c",
                WireValue::Float(self.design_outdoor_temperature_c),
            ),
            (
                "minimum_storey_area_m2",
                WireValue::Float(self.minimum_storey_area_m2),
            ),
            ("maximum_height_m", WireValue::Float(self.maximum_height_m)),
            (
                "maximum_heating_intensity_kwh_m2",
                WireValue::Float(self.maximum_heating_intensity_kwh_m2),
            ),
            (
                "code_version",
                WireValue::Text(self.code_version.as_str().into()),
            ),
        ])
    }

    fn schema_digest() -> String {
        BUILDING_SCHEMA_DIGEST.clone()
    }
}

impl BuildingParameters {
    fn validate_heating_case(&self) -> Result<(), ParameterError> {
        if self.design_outdoor_temperature_c >= self.design_indoor_temperature_c {
            return Err(ParameterError::model(
                "the design outdoor temperature must be below the indoor one",
            ));
        }
        if !(0.0..=0.9).contains(&self.window_to_wall_ratio) {
            return Err(ParameterError::model(
                "the glazed fraction must be within the declared range",
            ));
        }
        Ok(())
    }
}

/// Declared parameter specifications.
pub static PARAMETER_SPECS: LazyLock<Vec<ParameterSpec>> = LazyLock::new(|| {
    vec![
        ParameterSpec::new(
            "storey_count",
            RATIO_UNIT,
            1.0,
            200.0,
            ParameterDefault::Float(4.0),
            "Number of storeys in the stack.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "storey_height_m",
            METRE_UNIT,
            2.0,
            6.0,
            ParameterDefault::Float(3.0),
            "Height of one storey in metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "gross_floor_area_m2",
            SQUARE_METRE_UNIT,
            10.0,
            100_000.0,
            ParameterDefault::Float(250.0),
            "Gross floor area of one storey in square metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "perimeter_m",
            METRE_UNIT,
            5.0,
            500.0,
            ParameterDefault::Float(32.0),
            "Plate perimeter in metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "wall_conductivity_w_m2k",
            CONDUCTIVITY_UNIT,
            0.0,
            5.0,
            ParameterDefault::Float(0.30),
            "Opaque wall transmittance.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "window_conductivity_w_m2k",
            CONDUCTIVITY_UNIT,
            0.0,
            6.0,
            ParameterDefault::Float(1.6),
            "Window transmittance.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "roof_conductivity_w_m2k",
            CONDUCTIVITY_UNIT,
            0.0,
            5.0,
            ParameterDefault::Float(0.20),
            "Roof transmittance.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "window_to_wall_ratio",
            RATIO_UNIT,
            0.0,
            0.9,
            ParameterDefault::Float(0.30),
            "Glazed fraction of the wall area.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "heating_degree_days_k",
            DEGREE_DAY_UNIT,
            1.0,
            40_000.0,
            ParameterDefault::Float(2_500.0),
            "Heating degree days for the transmission surrogate.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "design_indoor_temperature_c",
            TEMPERATURE_UNIT,
            10.0,
            30.0,
            ParameterDefault::Float(20.0),
            "Design indoor temperature in degrees Celsius.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "design_outdoor_temperature_c",
            TEMPERATURE_UNIT,
            -40.0,
            20.0,
            ParameterDefault::Float(-8.0),
            "Design outdoor temperature in degrees Celsius.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "minimum_storey_area_m2",
            SQUARE_METRE_UNIT,
            1.0,
            1_000.0,
            ParameterDefault::Float(20.0),
            "Placeholder minimum storey area in square metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "maximum_height_m",
            METRE_UNIT,
            10.0,
            400.0,
            ParameterDefault::Float(200.0),
            "Placeholder maximum total height in metres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "maximum_heating_intensity_kwh_m2",
            INTENSITY_UNIT,
            10.0,
            2_000.0,
            ParameterDefault::Float(600.0),
            "Placeholder maximum heating intensity in kilowatt hours per square metre.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "code_version",
            "key",
            0.0,
            0.0,
            ParameterDefault::Text(NON_AUTHORITATIVE_CODE_VERSION.to_owned()),
            "Declared code reference; only a non-authoritative placeholder exists.",
        )
        .expect("declared specification"),
    ]
});

/// Parameter-schema digest.
pub static BUILDING_SCHEMA_DIGEST: LazyLock<String> = LazyLock::new(|| {
    parameter_schema_digest(&PARAMETER_SPECS).expect("the declared schema is canonical")
});
