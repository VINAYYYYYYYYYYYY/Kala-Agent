//! Typed parameters for the mechanical domain package.

use std::sync::LazyLock;

use kala_contracts::WireValue;
use kala_json::CanonicalValue;

use crate::error::{DomainError, ParameterError};
use crate::input::{
    as_object, coerce_enum, coerce_float, coerce_int_in_range, get, python_string_repr,
    reject_extra,
};
use crate::parameters::ParameterModel;
use crate::spec::{ParameterDefault, ParameterSpec, parameter_schema_digest};

/// Length unit.
pub const LENGTH_UNIT: &str = "mm";
/// Area unit.
pub const AREA_UNIT: &str = "mm^2";
/// Volume unit.
pub const VOLUME_UNIT: &str = "mm^3";
/// Mass unit.
pub const MASS_UNIT: &str = "kg";
/// Force unit.
pub const FORCE_UNIT: &str = "N";
/// Moment unit.
pub const MOMENT_UNIT: &str = "N*mm";
/// Stress unit.
pub const STRESS_UNIT: &str = "MPa";
/// Ratio unit.
pub const RATIO_UNIT: &str = "1";
/// Density unit.
pub const DENSITY_UNIT: &str = "kg/m^3";
/// Modulus unit.
pub const MODULUS_UNIT: &str = "GPa";
/// Expansion unit.
pub const EXPANSION_UNIT: &str = "1/K";
/// Material status marker.
pub const ILLUSTRATIVE_MATERIAL_STATUS: &str = "illustrative-constant-not-certified-material-data";

/// Material keys declared by this package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MaterialKey {
    /// Aluminium 6061-T6.
    Aluminium6061T6,
    /// Structural steel S355.
    SteelS355,
    /// Titanium Ti-6Al-4V.
    TitaniumTi6Al4V,
}

impl MaterialKey {
    /// The declared string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aluminium6061T6 => "aluminium-6061-t6",
            Self::SteelS355 => "steel-s355",
            Self::TitaniumTi6Al4V => "titanium-ti6al4v",
        }
    }

    /// Parse an exact declared value.
    pub fn parse(value: &str) -> Option<Self> {
        MATERIAL_KEYS
            .iter()
            .copied()
            .find(|key| key.as_str() == value)
    }
}

const MATERIAL_KEYS: [MaterialKey; 3] = [
    MaterialKey::Aluminium6061T6,
    MaterialKey::SteelS355,
    MaterialKey::TitaniumTi6Al4V,
];

/// Illustrative material constants used by the reference simulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialReference {
    /// The material key.
    pub key: MaterialKey,
    /// Human-readable label.
    pub label: &'static str,
    /// Density in kg/m^3.
    pub density_kg_m3: f64,
    /// Yield strength in MPa.
    pub yield_strength_mpa: f64,
    /// Elastic modulus in GPa.
    pub elastic_modulus_gpa: f64,
    /// Thermal expansion per kelvin.
    pub thermal_expansion_per_kelvin: f64,
    /// Reference status marker.
    pub reference_status: &'static str,
}

/// Declared material constants, in declaration order.
pub const MATERIALS: [MaterialReference; 3] = [
    MaterialReference {
        key: MaterialKey::Aluminium6061T6,
        label: "Aluminium 6061-T6 (illustrative)",
        density_kg_m3: 2_700.0,
        yield_strength_mpa: 276.0,
        elastic_modulus_gpa: 68.9,
        thermal_expansion_per_kelvin: 23.6e-6,
        reference_status: ILLUSTRATIVE_MATERIAL_STATUS,
    },
    MaterialReference {
        key: MaterialKey::SteelS355,
        label: "Structural steel S355 (illustrative)",
        density_kg_m3: 7_850.0,
        yield_strength_mpa: 355.0,
        elastic_modulus_gpa: 210.0,
        thermal_expansion_per_kelvin: 12.0e-6,
        reference_status: ILLUSTRATIVE_MATERIAL_STATUS,
    },
    MaterialReference {
        key: MaterialKey::TitaniumTi6Al4V,
        label: "Titanium Ti-6Al-4V (illustrative)",
        density_kg_m3: 4_430.0,
        yield_strength_mpa: 880.0,
        elastic_modulus_gpa: 113.8,
        thermal_expansion_per_kelvin: 8.6e-6,
        reference_status: ILLUSTRATIVE_MATERIAL_STATUS,
    },
];

