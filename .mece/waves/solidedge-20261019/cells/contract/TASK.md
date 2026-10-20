# Contract foundation — OpenCode Go

Implement only your owned files from WAVE.md. Read /home/v9i/Projects/Kala-Agent/.mece/waves/solidedge-20261019/WAVE.md and existing FreeCAD pyproject style. Work in /home/v9i/Projects/Kala-Agent. Do not delegate, commit, push, install unrelated software or change user files. Read applicable AGENTS.md if any.

Create standalone distribution kala-solidedge-bridge, package kala_solidedge_bridge, Python>=3.11, deps pydantic>=2.10,<3 and pywin32 with Windows platform marker. __init__.py imports protocol only, never runtime/session/server or kala_agent. Add CLI script kala-se-bridge pointing to kala_solidedge_bridge.__main__:main (bridge worker adds implementation later). Dev settings strict pytest/ruff/mypy following adapter style; pythonpath only src for this standalone package. No core dependency.

Implement protocol.py frozen/extra-forbid Pydantic v2 records with finite numeric validation rejecting bools and numeric strings, bounded identifiers, version=1, documented JSON serialization. Use discriminated op models:
PartNew(op='part.new',name).
SketchRectangle(op='sketch.rectangle',plane XY|XZ|YZ,width,height,x=0,y=0).
SketchCircle(op='sketch.circle',plane,diameter,x=0,y=0).
SolidExtrude(op='solid.extrude',length,reversed=False).
SolidCut(op='solid.cut',length=None => through-all; fixed canonical normal direction, no invented reversed flag).
SolidRevolve(op='solid.revolve',angle=360,axis='Y').
PartMeasure(op='part.measure').
PartSave(op='part.save').
PartExportStep(op='part.export_step').
AssemblyNew(op='assembly.new',name).
AssemblyPlace(op='assembly.place',component,x=0,y=0,z=0).
AssemblySave(op='assembly.save').
AssemblyExportStep(op='assembly.export_step').
Procedure sequence must validate: unique part names <=8; profile must be consumed before next/new part; solid needs profile and part; cut needs existing solid; measure/save/export need solid; assembly references only finished created parts, translations each component max once; never accepts unknown op/path/code. Part selection is current part; no arbitrary document IDs. Model operation cap600 independent of core max12 IR steps per component (compiler enforces IR bounds). Sizes inclusive0.001..5000mm; offsets[-5000,5000], angle(0,360], procedure1..600. Artifact paths are NEVER wire input.

BuildRequest(protocol_version=1,request_id safe<=64,calibration_id safe<=128,summary<=500,procedure tuple[BridgeOp,...],timeout_seconds default120 range1..900,density_kg_m3 optional finite positive<=22000 defaultNone). Inputs accepted from JSON arrays using model_validate_json; don't reject normal JSON ints for real-valued numbers. canonical_request_bytes()/request_digest() stdlib SHA256 deterministic over complete request payload except request_id (must document choice).
JobState queued,running,succeeded,failed,cancelled,indeterminate,interrupted.
StepTrace(seq>=0,op,status/message bounded,duration_ms nonnegative).
RangeBox(min_mm,max_mm triples finite; min<=max).
PartResult(name,valid,volume_mm3,surface_area_mm2,range_box local coordinates, optionalmass fields if explicit density only).
Artifact(name safe filename,format par|asm|step,media_type,size_bytes>=0,sha256), NO absolute/relative disk paths.
BuildResult(build_id,state,parts,assembly_bounds optional,artifacts,trace,warnings,error optional).
BuildStatus(build_id,state,step_index,total,trace,result optional,message).
Health(protocol_version,bridge_version,platform,supported bool,available bool,busy bool,se_version optional,calibration_id optional,capabilities tuple bounded,detail bounded). Capabilities not magicready.
ErrorResponse(code,message) and BuildAccepted(build_id,state).
Keep protocols minimum practical and precise; add procedure invariants necessary to avoid wrongfeature ordering. Document publicnames in cell report for dependent workers.

units.py mm_to_m,m_to_mm,deg_to_rad,m3_to_mm3,m2_to_mm2 with finite validation no bool/NaN. errors.py BridgeError(code,message), InputError, UnavailableError, CalibrationRequiredError, BusyError, ConflictError or consistent typed subset. No pywin32 imports anywhere in owned files.

Meaningful tests valid plate-with-hole, assembly request, malformed sequences/references, unknown/extra fields, bool/NaN/string/out-of-range/version limits, deterministic hashing, model_validate_json arrays, unit conversion. Strict mypy source (tests if practical). Run actual checks via project .venv tools, don't install.

Write /home/v9i/Projects/Kala-Agent/.mece/waves/solidedge-20261019/cells/contract/REPORT.md with actual changedpaths, public contract summary, checks and outcomes, blockers and session id if known. You may add SESSION.md in own cell. Stop when finished; report no Windows tests. Keep implementation focused <=1500lines total if possible, no excessive abstraction.

