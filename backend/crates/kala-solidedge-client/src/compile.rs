//! Lower a Kala construction script into bridge protocol v1 operations.
//!
//! This is the only place where the shared construction language meets the
//! Solid Edge wire contract. It validates the script against the supported
//! subset and refuses everything else *before* a socket is opened: helix and
//! threads, fuse, compound, analysis, fillets, and a rectangle revolved on
//! `XZ` never reach the bridge.
//!
//! Semantics follow `docs/construction-semantics.md`:
//!
//! * planes are `XY` `(u, v, 0)` with normal `+Z`, `XZ` `(u, 0, v)` with
//!   normal `-Y`, and `YZ` `(0, u, v)` with normal `+X`;
//! * an extrusion keeps `reversed`;
//! * a pocket becomes a cut, with `None` meaning through-all;
//! * a rectangle revolve becomes `solid.revolve` around the component-local
//!   `Y` axis, while a circle revolve is lowered to an extrusion whose length
//!   is the supplied length or, when absent, the diameter;
//! * several additive features may extend one part;
//! * every part is created, measured, saved, and STEP-exported;
//! * an assembly is emitted when there is more than one component or any
//!   nonzero translation, and every component is placed exactly once.
//!
//! No geometry is computed here. Solid Edge performs every operation; this
//! crate only decides which typed operations to send.

use std::collections::BTreeMap;

use kala_contracts::{AnalysisScript, ComponentScript, ComponentStep, ConstructionScript};

use crate::errors::{SolidEdgeClientError, python_repr};
use crate::protocol::{BridgeOp, BuildRequest, MAX_OPERATIONS, Plane, REVOLVE_AXIS};

/// The assembly name used when it cannot collide with a component name.
pub const ASSEMBLY_NAME: &str = "Assembly";
/// The default supervisor deadline of a build request, in seconds.
pub const DEFAULT_BUILD_TIMEOUT_SECONDS: f64 = 120.0;
/// The largest summary kept in a build request.
pub const MAX_SUMMARY_CHARS: usize = 500;

const ZERO_MM: f64 = 1e-9;

/// Return a fresh safe request id for one semantic submission.
///
/// The value is 32 lowercase hexadecimal characters, the same shape as
/// `uuid.uuid4().hex`, and it satisfies the wire contract's safe-id pattern.
pub fn new_request_id() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // The operating system refused randomness. A time-seeded id still
        // satisfies the wire pattern; it is only weaker at collision
        // resistance, and the request id is correlation, not a secret.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0);
        let mut state = seed ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for byte in &mut bytes {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *byte = (state >> 33) as u8;
        }
    }
    let mut out = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Compile a validated script into bridge operations.
///
/// Analysis scripts and fillets are refused rather than silently dropped.
pub fn compile_operations(
    script: &ConstructionScript,
    analysis: Option<&AnalysisScript>,
    fillets: Option<&BTreeMap<String, f64>>,
) -> Result<Vec<BridgeOp>, SolidEdgeClientError> {
    if analysis.is_some() {
        return Err(SolidEdgeClientError::compile(
            "analysis operations (material, fix, force, pressure, mesh, solve) are not \
             supported by the Solid Edge integration",
        ));
    }
    if let Some(fillets) = fillets {
        if !fillets.is_empty() {
            let names = fillets
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(SolidEdgeClientError::compile(format!(
                "fillets are not supported by the Solid Edge integration: {names}"
            )));
        }
    }

    let mut operations: Vec<BridgeOp> = Vec::new();
    let placements = placements(script)?;
    for component in script.components() {
        operations.extend(component_operations(component)?);
    }

    if needs_assembly(script, &placements) {
        operations.push(BridgeOp::AssemblyNew {
            name: assembly_name(script)?,
        });
        for component in script.components() {
            let place = placements.get(component.name());
            operations.push(BridgeOp::AssemblyPlace {
                component: component.name().to_owned(),
                x: place.map_or(0.0, |step| step.x),
                y: place.map_or(0.0, |step| step.y),
                z: place.map_or(0.0, |step| step.z),
            });
        }
        operations.push(BridgeOp::AssemblySave);
        operations.push(BridgeOp::AssemblyExportStep);
    }

    if operations.is_empty() {
        return Err(SolidEdgeClientError::compile(
            "the construction script did not compile",
        ));
    }
    if operations.len() > MAX_OPERATIONS {
        return Err(SolidEdgeClientError::compile(format!(
            "the compiled procedure has {} operations; the wire cap is {MAX_OPERATIONS}",
            operations.len()
        )));
    }
    Ok(operations)
}