/// Return the declared material constants for a key.
pub fn material_reference(key: &str) -> Result<MaterialReference, DomainError> {
    let resolved = MaterialKey::parse(key).ok_or_else(|| {
        DomainError::Parameter(format!(
            "material {} is not declared by this package",
            python_string_repr(key)
        ))
    })?;
    material_reference_for(resolved)
}

/// Return the declared material constants for a typed key.
pub fn material_reference_for(key: MaterialKey) -> Result<MaterialReference, DomainError> {
    MATERIALS
        .iter()
        .copied()
        .find(|reference| reference.key == key)
        .ok_or_else(|| {
            DomainError::Parameter(format!(
                "material {} has no declared constants",
                key.as_str()
            ))
        })
}

/// Bracket-like component parameters with units and inclusive ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct BracketParameters {
    length_mm: f64,
    width_mm: f64,
    thickness_mm: f64,
    hole_diameter_mm: f64,
    hole_count: i64,
    minimum_edge_margin_mm: f64,
    material: MaterialKey,
    load_kn: f64,
    load_points: i64,
    minimum_thickness_mm: f64,
    maximum_stress_mpa: f64,
    target_safety_factor: f64,
}

impl BracketParameters {
    /// Plate length in millimetres.
    pub fn length_mm(&self) -> f64 {
        self.length_mm
    }
    /// Plate width in millimetres.
    pub fn width_mm(&self) -> f64 {
        self.width_mm
    }
    /// Plate thickness in millimetres.
    pub fn thickness_mm(&self) -> f64 {
        self.thickness_mm
    }
    /// Fastener hole diameter in millimetres.
    pub fn hole_diameter_mm(&self) -> f64 {
        self.hole_diameter_mm
    }
    /// Fastener hole count.
    pub fn hole_count(&self) -> i64 {
        self.hole_count
    }
    /// Required material beside the hole pattern.
    pub fn minimum_edge_margin_mm(&self) -> f64 {
        self.minimum_edge_margin_mm
    }
    /// Declared material key.
    pub fn material(&self) -> MaterialKey {
        self.material
    }
    /// Applied static load in kilonewtons.
    pub fn load_kn(&self) -> f64 {
        self.load_kn
    }
    /// Load introduction points.
    pub fn load_points(&self) -> i64 {
        self.load_points
    }
    /// Hard minimum plate thickness.
    pub fn minimum_thickness_mm(&self) -> f64 {
        self.minimum_thickness_mm
    }
    /// Hard maximum surrogate stress.
    pub fn maximum_stress_mpa(&self) -> f64 {
        self.maximum_stress_mpa
    }
    /// Safety factor target.
    pub fn target_safety_factor(&self) -> f64 {
        self.target_safety_factor
    }

    /// The illustrative constants for the declared material.
    pub fn material_reference(&self) -> Result<MaterialReference, DomainError> {
        material_reference_for(self.material)
    }
}

impl Default for BracketParameters {
    fn default() -> Self {
        Self::from_value(&CanonicalValue::Object(Default::default()))
            .expect("the declared defaults satisfy every invariant")
    }
}

