//! argparse-compatible argument parsing for `kala-agent`.
//!
//! The Python command line declares its whole surface with `argparse`:
//! twelve subcommands, short and long option abbreviation, `--opt=value`,
//! negative-number values, append options, typed integers, choices, required
//! options and argparse's exact error wording and exit code 2. This module
//! reproduces that surface for the fixed definitions in
//! [`crate::help`], and nothing more: there is no third-party parser whose
//! defaults could drift from the Python command.

use std::collections::BTreeMap;

use kala_application::LOCAL_ACTOR_ID;
use num_bigint::BigInt;

use crate::help::{
    CommandSpec, OptionSpec, ROOT_PROG, command_help, command_usage, formatter_width, root_help,
    root_usage,
};

/// The `serve` bind address.
pub const SERVE_HOST: &str = "127.0.0.1";
/// The default `serve` port.
pub const SERVE_PORT: i64 = 8000;
/// The lowest accepted `serve` port.
pub const MIN_SERVE_PORT: i64 = 1;
/// The highest accepted `serve` port.
pub const MAX_SERVE_PORT: i64 = 65_535;
/// The default mechanical design identifier.
pub const DEFAULT_MECHANICAL_DESIGN_ID: &str = "mechanical-reference";
/// The default revision-commit decision.
pub const DEFAULT_REVISION_DECISION: &str = "approve";
/// The default journal page size.
pub const DEFAULT_EVENT_LIMIT: i64 = 50;
/// The largest journal page size.
pub const MAX_EVENT_LIMIT: i64 = 1_000;

const HELP: OptionSpec = crate::help::HELP_OPTION;

const HOST: OptionSpec = OptionSpec {
    strings: &["--host"],
    dest: "host",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Loopback address to bind, either 127.0.0.1 or ::1. Any other address, including a wildcard, is refused.",
};
const PORT: OptionSpec = OptionSpec {
    strings: &["--port"],
    dest: "port",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "TCP port to bind, 1 to 65535.",
};
const DOMAIN: OptionSpec = OptionSpec {
    strings: &["--domain"],
    dest: "domain",
    metavar: None,
    required: true,
    takes_value: true,
    choices: &[],
    help: "Registered domain package name.",
};
const DESIGN_ID: OptionSpec = OptionSpec {
    strings: &["--design-id"],
    dest: "design_id",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Design identifier to evaluate the reference parameters under.",
};
const PARAM: OptionSpec = OptionSpec {
    strings: &["--param"],
    dest: "param",
    metavar: Some("NAME=VALUE"),
    required: false,
    takes_value: true,
    choices: &[],
    help: "Parameter override, repeatable. Values are parsed as JSON when possible.",
};
const PARAMETERS_JSON: OptionSpec = OptionSpec {
    strings: &["--parameters-json"],
    dest: "parameters_json",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "A JSON object of parameter overrides.",
};
const RUN_DESIGN_ID: OptionSpec = OptionSpec {
    strings: &["--design-id"],
    dest: "design_id",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Design identifier the run belongs to.",
};
const RUN_WORKFLOW_ID: OptionSpec = OptionSpec {
    strings: &["--workflow-id"],
    dest: "workflow_id",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Workflow identifier. Defaults to the identifier derived from the design.",
};
const ACTOR: OptionSpec = OptionSpec {
    strings: &["--actor"],
    dest: "actor",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Local actor identifier recorded with the decision.",
};
const REASON: OptionSpec = OptionSpec {
    strings: &["--reason"],
    dest: "reason",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Optional operator reason recorded with the decision.",
};
const STATUS_WORKFLOW_ID: OptionSpec = OptionSpec {
    strings: &["--workflow-id"],
    dest: "workflow_id",
    metavar: None,
    required: true,
    takes_value: true,
    choices: &[],
    help: "Workflow identifier to inspect.",
};
const APPROVE_WORKFLOW_ID: OptionSpec = OptionSpec {
    strings: &["--workflow-id"],
    dest: "workflow_id",
    metavar: None,
    required: true,
    takes_value: true,
    choices: &[],
    help: "Workflow identifier whose checkpoint should be approved.",
};
const REJECT_WORKFLOW_ID: OptionSpec = OptionSpec {
    strings: &["--workflow-id"],
    dest: "workflow_id",
    metavar: None,
    required: true,
    takes_value: true,
    choices: &[],
    help: "Workflow identifier whose checkpoint should be rejected.",
};
const PREPARE_DESIGN_ID: OptionSpec = OptionSpec {
    strings: &["--design-id"],
    dest: "design_id",
    metavar: None,
    required: true,
    takes_value: true,
    choices: &[],
    help: "Stable design identifier the revision belongs to.",
};
const SUMMARY: OptionSpec = OptionSpec {
    strings: &["--summary"],
    dest: "summary",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Optional summary recorded with the revision specification.",
};
const PARENT_REVISION_ID: OptionSpec = OptionSpec {
    strings: &["--parent-revision-id"],
    dest: "parent_revision_id",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Revision this commit extends. Defaults to the current head of the design.",
};
const REQUEST_ID: OptionSpec = OptionSpec {
    strings: &["--request-id"],
    dest: "request_id",
    metavar: None,
    required: true,
    takes_value: true,
    choices: &[],
    help: "Identifier of the prepared revision commit.",
};
const DECISION: OptionSpec = OptionSpec {
    strings: &["--decision"],
    dest: "decision",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &["approve", "reject"],
    help: "Record an approval or a rejection against the prepared digests.",
};
const STATE_DIGEST: OptionSpec = OptionSpec {
    strings: &["--state-digest"],
    dest: "state_digest",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "State digest the operator reviewed. Refused when it is stale.",
};
const ACTION_DIGEST: OptionSpec = OptionSpec {
    strings: &["--action-digest"],
    dest: "action_digest",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Action digest the operator reviewed. Refused when it is stale.",
};
const AFTER_SEQUENCE: OptionSpec = OptionSpec {
    strings: &["--after-sequence"],
    dest: "after_sequence",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Return events after this sequence number.",
};
const LIMIT: OptionSpec = OptionSpec {
    strings: &["--limit"],
    dest: "limit",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Maximum number of events to return, 1 to 1000.",
};
const EVENT_WORKFLOW_ID: OptionSpec = OptionSpec {
    strings: &["--workflow-id"],
    dest: "workflow_id",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Restrict events to one workflow.",
};
const EVENT_TYPE: OptionSpec = OptionSpec {
    strings: &["--event-type"],
    dest: "event_type",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Restrict events to one event type.",
};
const AGGREGATE_ID: OptionSpec = OptionSpec {
    strings: &["--aggregate-id"],
    dest: "aggregate_id",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Restrict events to one aggregate.",
};

