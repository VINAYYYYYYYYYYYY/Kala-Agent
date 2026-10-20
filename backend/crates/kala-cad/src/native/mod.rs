//! Native kernel implementation module.

pub(crate) mod compile;
pub(crate) mod drawing;
pub(crate) mod ffi;
pub(crate) mod measure;
pub(crate) mod mesh;
pub(crate) mod shape;
pub(crate) mod step;

pub use compile::{
    CompilationResult, CompileOptions, CompiledComponent, compile_json_bytes, compile_json_str,
    compile_script, compile_script_with_options,
};
pub use drawing::{
    default_section_offset, hlr_views, hlr_views_with_options, section, section_with_options,
};
pub use measure::{measure_and_check, measure_shape, topology};
pub use mesh::tessellate;
pub use shape::Shape;
pub use step::{
    COARSENED_WARNING, COARSER_ANGULAR_DEFLECTION_RAD, COARSER_MESH_DEFLECTION_MM,
    StepImportOutcome, import_step_view, step_bytes, step_bytes_with_options, step_shape,
};