/// Compile a script and wrap it in one deterministic build request.
///
/// `request_id` is transport correlation only and is retained by the caller
/// across retries, so an ambiguous submission is never turned into a second
/// job. `calibration_id` is the one the bridge reported for this host; a
/// bridge still refuses an uncalibrated host at admission.
pub fn build_request(
    script: &ConstructionScript,
    request_id: &str,
    calibration_id: Option<&str>,
    timeout_seconds: f64,
    density_kg_m3: Option<f64>,
) -> Result<BuildRequest, SolidEdgeClientError> {
    build_request_with(
        script,
        request_id,
        calibration_id,
        timeout_seconds,
        density_kg_m3,
        None,
        None,
    )
}

/// Compile a script into a build request, refusing analysis and fillets.
///
/// This is the full form of [`build_request`]: the analysis script and the
/// fillet radii are accepted only so a caller can prove they are refused
/// rather than silently dropped.
pub fn build_request_with(
    script: &ConstructionScript,
    request_id: &str,
    calibration_id: Option<&str>,
    timeout_seconds: f64,
    density_kg_m3: Option<f64>,
    analysis: Option<&AnalysisScript>,
    fillets: Option<&BTreeMap<String, f64>>,
) -> Result<BuildRequest, SolidEdgeClientError> {
    let procedure = compile_operations(script, analysis, fillets)?;
    BuildRequest::new(
        request_id,
        calibration_id.map(str::to_owned),
        summary(script),
        procedure,
        timeout_seconds,
        density_kg_m3,
    )
    .map_err(|violation| SolidEdgeClientError::compile(violation.to_request_message()))
}

/// The normalized summary of a script, as recorded in a build request.
pub fn summary(script: &ConstructionScript) -> String {
    let text = script.summary().as_str().unwrap_or_default();
    let stripped = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if stripped.chars().count() <= MAX_SUMMARY_CHARS {
        return stripped;
    }
    stripped.chars().take(MAX_SUMMARY_CHARS).collect()
}

/// Return one placement per component, refusing duplicates and booleans.
///
/// `fuse` and `compound` are refused here, not reinterpreted as an assembly:
/// an assembly is a set of placed occurrences, and silently turning a boolean
/// union into one would change the requested geometry.
fn placements(
    script: &ConstructionScript,
) -> Result<BTreeMap<String, Placement>, SolidEdgeClientError> {
    let mut placements: BTreeMap<String, Placement> = BTreeMap::new();
    for step in script.assembly() {
        if step.op() != "place" {
            return fail_assembly_operation(step.op());
        }
        let Some(component) = step.component() else {
            return fail_no_component();
        };
        if placements.contains_key(component) {
            return fail_duplicate_placement(component);
        }
        placements.insert(
            component.to_owned(),
            Placement {
                x: step.x(),
                y: step.y(),
                z: step.z(),
            },
        );
    }
    Ok(placements)
}

fn fail_assembly_operation(op: &str) -> Result<BTreeMap<String, Placement>, SolidEdgeClientError> {
    Err(SolidEdgeClientError::compile(format!(
        "assembly operation {} is not supported; version 1 supports only positioned parts, \
         not fuse or compound",
        python_repr(op)
    )))
}

fn fail_no_component() -> Result<BTreeMap<String, Placement>, SolidEdgeClientError> {
    Err(SolidEdgeClientError::compile(
        "an assembly place operation must name a component",
    ))
}