/// The declared subcommands, in declaration order.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "init",
        help: "Create or verify the local data directory and report local facts.",
        description: "Create the local data directory, database, and artifact store, then report provider, model, and schema facts without printing credentials.",
        options: &[],
    },
    CommandSpec {
        name: "domains",
        help: "List every registered domain package and workflow definition.",
        description: "List the registered domain packages, their digests, and workflows.",
        options: &[],
    },
    CommandSpec {
        name: "token",
        help: "Print the bearer token of the running local API launch.",
        description: "Read and print the bearer token the local API stored in the data directory when it started. This is the only place the token is printed; it is never logged, and no other command reports it.",
        options: &[],
    },
    CommandSpec {
        name: "serve",
        help: "Serve the local API on the loopback interface until interrupted.",
        description: "Mint a launch token, write it to the local data directory, and serve the local API on the loopback interface with the same Host guard and the same per-launch bearer token every other launch carries. The address is loopback only: a wildcard or routable bind is refused. Requires the optional uvicorn runtime dependency.",
        options: &[HOST, PORT],
    },
    CommandSpec {
        name: "evaluate",
        help: "Evaluate one reference design of a domain package.",
        description: "Run the deterministic reference evaluation of one domain package and print its bounded surrogate metrics and findings.",
        options: &[DOMAIN, DESIGN_ID, PARAM, PARAMETERS_JSON],
    },
    CommandSpec {
        name: "run-mechanical",
        help: "Run the mechanical reference workflow to its policy checkpoint.",
        description: "Run the mechanical reference workflow until it pauses at the declared pre-commit policy checkpoint, or until it is completed.",
        options: &[RUN_DESIGN_ID, RUN_WORKFLOW_ID, ACTOR, REASON],
    },
    CommandSpec {
        name: "workflow-status",
        help: "Print the current state of one workflow.",
        description: "Print the current state of one stored workflow run.",
        options: &[STATUS_WORKFLOW_ID],
    },
    CommandSpec {
        name: "approve",
        help: "Approve the pending policy checkpoint of a workflow.",
        description: "Approve the pending policy checkpoint, drive the workflow to completion, and commit the declared design revision.",
        options: &[APPROVE_WORKFLOW_ID, ACTOR, REASON],
    },
    CommandSpec {
        name: "reject",
        help: "Reject the pending policy checkpoint of a workflow.",
        description: "Reject the pending policy checkpoint. The workflow ends as failed and no revision is committed.",
        options: &[REJECT_WORKFLOW_ID, ACTOR, REASON],
    },
    CommandSpec {
        name: "prepare-revision",
        help: "Prepare one revision commit and open its pre-commit checkpoint.",
        description: "Derive one design revision commit, publish the digests its approval is bound to, and write nothing to the design. Follow it with commit-revision.",
        options: &[
            PREPARE_DESIGN_ID,
            DOMAIN,
            SUMMARY,
            PARENT_REVISION_ID,
            PARAM,
            PARAMETERS_JSON,
        ],
    },
    CommandSpec {
        name: "commit-revision",
        help: "Approve or reject one prepared revision commit.",
        description: "Record a decision for a prepared revision commit. An approval is honoured only against the digests the state carries now, so a design head that moved since the commit was prepared writes nothing. A rejection never writes.",
        options: &[
            REQUEST_ID,
            DECISION,
            STATE_DIGEST,
            ACTION_DIGEST,
            ACTOR,
            REASON,
        ],
    },
    CommandSpec {
        name: "events",
        help: "Print a bounded page of journal events.",
        description: "Print journal events in sequence order, newest page last.",
        options: &[
            AFTER_SEQUENCE,
            LIMIT,
            EVENT_WORKFLOW_ID,
            EVENT_TYPE,
            AGGREGATE_ID,
        ],
    },
];