impl ParameterModel for BracketParameters {
    const MODEL_NAME: &'static str = "BracketParameters";
    const DOMAIN_NAME: &'static str = "mechanical";
    const FIELD_NAMES: &'static [&'static str] = &[
        "length_mm",
        "width_mm",
        "thickness_mm",
        "hole_diameter_mm",
        "hole_count",
        "minimum_edge_margin_mm",
        "material",
        "load_kn",
        "load_points",
        "minimum_thickness_mm",
        "maximum_stress_mpa",
        "target_safety_factor",
    ];

    fn from_value(value: &CanonicalValue) -> Result<Self, ParameterError> {
        let object = as_object(value, Self::MODEL_NAME)?;
        reject_extra(object, Self::FIELD_NAMES)?;

        let length_mm = match get(object, "length_mm") {
            Some(value) => coerce_float("length_mm", value, 20.0, 500.0)?,
            None => 120.0,
        };
        let width_mm = match get(object, "width_mm") {
            Some(value) => coerce_float("width_mm", value, 20.0, 300.0)?,
            None => 80.0,
        };
        let thickness_mm = match get(object, "thickness_mm") {
            Some(value) => coerce_float("thickness_mm", value, 0.5, 25.0)?,
            None => 8.0,
        };
        let hole_diameter_mm = match get(object, "hole_diameter_mm") {
            Some(value) => coerce_float("hole_diameter_mm", value, 0.0, 50.0)?,
            None => 6.0,
        };
        let hole_count = match get(object, "hole_count") {
            Some(value) => coerce_int_in_range("hole_count", value, 0, 12)?,
            None => 2,
        };
        let minimum_edge_margin_mm = match get(object, "minimum_edge_margin_mm") {
            Some(value) => coerce_float("minimum_edge_margin_mm", value, 0.0, 10.0)?,
            None => 2.0,
        };
        let material = match get(object, "material") {
            Some(value) => coerce_enum(
                "material",
                value,
                &[
                    ("aluminium-6061-t6", MaterialKey::Aluminium6061T6),
                    ("steel-s355", MaterialKey::SteelS355),
                    ("titanium-ti6al4v", MaterialKey::TitaniumTi6Al4V),
                ],
            )?,
            None => MaterialKey::Aluminium6061T6,
        };
        let load_kn = match get(object, "load_kn") {
            Some(value) => coerce_float("load_kn", value, 0.0, 200.0)?,
            None => 1.0,
        };
        let load_points = match get(object, "load_points") {
            Some(value) => coerce_int_in_range("load_points", value, 1, 8)?,
            None => 2,
        };
        let minimum_thickness_mm = match get(object, "minimum_thickness_mm") {
            Some(value) => coerce_float("minimum_thickness_mm", value, 0.1, 25.0)?,
            None => 2.0,
        };
        let maximum_stress_mpa = match get(object, "maximum_stress_mpa") {
            Some(value) => coerce_float("maximum_stress_mpa", value, 1.0, 2_000.0)?,
            None => 150.0,
        };
        let target_safety_factor = match get(object, "target_safety_factor") {
            Some(value) => coerce_float("target_safety_factor", value, 1.0, 10.0)?,
            None => 2.0,
        };

        let parameters = Self {
            length_mm,
            width_mm,
            thickness_mm,
            hole_diameter_mm,
            hole_count,
            minimum_edge_margin_mm,
            material,
            load_kn,
            load_points,
            minimum_thickness_mm,
            maximum_stress_mpa,
            target_safety_factor,
        };
        parameters.validate_pattern()?;
        Ok(parameters)
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("length_mm", WireValue::Float(self.length_mm)),
            ("width_mm", WireValue::Float(self.width_mm)),
            ("thickness_mm", WireValue::Float(self.thickness_mm)),
            ("hole_diameter_mm", WireValue::Float(self.hole_diameter_mm)),
            ("hole_count", WireValue::Integer(self.hole_count.into())),
            (
                "minimum_edge_margin_mm",
                WireValue::Float(self.minimum_edge_margin_mm),
            ),
            ("material", WireValue::Text(self.material.as_str().into())),
            ("load_kn", WireValue::Float(self.load_kn)),
            ("load_points", WireValue::Integer(self.load_points.into())),
            (
                "minimum_thickness_mm",
                WireValue::Float(self.minimum_thickness_mm),
            ),
            (
                "maximum_stress_mpa",
                WireValue::Float(self.maximum_stress_mpa),
            ),
            (
                "target_safety_factor",
                WireValue::Float(self.target_safety_factor),
            ),
        ])
    }

    fn schema_digest() -> String {
        BRACKET_SCHEMA_DIGEST.clone()
    }
}

