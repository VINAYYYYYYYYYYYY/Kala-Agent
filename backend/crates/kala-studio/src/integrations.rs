//! The static Studio integration registry.
//!
//! Rows, platform lists, neutral-file capabilities, honest status labels and
//! notes are copied from `kala_agent.studio.integrations.registry`. Nothing
//! here performs native automation or reaches the network.

/// One integration row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrationRecord {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub platforms: &'static [&'static str],
    pub status: &'static str,
    pub capabilities: &'static [&'static str],
    pub notes: &'static str,
}

const STEP_ONLY: &[&str] = &["step"];
const STEP_STL: &[&str] = &["step", "stl"];
const STL_ONLY: &[&str] = &["stl"];

const REGISTRY: &[IntegrationRecord] = &[
    IntegrationRecord {
        id: "solidworks",
        name: "SOLIDWORKS",
        category: "cad",
        platforms: &["windows", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "Native automation requires a SOLIDWORKS licence and Windows; the studio does not drive SOLIDWORKS through its COM automation and is limited to neutral-file imports. Hand off the STEP file the studio exports into the user's local SOLIDWORKS session.",
    },
    IntegrationRecord {
        id: "solid_edge",
        name: "Solid Edge",
        category: "cad",
        platforms: &["windows", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "Native automation requires Solid Edge and a Windows host; the studio does not call into Solid Edge from this process and exposes STEP neutral-file handoff only.",
    },
    IntegrationRecord {
        id: "autocad",
        name: "AutoCAD",
        category: "cad",
        platforms: &["windows", "macos", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "STEP support varies by AutoCAD release and product edition; operators should verify their local installation imports STEP before relying on a handoff. The studio does not drive AutoCAD through its automation API and exports the STEP file for the operator to import locally.",
    },
    IntegrationRecord {
        id: "fusion360",
        name: "Fusion 360",
        category: "cad",
        platforms: &["windows", "macos", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "Fusion 360 imports STEP through its data panel; native REST automation requires a Fusion 360 licence and an authenticated client. The studio is limited to neutral-file imports and does not attempt to call the Fusion REST API.",
    },
    IntegrationRecord {
        id: "freecad",
        name: "FreeCAD",
        category: "cad",
        platforms: &["windows", "macos", "linux", "open-source"],
        status: "available_via_neutral_file",
        capabilities: STEP_STL,
        notes: "FreeCAD imports STEP and STL natively. The studio does not automate FreeCAD; the operator imports the exported STEP or STL in a local FreeCAD session.",
    },
    IntegrationRecord {
        id: "openscad",
        name: "OpenSCAD",
        category: "cad",
        platforms: &["windows", "macos", "linux", "open-source"],
        status: "not_configured",
        capabilities: STL_ONLY,
        notes: "The studio does not transpile geometry into OpenSCAD script; OpenSCAD's CSG language is not a parametric sweep of size, radius, and height. STL export is the supported handoff for this integration status; install OpenSCAD locally to view the studio STL output.",
    },
    IntegrationRecord {
        id: "ansys_mechanical",
        name: "ANSYS Mechanical",
        category: "analysis",
        platforms: &["windows", "linux", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "ANSYS Mechanical requires a Windows host with ANSYS installed and a paid licence. Geometry is imported through SpaceClaim or ANSYS Workbench, not directly by the Mechanical solver. The studio does not automate ANSYS Mechanical.",
    },
    IntegrationRecord {
        id: "abaqus",
        name: "Abaqus",
        category: "analysis",
        platforms: &["windows", "linux", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "Abaqus/CAE ingests STEP geometry through its part module and requires a paid licence. The studio exports the STEP file and the operator drives Abaqus/CAE locally; the studio does not run the Abaqus analysis.",
    },
    IntegrationRecord {
        id: "comsol",
        name: "COMSOL Multiphysics",
        category: "analysis",
        platforms: &["windows", "macos", "linux", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "COMSOL Multiphysics imports STEP through its geometry import node and requires a paid licence. The studio does not invoke the COMSOL server API; the operator completes meshing, physics, and the local solve in their COMSOL session.",
    },
    IntegrationRecord {
        id: "simcenter_nastran",
        name: "Simcenter Nastran",
        category: "analysis",
        platforms: &["windows", "linux", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "Simcenter Nastran does not ingest STEP directly; the geometry passes through Femap or Siemens NX as a preprocessor. The studio exports STEP for the geometry phase only; Nastran is a separate analysis product and is not driven by this process.",
    },
    IntegrationRecord {
        id: "altair_optistruct",
        name: "Altair OptiStruct",
        category: "analysis",
        platforms: &["windows", "linux", "commercial"],
        status: "native_api_unavailable",
        capabilities: STEP_ONLY,
        notes: "OptiStruct reads solver decks rather than STEP geometry; Altair HyperMesh is the mesh and property preprocessor. The studio exports STEP for the geometry phase only; OptiStruct is a separate analysis product and is not driven by this process.",
    },
];

/// Return the registry in its frozen order.
#[must_use]
pub fn build_registry() -> &'static [IntegrationRecord] {
    REGISTRY
}

/// Return one registry row by id.
#[must_use]
pub fn integration_by_id(integration_id: &str) -> Option<&'static IntegrationRecord> {
    REGISTRY.iter().find(|row| row.id == integration_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_capabilities_are_frozen() {
        let ids: Vec<&str> = build_registry().iter().map(|row| row.id).collect();
        assert_eq!(
            ids,
            vec![
                "solidworks",
                "solid_edge",
                "autocad",
                "fusion360",
                "freecad",
                "openscad",
                "ansys_mechanical",
                "abaqus",
                "comsol",
                "simcenter_nastran",
                "altair_optistruct",
            ]
        );
        assert_eq!(
            integration_by_id("openscad").unwrap().capabilities,
            &["stl"]
        );
        assert!(integration_by_id("nope").is_none());
    }
}