/// One parsed command.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Create or verify the local data directory.
    Init,
    /// List the registered packages and workflows.
    Domains,
    /// Read the stored launch token.
    Token,
    /// Resolve the loopback launch (the native server entry point is absent).
    Serve {
        /// Requested bind host.
        host: String,
        /// Requested bind port.
        port: BigInt,
    },
    /// Evaluate one reference design.
    Evaluate {
        /// Domain package.
        domain: String,
        /// Optional design identifier.
        design_id: Option<String>,
        /// Repeatable name=value overrides.
        parameters: Vec<String>,
        /// A JSON object of overrides.
        parameters_json: Option<String>,
    },
    /// Run the mechanical reference workflow.
    RunMechanical {
        /// Design identifier.
        design_id: String,
        /// Workflow identifier.
        workflow_id: Option<String>,
        /// Actor recorded with any decision.
        actor: String,
        /// Optional reason.
        reason: Option<String>,
    },
    /// Print one workflow state.
    WorkflowStatus {
        /// Workflow identifier.
        workflow_id: String,
    },
    /// Approve a pending checkpoint.
    Approve {
        /// Workflow identifier.
        workflow_id: String,
        /// Actor recorded with the decision.
        actor: String,
        /// Optional reason.
        reason: Option<String>,
    },
    /// Reject a pending checkpoint.
    Reject {
        /// Workflow identifier.
        workflow_id: String,
        /// Actor recorded with the decision.
        actor: String,
        /// Optional reason.
        reason: Option<String>,
    },
    /// Prepare one revision commit.
    PrepareRevision {
        /// Design identifier.
        design_id: String,
        /// Domain package.
        domain: String,
        /// Optional summary.
        summary: Option<String>,
        /// Parent revision.
        parent_revision_id: Option<String>,
        /// Repeatable name=value overrides.
        parameters: Vec<String>,
        /// A JSON object of overrides.
        parameters_json: Option<String>,
    },
    /// Approve or reject one prepared revision commit.
    CommitRevision {
        /// Prepared commit identifier.
        request_id: String,
        /// `approve` or `reject`.
        decision: String,
        /// Reviewed state digest.
        state_digest: Option<String>,
        /// Reviewed action digest.
        action_digest: Option<String>,
        /// Deciding actor.
        actor: String,
        /// Optional reason.
        reason: Option<String>,
    },
    /// Print one page of journal events.
    Events {
        /// Exclusive lower sequence bound.
        after_sequence: BigInt,
        /// Page size.
        limit: BigInt,
        /// Workflow filter.
        workflow_id: Option<String>,
        /// Event-type filter.
        event_type: Option<String>,
        /// Aggregate filter.
        aggregate_id: Option<String>,
    },
}