impl BracketParameters {
    fn validate_pattern(&self) -> Result<(), ParameterError> {
        if self.hole_count > 0 && self.hole_diameter_mm <= 0.0 {
            return Err(ParameterError::model(
                "hole_diameter_mm must be positive when holes are declared",
            ));
        }
        if (self.hole_count as f64) * self.hole_diameter_mm > self.length_mm {
            return Err(ParameterError::model(
                "the hole pattern does not fit along the declared length",
            ));
        }
        if (self.hole_count as f64) * (self.hole_diameter_mm * self.hole_diameter_mm)
            >= self.length_mm * self.width_mm
        {
            return Err(ParameterError::model(
                "the hole pattern cannot fit on the declared plate",
            ));
        }
        if self.hole_count == 0 && self.minimum_edge_margin_mm > 0.0 {
            return Err(ParameterError::model(
                "minimum_edge_margin_mm requires at least one declared hole",
            ));
        }
        Ok(())
    }
}

/// Declared parameter specifications.
pub static PARAMETER_SPECS: LazyLock<Vec<ParameterSpec>> = LazyLock::new(|| {
    vec![
        ParameterSpec::new(
            "length_mm",
            LENGTH_UNIT,
            20.0,
            500.0,
            ParameterDefault::Float(120.0),
            "Plate length in millimetres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "width_mm",
            LENGTH_UNIT,
            20.0,
            300.0,
            ParameterDefault::Float(80.0),
            "Plate width in millimetres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "thickness_mm",
            LENGTH_UNIT,
            0.5,
            25.0,
            ParameterDefault::Float(8.0),
            "Plate thickness in millimetres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "hole_diameter_mm",
            LENGTH_UNIT,
            0.0,
            50.0,
            ParameterDefault::Float(6.0),
            "Fastener hole diameter in millimetres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "hole_count",
            RATIO_UNIT,
            0.0,
            12.0,
            ParameterDefault::Float(2.0),
            "Number of fastener holes.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "minimum_edge_margin_mm",
            LENGTH_UNIT,
            0.0,
            10.0,
            ParameterDefault::Float(2.0),
            "Material required beside the hole pattern in millimetres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "material",
            "key",
            0.0,
            0.0,
            ParameterDefault::Text("aluminium-6061-t6".to_owned()),
            "Declared material key with illustrative constants.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "load_kn",
            "kN",
            0.0,
            200.0,
            ParameterDefault::Float(1.0),
            "Applied static load in kilonewtons.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "load_points",
            RATIO_UNIT,
            1.0,
            8.0,
            ParameterDefault::Float(2.0),
            "Number of load introduction points.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "minimum_thickness_mm",
            LENGTH_UNIT,
            0.1,
            25.0,
            ParameterDefault::Float(2.0),
            "Hard minimum plate thickness in millimetres.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "maximum_stress_mpa",
            STRESS_UNIT,
            1.0,
            2_000.0,
            ParameterDefault::Float(150.0),
            "Hard maximum surrogate stress in megapascals.",
        )
        .expect("declared specification"),
        ParameterSpec::new(
            "target_safety_factor",
            RATIO_UNIT,
            1.0,
            10.0,
            ParameterDefault::Float(2.0),
            "Safety factor target used by the bounded score.",
        )
        .expect("declared specification"),
    ]
});

/// Sorted declared parameter names.
pub static PARAMETER_NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut names: Vec<String> = PARAMETER_SPECS
        .iter()
        .map(|spec| spec.name().to_owned())
        .collect();
    names.sort();
    names
});

/// Parameter-schema digest.
pub static BRACKET_SCHEMA_DIGEST: LazyLock<String> = LazyLock::new(|| {
    parameter_schema_digest(&PARAMETER_SPECS).expect("the declared schema is canonical")
});