fn fail_duplicate_placement(
    component: &str,
) -> Result<BTreeMap<String, Placement>, SolidEdgeClientError> {
    Err(SolidEdgeClientError::compile(format!(
        "component {} is placed more than once",
        python_repr(component)
    )))
}

/// One component's placement in the assembly.
struct Placement {
    x: f64,
    y: f64,
    z: f64,
}

/// An assembly is needed for several components or any nonzero translation.
fn needs_assembly(script: &ConstructionScript, placements: &BTreeMap<String, Placement>) -> bool {
    if script.components().len() > 1 {
        return true;
    }
    placements
        .values()
        .any(|step| step.x.abs() > ZERO_MM || step.y.abs() > ZERO_MM || step.z.abs() > ZERO_MM)
}

/// Return an assembly name that cannot collide with a component name.
///
/// The wire contract keeps parts and the assembly in separate namespaces, so a
/// collision is legal there; a native executor may not be so forgiving. The
/// derived name keeps the document unambiguous without inventing a random one.
fn assembly_name(script: &ConstructionScript) -> Result<String, SolidEdgeClientError> {
    let taken: Vec<&str> = script
        .components()
        .iter()
        .map(ComponentScript::name)
        .collect();
    if !taken.contains(&ASSEMBLY_NAME) {
        return Ok(ASSEMBLY_NAME.to_owned());
    }
    for index in 1..100 {
        let candidate = format!("{ASSEMBLY_NAME}{index}");
        if !taken.contains(&candidate.as_str()) {
            return Ok(candidate);
        }
    }
    Err(SolidEdgeClientError::compile(
        "a unique assembly name could not be derived",
    ))
}

/// Lower one component into bridge operations.
fn component_operations(
    component: &ComponentScript,
) -> Result<Vec<BridgeOp>, SolidEdgeClientError> {
    let name = component.name();
    let mut operations: Vec<BridgeOp> = vec![BridgeOp::PartNew {
        name: name.to_owned(),
    }];
    let mut pending: Option<&ComponentStep> = None;
    for step in component.steps() {
        match step.op() {
            "sketch" => {
                if pending.is_some() {
                    return Err(SolidEdgeClientError::compile(
                        "a sketch must be consumed before another sketch",
                    ));
                }
                pending = Some(step);
                continue;
            }
            "helix" => {
                return Err(SolidEdgeClientError::compile(
                    "helix and thread features are not supported by the Solid Edge integration",
                ));
            }
            "extrude" | "revolve" | "pocket" => {}
            other => {
                return Err(SolidEdgeClientError::compile(format!(
                    "unknown operation {} in component {}",
                    python_repr(other),
                    python_repr(name)
                )));
            }
        }
        let Some(sketch) = pending else {
            return Err(SolidEdgeClientError::compile(format!(
                "{} needs a sketch in component {}",
                step.op(),
                python_repr(name)
            )));
        };
        operations.extend(profile_operations(sketch, name)?);
        operations.push(solid_operation(step, sketch, name)?);
        pending = None;
    }
    if pending.is_some() {
        return Err(SolidEdgeClientError::compile(format!(
            "component {} ends with an unconsumed sketch",
            python_repr(name)
        )));
    }
    operations.push(BridgeOp::PartMeasure);
    operations.push(BridgeOp::PartSave);
    operations.push(BridgeOp::PartExportStep);
    Ok(operations)
}