/// A complete invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    /// The root `--data-dir`, when supplied.
    pub data_dir: Option<String>,
    /// The selected command.
    pub command: Command,
}

/// The outcome of parsing one argv.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Run this command.
    Run(Invocation),
    /// Print this help document to stdout and exit 0.
    Help(String),
    /// Print this usage and error to stderr and exit 2.
    Usage(UsageError),
}

/// One argparse usage failure.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageError {
    /// The parser that refused, as argparse names it.
    pub prog: String,
    /// The usage block.
    pub usage: String,
    /// The message after `error:`.
    pub message: String,
}

impl UsageError {
    /// The complete stderr text.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{}{}: error: {}\n", self.usage, self.prog, self.message)
    }
}

#[derive(Clone, Debug)]
enum ParsedValue {
    Text(String),
    List(Vec<String>),
}

/// Parse one argv (without the program name).
#[must_use]
pub fn parse(arguments: &[String]) -> Outcome {
    let width = formatter_width();
    let mut data_dir: Option<String> = None;
    let mut root_extras: Vec<String> = Vec::new();
    let mut index = 0usize;
    let mut options_ended = false;
    while index < arguments.len() {
        let token = &arguments[index];
        if !options_ended && token == "--" {
            options_ended = true;
            index += 1;
            continue;
        }
        if !options_ended && looks_like_option(token) {
            match classify_short(token) {
                Short::Help => return Outcome::Help(root_help(width)),
                Short::IgnoredExplicit(value) => {
                    return Outcome::Usage(UsageError {
                        prog: ROOT_PROG.to_owned(),
                        usage: root_usage(width),
                        message: format!("argument -h/--help: ignored explicit argument '{value}'"),
                    });
                }
                Short::Plain => {}
            }
            let (name, explicit) = split_explicit(token);
            match resolve(
                name,
                &[HELP.strings[1], crate::help::ROOT_OPTIONS[0].strings[0]],
            ) {
                Resolution::NotFound => {
                    root_extras.push(token.clone());
                    index += 1;
                }
                Resolution::Ambiguous(candidates) => {
                    return Outcome::Usage(UsageError {
                        prog: ROOT_PROG.to_owned(),
                        usage: root_usage(width),
                        message: format!(
                            "ambiguous option: {name} could match {}",
                            candidates.join(", ")
                        ),
                    });
                }
                Resolution::Found(found) => {
                    if found == HELP.strings[1] {
                        if let Some(value) = explicit {
                            return Outcome::Usage(UsageError {
                                prog: ROOT_PROG.to_owned(),
                                usage: root_usage(width),
                                message: format!(
                                    "argument -h/--help: ignored explicit argument '{value}'"
                                ),
                            });
                        }
                        return Outcome::Help(root_help(width));
                    }
                    let data_option = &crate::help::ROOT_OPTIONS[0];
                    if let Some(value) = explicit {
                        data_dir = Some(value.to_owned());
                        index += 1;
                    } else {
                        let Some(value) = arguments.get(index + 1) else {
                            return expected_argument(ROOT_PROG, root_usage(width), data_option);
                        };
                        if looks_like_option(value) {
                            return expected_argument(ROOT_PROG, root_usage(width), data_option);
                        }
                        data_dir = Some(value.clone());
                        index += 2;
                    }
                }
            }
            continue;
        }
        // The command name: everything after it belongs to the subparser.
        let Some(command) = COMMANDS.iter().find(|command| command.name == token) else {
            return invalid_choice(
                ROOT_PROG,
                root_usage(width),
                "command",
                token,
                &COMMANDS
                    .iter()
                    .map(|command| command.name)
                    .collect::<Vec<_>>(),
            );
        };
        return parse_command(
            command,
            data_dir,
            &arguments[index + 1..],
            root_extras,
            width,
        );
    }
    Outcome::Usage(UsageError {
        prog: ROOT_PROG.to_owned(),
        usage: root_usage(width),
        message: "the following arguments are required: command".to_owned(),
    })
}

enum Short {
    Help,
    IgnoredExplicit(String),
    Plain,
}

