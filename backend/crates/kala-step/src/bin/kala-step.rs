//! `kala-step` — export one studio primitive as a native exact B-rep STEP file.
//!
//! The CLI is deliberately small: it is the operator/debug entry point and the
//! driver for future service and benchmark validation, not a second API.
//!
//! ```text
//! kala-step export --kind box --size-x 20 --size-y 30 --size-z 40 --out box.step
//! kala-step export --kind cylinder --radius 5 --height 10
//! kala-step capability
//! ```
//!
//! Dimensions use the same defaults, bounds and messages as the studio route.
//! The writer never overwrites an existing file unless `--force` is given, and
//! `--repeat N` runs the same export N times and reports the elapsed wall time
//! for the same host (a local harness, not an optimization claim).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use kala_geometry::{PrimitiveKind, PrimitiveParameters};
use kala_step::{MAX_STEP_BYTES, NativeStepExporter, StepExporter, export_parameters};

const USAGE: &str = "\
kala-step — native exact B-rep STEP export for Kala Studio primitives

USAGE:
    kala-step capability
    kala-step export --kind <box|cylinder|sphere> [dimension flags] [options]

DIMENSION FLAGS (defaults in brackets, millimetres):
    --size-x <mm>      box extent on X [10]
    --size-y <mm>      box extent on Y [10]
    --size-z <mm>      box extent on Z [10]
    --radius <mm>      cylinder or sphere radius [5]
    --height <mm>      cylinder height [10]

OPTIONS:
    --out <path>       write the STEP file (- means stdout) [-]
    --force            allow overwriting an existing --out file
    --repeat <n>       export n times and report the elapsed wall time [1]
    -h, --help         show this help

The export is an exact analytic B-rep in millimetres, centred on the origin.
No Python, OCP or other sidecar is involved.
";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kala-step: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        print!("{USAGE}");
        return Err("a command is required".to_owned());
    };
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        "capability" => {
            if let Some(arg) = rest.first() {
                return Err(format!("unknown argument for capability: {arg}"));
            }
            let capability = NativeStepExporter.capability();
            println!(
                "{{\"available\":{},\"exporter\":{}}}",
                capability.available,
                match capability.exporter_name {
                    Some(name) => format!("\"{name}\""),
                    None => "null".to_owned(),
                }
            );
            Ok(())
        }
        "export" => export(&rest),
        other => Err(format!("unknown command: {other}")),
    }
}

struct Options {
    kind: String,
    radius: Option<f64>,
    height: Option<f64>,
    size: [Option<f64>; 3],
    out: PathBuf,
    force: bool,
    repeat: u32,
}

fn export(args: &[String]) -> Result<(), String> {
    let mut options = Options {
        kind: String::new(),
        radius: None,
        height: None,
        size: [None; 3],
        out: PathBuf::from("-"),
        force: false,
        repeat: 1,
    };
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = |index: &mut usize| -> Result<&str, String> {
            *index += 1;
            args.get(*index)
                .map(String::as_str)
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag {
            "--kind" => options.kind = value(&mut index)?.to_owned(),
            "--radius" => options.radius = Some(parse_dimension("radius", value(&mut index)?)?),
            "--height" => options.height = Some(parse_dimension("height", value(&mut index)?)?),
            "--size-x" => options.size[0] = Some(parse_dimension("size_x", value(&mut index)?)?),
            "--size-y" => options.size[1] = Some(parse_dimension("size_y", value(&mut index)?)?),
            "--size-z" => options.size[2] = Some(parse_dimension("size_z", value(&mut index)?)?),
            "--out" => options.out = PathBuf::from(value(&mut index)?),
            "--force" => options.force = true,
            "--repeat" => {
                let repeat = value(&mut index)?;
                options.repeat = repeat
                    .parse::<u32>()
                    .map_err(|_| format!("--repeat must be a positive integer, got {repeat}"))?;
                if options.repeat == 0 {
                    return Err("--repeat must be at least 1".to_owned());
                }
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }

    let kind = PrimitiveKind::try_from(options.kind.as_str())
        .map_err(|error| error.message().to_owned())?;

    let mut parameters = PrimitiveParameters::new();
    match kind {
        PrimitiveKind::Box => {
            for (name, value) in ["size_x", "size_y", "size_z"].iter().zip(options.size) {
                if let Some(value) = value {
                    parameters.insert(*name, value);
                }
            }
            if options.radius.is_some() || options.height.is_some() {
                return Err("--radius/--height only apply to cylinder and sphere".to_owned());
            }
        }
        PrimitiveKind::Cylinder => {
            if let Some(radius) = options.radius {
                parameters.insert("radius", radius);
            }
            if let Some(height) = options.height {
                parameters.insert("height", height);
            }
            if options.size.iter().any(Option::is_some) {
                return Err("--size-* only applies to box".to_owned());
            }
        }
        PrimitiveKind::Sphere => {
            if let Some(radius) = options.radius {
                parameters.insert("radius", radius);
            }
            if options.height.is_some() || options.size.iter().any(Option::is_some) {
                return Err("--height/--size-* do not apply to sphere".to_owned());
            }
        }
    }

    if options.out != Path::new("-") && options.out.exists() && !options.force {
        return Err(format!(
            "refusing to overwrite {}; pass --force to replace it",
            options.out.display()
        ));
    }

    let started = Instant::now();
    let mut bytes = Vec::new();
    for _ in 0..options.repeat {
        bytes = export_parameters(kind, &parameters).map_err(|error| error.to_string())?;
    }
    let elapsed = started.elapsed();

    if bytes.len() > MAX_STEP_BYTES {
        return Err("the writer produced an oversized document".to_owned());
    }

    if options.out == Path::new("-") {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(&bytes)
            .and_then(|()| stdout.flush())
            .map_err(|error| format!("could not write to stdout: {error}"))?;
    } else {
        std::fs::write(&options.out, &bytes)
            .map_err(|error| format!("could not write {}: {error}", options.out.display()))?;
    }

    eprintln!(
        "kala-step: {} exact B-rep, {} bytes, {} export(s) in {:.3} ms",
        kind.as_str(),
        bytes.len(),
        options.repeat,
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}

fn parse_dimension(name: &str, text: &str) -> Result<f64, String> {
    text.parse::<f64>()
        .map_err(|_| format!("{name} must be a finite number, got {text}"))
}
