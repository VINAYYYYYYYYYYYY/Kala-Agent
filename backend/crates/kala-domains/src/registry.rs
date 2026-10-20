//! Typed registry and untyped dispatch for the three existing domain names.

use kala_contracts::{DomainPackageManifest, EvaluationResult, WireValue};
use kala_json::CanonicalValue;

use crate::building::{BUILDING_PACKAGE, BuildingParameters};
use crate::context::EvaluationContext;
use crate::error::{DomainCallError, DomainError};
use crate::mechanical::{BracketParameters, MECHANICAL_PACKAGE};
use crate::package::DomainPackageApi;
use crate::parameters::ParameterModel;
use crate::wing::{WING_PACKAGE, WingParameters};

/// The three declared domain names, in the application's stable order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DomainName {
    /// `building`
    Building,
    /// `mechanical`
    Mechanical,
    /// `wing`
    Wing,
}

impl DomainName {
    /// Every domain name in stable order.
    pub const ALL: [DomainName; 3] = [
        DomainName::Building,
        DomainName::Mechanical,
        DomainName::Wing,
    ];

    /// The registry name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Mechanical => "mechanical",
            Self::Wing => "wing",
        }
    }

    /// Parse an exact registry name.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|domain| domain.as_str() == value)
    }
}

/// The package registered under a domain name.
pub fn package_for(domain: DomainName) -> &'static dyn DomainPackageApi {
    match domain {
        DomainName::Building => &*BUILDING_PACKAGE,
        DomainName::Mechanical => &*MECHANICAL_PACKAGE,
        DomainName::Wing => &*WING_PACKAGE,
    }
}

/// The package registered under an exact domain name.
pub fn package(domain: &str) -> Option<&'static dyn DomainPackageApi> {
    DomainName::parse(domain).map(package_for)
}

/// Every registered package, in stable order.
pub fn packages() -> Vec<&'static dyn DomainPackageApi> {
    DomainName::ALL.into_iter().map(package_for).collect()
}

/// The manifests of every registered package, in stable order.
pub fn manifests() -> Vec<DomainPackageManifest> {
    packages()
        .into_iter()
        .map(|package| package.manifest().clone())
        .collect()
}

/// A typed parameter set for one of the three domains.
#[derive(Clone, Debug, PartialEq)]
pub enum DomainParameters {
    /// Building parameters.
    Building(BuildingParameters),
    /// Mechanical parameters.
    Mechanical(BracketParameters),
    /// Wing parameters.
    Wing(WingParameters),
}

impl DomainParameters {
    /// The domain name this parameter set belongs to.
    pub const fn domain(&self) -> DomainName {
        match self {
            Self::Building(_) => DomainName::Building,
            Self::Mechanical(_) => DomainName::Mechanical,
            Self::Wing(_) => DomainName::Wing,
        }
    }

    /// Parse one domain's parameters from an untyped JSON value.
    pub fn from_value(domain: DomainName, value: &CanonicalValue) -> Result<Self, DomainCallError> {
        Ok(match domain {
            DomainName::Building => Self::Building(
                BuildingParameters::from_value(value).map_err(DomainCallError::Parameters)?,
            ),
            DomainName::Mechanical => Self::Mechanical(
                BracketParameters::from_value(value).map_err(DomainCallError::Parameters)?,
            ),
            DomainName::Wing => {
                Self::Wing(WingParameters::from_value(value).map_err(DomainCallError::Parameters)?)
            }
        })
    }

    /// Build the declared defaults for one domain.
    pub fn defaults(domain: DomainName) -> Self {
        match domain {
            DomainName::Building => Self::Building(BuildingParameters::default()),
            DomainName::Mechanical => Self::Mechanical(BracketParameters::default()),
            DomainName::Wing => Self::Wing(WingParameters::default()),
        }
    }

    /// The `model_dump(mode="json")` shape.
    pub fn to_wire_value(&self) -> WireValue {
        match self {
            Self::Building(parameters) => parameters.to_wire_value(),
            Self::Mechanical(parameters) => parameters.to_wire_value(),
            Self::Wing(parameters) => parameters.to_wire_value(),
        }
    }

    /// Compact UTF-8 JSON in model field order.
    pub fn to_wire_json(&self) -> Result<String, DomainCallError> {
        self.to_wire_value()
            .encode(false)
            .map_err(|error| DomainCallError::Domain(DomainError::Package(error.to_string())))
    }
}

/// Parse parameters and evaluate one step through the registry.
pub fn evaluate(
    domain: DomainName,
    step_id: &str,
    context: &EvaluationContext,
    parameters: &CanonicalValue,
) -> Result<EvaluationResult, DomainCallError> {
    package_for(domain).evaluate_value(step_id, context, parameters)
}

/// Parse one domain's parameters and evaluate through the typed enum.
pub fn evaluate_typed(
    parameters: &DomainParameters,
    step_id: &str,
    context: &EvaluationContext,
) -> Result<EvaluationResult, DomainCallError> {
    let domain = parameters.domain();
    let value = parameters
        .to_wire_value()
        .to_canonical_value()
        .map_err(|error| DomainCallError::Domain(DomainError::Package(error.to_string())))?;
    evaluate(domain, step_id, context, &value)
}