fn classify_short(token: &str) -> Short {
    if token.starts_with("--") || !token.starts_with('-') || token == "-" {
        return Short::Plain;
    }
    if token == "-h" {
        return Short::Help;
    }
    if let Some(rest) = token.strip_prefix("-h") {
        if let Some(explicit) = rest.strip_prefix('=') {
            return Short::IgnoredExplicit(explicit.to_owned());
        }
        return Short::Help;
    }
    Short::Plain
}

enum Resolution {
    NotFound,
    Ambiguous(Vec<String>),
    Found(&'static str),
}

fn resolve(candidate: &str, names: &[&'static str]) -> Resolution {
    if let Some(exact) = names.iter().find(|name| **name == candidate) {
        return Resolution::Found(exact);
    }
    let matches: Vec<&'static str> = names
        .iter()
        .copied()
        .filter(|name| name.starts_with(candidate))
        .collect();
    match matches.len() {
        0 => Resolution::NotFound,
        1 => Resolution::Found(matches[0]),
        _ => Resolution::Ambiguous(matches.into_iter().map(str::to_owned).collect()),
    }
}

/// Split `--name=value` into its name and explicit value.
fn split_explicit(token: &str) -> (&str, Option<&str>) {
    match token.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (token, None),
    }
}

fn looks_like_option(token: &str) -> bool {
    if token == "-" || !token.starts_with('-') {
        return false;
    }
    !is_negative_number(token)
}

fn is_negative_number(token: &str) -> bool {
    let Some(rest) = token.strip_prefix('-') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    // `^-\d+$|^-\d*\.\d+$`
    if rest.chars().all(|character| character.is_ascii_digit()) {
        return true;
    }
    let mut parts = rest.splitn(2, '.');
    let integer = parts.next().unwrap_or("");
    let Some(fraction) = parts.next() else {
        return false;
    };
    integer.chars().all(|character| character.is_ascii_digit())
        && !fraction.is_empty()
        && fraction.chars().all(|character| character.is_ascii_digit())
}

fn parse_command(
    command: &'static CommandSpec,
    data_dir: Option<String>,
    tokens: &[String],
    root_extras: Vec<String>,
    width: usize,
) -> Outcome {
    let prog = format!("{ROOT_PROG} {}", command.name);
    let usage = command_usage(command, width);
    let mut values: BTreeMap<&'static str, ParsedValue> = BTreeMap::new();
    let mut extras: Vec<String> = root_extras;
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        if token == "--" {
            extras.extend(tokens[index..].iter().cloned());
            break;
        }
        if looks_like_option(token) {
            match classify_short(token) {
                Short::Help => return Outcome::Help(command_help(command, width)),
                Short::IgnoredExplicit(value) => {
                    return Outcome::Usage(UsageError {
                        prog: prog.clone(),
                        usage,
                        message: format!("argument -h/--help: ignored explicit argument '{value}'"),
                    });
                }
                Short::Plain => {}
            }
            let (name, explicit) = split_explicit(token);
            let mut names: Vec<&'static str> = vec![HELP.strings[1]];
            names.extend(command.options.iter().map(|option| option.strings[0]));
            match resolve(name, &names) {
                Resolution::NotFound => {
                    extras.push(token.clone());
                    index += 1;
                }
                Resolution::Ambiguous(candidates) => {
                    return Outcome::Usage(UsageError {
                        prog,
                        usage,
                        message: format!(
                            "ambiguous option: {name} could match {}",
                            candidates.join(", ")
                        ),
                    });
                }
                Resolution::Found(found) => {
                    if found == HELP.strings[1] {
                        if let Some(value) = explicit {
                            return Outcome::Usage(UsageError {
                                prog,
                                usage,
                                message: format!(
                                    "argument -h/--help: ignored explicit argument '{value}'"
                                ),
                            });
                        }
                        return Outcome::Help(command_help(command, width));
                    }
                    let option = command
                        .options
                        .iter()
                        .find(|option| option.strings[0] == found)
                        .expect("resolved option is declared");
                    let raw = match explicit {
                        Some(value) => {
                            index += 1;
                            value.to_owned()
                        }
                        None => {
                            let Some(value) = tokens.get(index + 1) else {
                                return expected_argument(&prog, usage, option);
                            };
                            if looks_like_option(value) {
                                return expected_argument(&prog, usage, option);
                            }
                            index += 2;
                            value.clone()
                        }
                    };
                    let converted = convert(option, &raw, &prog, &usage);
                    let value = match converted {
                        Converted::Value(value) => value,
                        Converted::Failure(error) => return Outcome::Usage(error),
                    };
                    if option.dest == "param" {
                        match values
                            .entry(option.dest)
                            .or_insert_with(|| ParsedValue::List(Vec::new()))
                        {
                            ParsedValue::List(items) => items.push(raw),
                            ParsedValue::Text(_) => {
                                values.insert(option.dest, ParsedValue::List(vec![raw]));
                            }
                        }
                    } else {
                        values.insert(option.dest, value);
                    }
                }
            }
            continue;
        }
        extras.push(token.clone());
        index += 1;
    }