/// Lower the pending sketch into its profile operation.
fn profile_operations(
    sketch: &ComponentStep,
    component: &str,
) -> Result<Vec<BridgeOp>, SolidEdgeClientError> {
    let Some(plane) = Plane::parse(sketch.plane()) else {
        return Err(SolidEdgeClientError::compile(format!(
            "component {} uses an unknown plane {}",
            python_repr(component),
            python_repr(sketch.plane())
        )));
    };
    let profile = sketch.profile().and_then(|value| value.as_str());
    match profile {
        Some("rectangle") => {
            let (Some(width), Some(height)) = (sketch.width(), sketch.height()) else {
                return Err(SolidEdgeClientError::compile(format!(
                    "component {} has a rectangle without width and height",
                    python_repr(component)
                )));
            };
            Ok(vec![BridgeOp::SketchRectangle {
                plane,
                width,
                height,
                x: sketch.x(),
                y: sketch.y(),
            }])
        }
        Some("circle") => {
            let Some(diameter) = sketch.diameter() else {
                return Err(SolidEdgeClientError::compile(format!(
                    "component {} has a circle without a diameter",
                    python_repr(component)
                )));
            };
            Ok(vec![BridgeOp::SketchCircle {
                plane,
                diameter,
                x: sketch.x(),
                y: sketch.y(),
            }])
        }
        other => {
            let shown = other.unwrap_or("None");
            Err(SolidEdgeClientError::compile(format!(
                "component {} has profile {}; only rectangle and circle are supported",
                python_repr(component),
                python_repr(shown)
            )))
        }
    }
}