    if !extras.is_empty() {
        // Required options are still checked first, as argparse's
        // `parse_known_args` does.
        if let Some(missing) = missing_required(command, &values) {
            return Outcome::Usage(UsageError {
                prog,
                usage,
                message: format!("the following arguments are required: {missing}"),
            });
        }
        return Outcome::Usage(UsageError {
            prog: ROOT_PROG.to_owned(),
            usage: root_usage(width),
            message: format!("unrecognized arguments: {}", extras.join(" ")),
        });
    }
    if let Some(missing) = missing_required(command, &values) {
        return Outcome::Usage(UsageError {
            prog,
            usage,
            message: format!("the following arguments are required: {missing}"),
        });
    }
    match build_invocation(command, data_dir, &values) {
        Ok(invocation) => Outcome::Run(invocation),
        Err(error) => Outcome::Usage(error),
    }
}

enum Converted {
    Value(ParsedValue),
    Failure(UsageError),
}

fn convert(option: &OptionSpec, raw: &str, prog: &str, usage: &str) -> Converted {
    if !option.choices.is_empty() && !option.choices.contains(&raw) {
        return Converted::Failure(UsageError {
            prog: prog.to_owned(),
            usage: usage.to_owned(),
            message: format!(
                "argument {}: invalid choice: '{}' (choose from {})",
                option_label(option),
                raw,
                option
                    .choices
                    .iter()
                    .map(|choice| format!("'{choice}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    if matches!(option.dest, "port" | "after_sequence" | "limit") {
        return python_int(raw).map_or_else(
            || {
                Converted::Failure(UsageError {
                    prog: prog.to_owned(),
                    usage: usage.to_owned(),
                    message: format!(
                        "argument {}: invalid int value: '{}'",
                        option_label(option),
                        raw
                    ),
                })
            },
            |value| Converted::Value(ParsedValue::Text(value.to_string())),
        );
    }
    if option.dest == "param" {
        return Converted::Value(ParsedValue::Text(raw.to_owned()));
    }
    Converted::Value(ParsedValue::Text(raw.to_owned()))
}

/// CPython's `int(str)` for the grammar argparse accepts, as a decimal value.
fn python_int(raw: &str) -> Option<BigInt> {
    let trimmed = raw.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
    });
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    if digits.is_empty() {
        return None;
    }
    let mut normalized = String::new();
    let mut previous_underscore = false;
    let mut seen_digit = false;
    for character in digits.chars() {
        if character == '_' {
            if !seen_digit || previous_underscore {
                return None;
            }
            previous_underscore = true;
            continue;
        }
        if !character.is_ascii_digit() {
            return None;
        }
        previous_underscore = false;
        seen_digit = true;
        normalized.push(character);
    }
    if previous_underscore || !seen_digit {
        return None;
    }
    let value = BigInt::parse_bytes(normalized.as_bytes(), 10)?;
    Some(if negative { -value } else { value })
}

fn option_label(option: &OptionSpec) -> String {
    if option.strings.len() > 1 {
        option.strings.join("/")
    } else {
        option.strings[0].to_owned()
    }
}

fn expected_argument(prog: &str, usage: String, option: &OptionSpec) -> Outcome {
    Outcome::Usage(UsageError {
        prog: prog.to_owned(),
        usage,
        message: format!("argument {}: expected one argument", option_label(option)),
    })
}

fn invalid_choice(prog: &str, usage: String, dest: &str, value: &str, choices: &[&str]) -> Outcome {
    Outcome::Usage(UsageError {
        prog: prog.to_owned(),
        usage,
        message: format!(
            "argument {dest}: invalid choice: '{value}' (choose from {})",
            choices
                .iter()
                .map(|choice| format!("'{choice}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

fn missing_required(
    command: &CommandSpec,
    values: &BTreeMap<&'static str, ParsedValue>,
) -> Option<String> {
    let missing: Vec<String> = command
        .options
        .iter()
        .filter(|option| option.required && !values.contains_key(option.dest))
        .map(|option| option.strings[0].to_owned())
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(missing.join(", "))
    }
}

fn text(values: &BTreeMap<&'static str, ParsedValue>, name: &str) -> Option<String> {
    match values.get(name) {
        Some(ParsedValue::Text(value)) => Some(value.clone()),
        Some(ParsedValue::List(_)) | None => None,
    }
}

fn texts(values: &BTreeMap<&'static str, ParsedValue>, name: &str) -> Vec<String> {
    match values.get(name) {
        Some(ParsedValue::List(items)) => items.clone(),
        Some(ParsedValue::Text(value)) => vec![value.clone()],
        None => Vec::new(),
    }
}

fn integer(values: &BTreeMap<&'static str, ParsedValue>, name: &str) -> Option<BigInt> {
    text(values, name).and_then(|value| BigInt::parse_bytes(value.as_bytes(), 10))
}

fn build_invocation(
    command: &CommandSpec,
    data_dir: Option<String>,
    values: &BTreeMap<&'static str, ParsedValue>,
) -> Result<Invocation, UsageError> {
    let command = match command.name {
        "init" => Command::Init,
        "domains" => Command::Domains,
        "token" => Command::Token,
        "serve" => Command::Serve {
            host: text(values, "host").unwrap_or_else(|| SERVE_HOST.to_owned()),
            port: integer(values, "port").unwrap_or_else(|| BigInt::from(SERVE_PORT)),
        },
        "evaluate" => Command::Evaluate {
            domain: text(values, "domain").unwrap_or_default(),
            design_id: text(values, "design_id"),
            parameters: texts(values, "param"),
            parameters_json: text(values, "parameters_json"),
        },
        "run-mechanical" => Command::RunMechanical {
            design_id: text(values, "design_id")
                .unwrap_or_else(|| DEFAULT_MECHANICAL_DESIGN_ID.to_owned()),
            workflow_id: text(values, "workflow_id"),
            actor: text(values, "actor").unwrap_or_else(|| LOCAL_ACTOR_ID.to_owned()),
            reason: text(values, "reason"),
        },
        "workflow-status" => Command::WorkflowStatus {
            workflow_id: text(values, "workflow_id").unwrap_or_default(),
        },
        "approve" => Command::Approve {
            workflow_id: text(values, "workflow_id").unwrap_or_default(),
            actor: text(values, "actor").unwrap_or_else(|| LOCAL_ACTOR_ID.to_owned()),
            reason: text(values, "reason"),
        },
        "reject" => Command::Reject {
            workflow_id: text(values, "workflow_id").unwrap_or_default(),
            actor: text(values, "actor").unwrap_or_else(|| LOCAL_ACTOR_ID.to_owned()),
            reason: text(values, "reason"),
        },
        "prepare-revision" => Command::PrepareRevision {
            design_id: text(values, "design_id").unwrap_or_default(),
            domain: text(values, "domain").unwrap_or_default(),
            summary: text(values, "summary"),
            parent_revision_id: text(values, "parent_revision_id"),
            parameters: texts(values, "param"),
            parameters_json: text(values, "parameters_json"),
        },
        "commit-revision" => Command::CommitRevision {
            request_id: text(values, "request_id").unwrap_or_default(),
            decision: text(values, "decision")
                .unwrap_or_else(|| DEFAULT_REVISION_DECISION.to_owned()),
            state_digest: text(values, "state_digest"),
            action_digest: text(values, "action_digest"),
            actor: text(values, "actor").unwrap_or_else(|| LOCAL_ACTOR_ID.to_owned()),
            reason: text(values, "reason"),
        },
        "events" => Command::Events {
            after_sequence: integer(values, "after_sequence").unwrap_or_default(),
            limit: integer(values, "limit").unwrap_or_else(|| BigInt::from(DEFAULT_EVENT_LIMIT)),
            workflow_id: text(values, "workflow_id"),
            event_type: text(values, "event_type"),
            aggregate_id: text(values, "aggregate_id"),
        },
        other => unreachable!("declared command {other}"),
    };
    Ok(Invocation { data_dir, command })
}