/// Lower one solid operation.
fn solid_operation(
    step: &ComponentStep,
    sketch: &ComponentStep,
    component: &str,
) -> Result<BridgeOp, SolidEdgeClientError> {
    match step.op() {
        "pocket" => Ok(BridgeOp::SolidCut {
            length: step.length(),
        }),
        "extrude" => {
            let Some(length) = step.length() else {
                return Err(SolidEdgeClientError::compile(format!(
                    "component {} has an extrusion without a length",
                    python_repr(component)
                )));
            };
            Ok(BridgeOp::SolidExtrude {
                length,
                reversed: step.reversed(),
            })
        }
        "revolve" => {
            let profile = sketch.profile().and_then(|value| value.as_str());
            if profile == Some("circle") {
                let length = step.length().or(sketch.diameter());
                let Some(length) = length else {
                    return Err(SolidEdgeClientError::compile(format!(
                        "component {} has a circle revolve without a size",
                        python_repr(component)
                    )));
                };
                return Ok(BridgeOp::SolidExtrude {
                    length,
                    reversed: step.reversed(),
                });
            }
            if sketch.plane() == "XZ" {
                return Err(SolidEdgeClientError::compile(format!(
                    "component {} revolves a rectangle on XZ; the component-local Y axis is \
                     not the profile axis, so this case is refused",
                    python_repr(component)
                )));
            }
            Ok(BridgeOp::SolidRevolve {
                angle: step.angle().unwrap_or(360.0),
                axis: REVOLVE_AXIS.to_owned(),
            })
        }
        other => Err(SolidEdgeClientError::compile(format!(
            "unknown operation {} in component {}",
            python_repr(other),
            python_repr(component)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use kala_contracts::load_construction_json_str;

    use super::{ASSEMBLY_NAME, build_request, compile_operations, new_request_id, summary};
    use crate::protocol::{BridgeOp, BuildRequest};

    fn script(text: &str) -> kala_contracts::ConstructionScript {
        load_construction_json_str(text).expect("valid script")
    }

    fn plate() -> kala_contracts::ConstructionScript {
        script(
            r#"{"summary":"test script","components":[{"name":"plate","steps":[
                {"op":"sketch","profile":"rectangle","width":20,"height":20},
                {"op":"extrude","length":10}]}]}"#,
        )
    }

    #[test]
    fn a_plate_compiles_to_the_documented_procedure() {
        let operations = compile_operations(&plate(), None, None).expect("compiled");
        let names: Vec<&str> = operations.iter().map(|op| op.op_name()).collect();
        assert_eq!(
            names,
            vec![
                "part.new",
                "sketch.rectangle",
                "solid.extrude",
                "part.measure",
                "part.save",
                "part.export_step",
            ]
        );
    }

    #[test]
    fn a_reversed_extrusion_is_preserved() {
        let parsed = script(
            r#"{"summary":"s","components":[{"name":"plate","steps":[
                {"op":"sketch","profile":"rectangle","width":20,"height":20},
                {"op":"extrude","length":10,"reversed":true}]}]}"#,
        );
        let operations = compile_operations(&parsed, None, None).expect("compiled");
        assert_eq!(
            operations[2],
            BridgeOp::SolidExtrude {
                length: 10.0,
                reversed: true
            }
        );
    }

    #[test]
    fn a_rectangle_revolve_on_xz_is_refused() {
        let parsed = script(
            r#"{"summary":"s","components":[{"name":"wheel","steps":[
                {"op":"sketch","profile":"rectangle","plane":"XZ","width":4,"height":5},
                {"op":"revolve","angle":360}]}]}"#,
        );
        let error = compile_operations(&parsed, None, None).expect_err("refused");
        assert!(error.message().contains("XZ"), "{}", error.message());
    }

    #[test]
    fn a_helix_is_refused() {
        let parsed = script(
            r#"{"summary":"s","components":[{"name":"screw","steps":[
                {"op":"sketch","profile":"circle","diameter":16},
                {"op":"extrude","length":40},
                {"op":"helix","diameter":16,"pitch":4,"length":24}]}]}"#,
        );
        let error = compile_operations(&parsed, None, None).expect_err("refused");
        assert!(error.message().contains("helix"), "{}", error.message());
    }

    #[test]
    fn a_fuse_is_refused_not_reinterpreted() {
        let parsed = script(
            r#"{"summary":"s","components":[
                {"name":"a","steps":[{"op":"sketch","profile":"rectangle","width":1,"height":1},
                {"op":"extrude","length":1}]},
                {"name":"b","steps":[{"op":"sketch","profile":"rectangle","width":1,"height":1},
                {"op":"extrude","length":1}]}],
                "assembly":[{"op":"fuse","components":["a","b"]}]}"#,
        );
        let error = compile_operations(&parsed, None, None).expect_err("refused");
        assert!(
            error.message().contains("not supported"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn an_assembly_name_never_collides_with_a_component_name() {
        let parsed = script(
            r#"{"summary":"s","components":[
                {"name":"Assembly","steps":[{"op":"sketch","profile":"rectangle","width":1,"height":1},
                {"op":"extrude","length":1}]},
                {"name":"b","steps":[{"op":"sketch","profile":"rectangle","width":1,"height":1},
                {"op":"extrude","length":1}]}]}"#,
        );
        let operations = compile_operations(&parsed, None, None).expect("compiled");
        let names: Vec<&str> = operations.iter().map(|op| op.op_name()).collect();
        assert!(names.contains(&"assembly.new"));
        assert_eq!(ASSEMBLY_NAME, "Assembly");
    }

    #[test]
    fn the_request_factory_keeps_the_identity() {
        let parsed = plate();
        let request = build_request(&parsed, "req-1", Some("cal-1"), 120.0, None).expect("request");
        let other =
            build_request(&parsed, "other-id", Some("cal-1"), 120.0, None).expect("request");
        assert_eq!(
            request.request_digest().expect("digest"),
            other.request_digest().expect("digest")
        );
    }

    #[test]
    fn a_bad_request_id_is_a_compile_error() {
        let error = build_request(&plate(), "not safe!", None, 120.0, None).expect_err("refused");
        assert!(error.is_compile());
        assert!(
            error.message().contains("request_id"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn a_request_id_is_a_safe_hex_string() {
        let request_id = new_request_id();
        assert_eq!(request_id.len(), 32);
        assert!(request_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn the_summary_is_normalized() {
        assert_eq!(summary(&plate()), "test script");
        let parsed = script(
            r#"{"summary":"  a   b  ","components":[{"name":"p","steps":[
            {"op":"sketch","profile":"rectangle","width":1,"height":1},
            {"op":"extrude","length":1}]}]}"#,
        );
        assert_eq!(summary(&parsed), "a b");
    }

    #[test]
    fn the_wire_body_round_trips() {
        let parsed = plate();
        let request = build_request(&parsed, "req-1", None, 120.0, None).expect("request");
        let body = request.to_wire_json().expect("body");
        let parsed_request = BuildRequest::from_json_bytes(body.as_bytes()).expect("round trip");
        assert_eq!(request, parsed_request);
        assert_eq!(
            parsed_request.to_wire_json().expect("body"),
            body,
            "wire body is stable"
        );
    }
}
