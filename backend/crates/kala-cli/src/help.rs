//! argparse-compatible help, usage and line wrapping.
//!
//! The Python command line is built with `argparse`, whose help layout is part
//! of the command's observable output. This module reproduces the CPython
//! 3.14 `HelpFormatter` for the fixed action set of `kala-agent`: the usage
//! line's wrapping, the two-column action layout, `textwrap.wrap`'s greedy
//! filling (including the break-after-hyphen rule) and the blank-line
//! normalization. The terminal width is `COLUMNS` when set to a positive
//! integer, otherwise 80, minus two columns, exactly as
//! `shutil.get_terminal_size()` behaves for a redirected stream.

/// One option as argparse sees it.
#[derive(Clone, Copy, Debug)]
pub struct OptionSpec {
    /// Option strings in declaration order (`-h`, `--help`).
    pub strings: &'static [&'static str],
    /// Destination name.
    pub dest: &'static str,
    /// Explicit metavar, when the source declares one.
    pub metavar: Option<&'static str>,
    /// Whether argparse requires the option.
    pub required: bool,
    /// Whether the option consumes a value.
    pub takes_value: bool,
    /// Declared choices, when the option is a choice.
    pub choices: &'static [&'static str],
    /// One-line help.
    pub help: &'static str,
}

/// One subcommand as argparse sees it.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    /// Command name.
    pub name: &'static str,
    /// One-line help shown in the parent's positional list.
    pub help: &'static str,
    /// The long description.
    pub description: &'static str,
    /// Declared options, excluding the implicit `-h/--help`.
    pub options: &'static [OptionSpec],
}

pub const ROOT_PROG: &str = "kala-agent";
pub const ROOT_DESCRIPTION: &str = "Local, offline engineering design agent over the declarative domain packages. Every command prints one JSON document and returns an exit code. No authentication, no network access.";
pub const EPILOG: &str =
    "Exit codes: 0 completed, 1 error, 2 usage, 3 completed with a failed outcome.";
pub const HELP_OPTION: OptionSpec = OptionSpec {
    strings: &["-h", "--help"],
    dest: "help",
    metavar: None,
    required: false,
    takes_value: false,
    choices: &[],
    help: "show this help message and exit",
};

/// The formatter width: terminal columns minus two, or 78.
#[must_use]
pub fn formatter_width() -> usize {
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(80);
    usize::try_from(columns).unwrap_or(80).saturating_sub(2)
}

/// Render the root help document.
#[must_use]
pub fn root_help(width: usize) -> String {
    let usage = root_usage_block(width);
    let description = fill(ROOT_DESCRIPTION, width);
    let actions: Vec<Rendered> = vec![Rendered::positional_command()];
    let action_max = [
        invocation_len("command") + 2,
        crate::parser::COMMANDS
            .iter()
            .map(|command| command.name.chars().count() + 4)
            .max()
            .unwrap_or(0),
        invocation_len("--data-dir DATA_DIR") + 2,
        invocation_len("-h, --help") + 2,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let mut positional = String::new();
    for action in &actions {
        positional.push_str(&render_action(action, 2, action_max, width));
    }
    let mut options = String::new();
    options.push_str(&render_action(
        &Rendered::option(&HELP_OPTION),
        2,
        action_max,
        width,
    ));
    for option in ROOT_OPTIONS {
        options.push_str(&render_action(
            &Rendered::option(option),
            2,
            action_max,
            width,
        ));
    }
    let body = format!(
        "{usage}{description}\npositional arguments:\n{positional}\noptions:\n{options}\n{epilog}\n\n",
        epilog = fill(EPILOG, width),
    );
    normalize(&body)
}

/// Render one subcommand's help document.
#[must_use]
pub fn command_help(command: &CommandSpec, width: usize) -> String {
    let usage = command_usage_block(command, width);
    let description = fill(command.description, width);
    let action_max = command
        .options
        .iter()
        .map(|option| invocation_len(&option_invocation(option)) + 2)
        .chain(std::iter::once(invocation_len("-h, --help") + 2))
        .max()
        .unwrap_or(0);
    let mut options = String::new();
    options.push_str(&render_action(
        &Rendered::option(&HELP_OPTION),
        2,
        action_max,
        width,
    ));
    for option in command.options {
        options.push_str(&render_action(
            &Rendered::option(option),
            2,
            action_max,
            width,
        ));
    }
    let body = format!("{usage}{description}\noptions:\n{options}\n");
    normalize(&body)
}

/// Render the root usage block, as `print_usage` writes it.
#[must_use]
pub fn root_usage(width: usize) -> String {
    trim_to_one(root_usage_block(width))
}

/// Render one subcommand's usage block.
#[must_use]
pub fn command_usage(command: &CommandSpec, width: usize) -> String {
    trim_to_one(command_usage_block(command, width))
}

/// The options the root parser declares, excluding the implicit help action.
pub const ROOT_OPTIONS: &[OptionSpec] = &[OptionSpec {
    strings: &["--data-dir"],
    dest: "data_dir",
    metavar: None,
    required: false,
    takes_value: true,
    choices: &[],
    help: "Local data directory, absolute. Overrides KALA_AGENT_DATA_DIR for this invocation only.",
}];

fn root_usage_block(width: usize) -> String {
    let mut parts = vec![usage_option(&HELP_OPTION)];
    for option in ROOT_OPTIONS {
        parts.push(usage_option(option));
    }
    let positional = "command ...".to_owned();
    format_usage_block(ROOT_PROG, &parts, &[positional], width)
}

fn command_usage_block(command: &CommandSpec, width: usize) -> String {
    let mut parts = vec![usage_option(&HELP_OPTION)];
    for option in command.options {
        parts.push(usage_option(option));
    }
    let prog = format!("{ROOT_PROG} {}", command.name);
    format_usage_block(&prog, &parts, &[], width)
}

fn format_usage_block(
    prog: &str,
    optionals: &[String],
    positionals: &[String],
    width: usize,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(optionals.iter().cloned());
    parts.extend(positionals.iter().cloned());
    let mut usage = if parts.is_empty() {
        prog.to_owned()
    } else {
        format!("{prog} {}", parts.join(" "))
    };
    let prefix = "usage: ";
    let text_width = width;
    if prefix.chars().count() + usage.chars().count() > text_width {
        let pos_start = optionals.len();
        let opt_parts = &parts[..pos_start];
        let pos_parts = &parts[pos_start..];
        let prog_len = prog.chars().count();
        let mut lines: Vec<String>;
        if prefix.chars().count() + prog_len <= (text_width * 3) / 4 {
            let indent = " ".repeat(prefix.chars().count() + prog_len + 1);
            if !opt_parts.is_empty() {
                let mut with_prog = Vec::with_capacity(opt_parts.len() + 1);
                with_prog.push(prog.to_owned());
                with_prog.extend(opt_parts.iter().cloned());
                lines = get_lines(&with_prog, &indent, Some(prefix), text_width);
                lines.extend(get_lines(pos_parts, &indent, None, text_width));
            } else if !pos_parts.is_empty() {
                let mut with_prog = Vec::with_capacity(pos_parts.len() + 1);
                with_prog.push(prog.to_owned());
                with_prog.extend(pos_parts.iter().cloned());
                lines = get_lines(&with_prog, &indent, Some(prefix), text_width);
            } else {
                lines = vec![prog.to_owned()];
            }
        } else {
            let indent = " ".repeat(prefix.chars().count());
            let mut all: Vec<String> = Vec::new();
            all.extend(opt_parts.iter().cloned());
            all.extend(pos_parts.iter().cloned());
            lines = get_lines(&all, &indent, None, text_width);
            if lines.len() > 1 {
                lines = get_lines(opt_parts, &indent, None, text_width);
                lines.extend(get_lines(pos_parts, &indent, None, text_width));
            }
            lines.insert(0, prog.to_owned());
        }
        usage = lines.join("\n");
    }
    format!("{prefix}{usage}\n\n")
}

fn get_lines(
    parts: &[String],
    indent: &str,
    first_prefix: Option<&str>,
    text_width: usize,
) -> Vec<String> {
    let indent_length = indent.chars().count();
    let mut lines: Vec<String> = Vec::new();
    let mut line: Vec<&str> = Vec::new();
    let mut line_len = match first_prefix {
        Some(prefix) => prefix.chars().count().saturating_sub(1),
        None => indent_length.saturating_sub(1),
    };
    for part in parts {
        let part_len = part.chars().count();
        if line_len + 1 + part_len > text_width && !line.is_empty() {
            lines.push(format!("{indent}{}", line.join(" ")));
            line.clear();
            line_len = indent_length.saturating_sub(1);
        }
        line.push(part);
        line_len += part_len + 1;
    }
    if !line.is_empty() {
        lines.push(format!("{indent}{}", line.join(" ")));
    }
    if first_prefix.is_some() {
        if let Some(first) = lines.first_mut() {
            *first = first.chars().skip(indent_length).collect();
        }
    }
    lines
}

fn usage_option(option: &OptionSpec) -> String {
    let mut part = if option.takes_value {
        format!("{} {}", option.strings[0], option_args(option, true))
    } else {
        option.strings[0].to_owned()
    };
    if !option.required {
        part = format!("[{part}]");
    }
    part
}

fn option_args(option: &OptionSpec, optional: bool) -> String {
    if !option.choices.is_empty() {
        return format!("{{{}}}", option.choices.join(","));
    }
    if let Some(metavar) = option.metavar {
        return metavar.to_owned();
    }
    if optional {
        option.dest.to_uppercase()
    } else {
        option.dest.to_owned()
    }
}

fn option_invocation(option: &OptionSpec) -> String {
    let names = option.strings.join(", ");
    if option.takes_value {
        format!("{names} {}", option_args(option, true))
    } else {
        names
    }
}

fn invocation_len(invocation: &str) -> usize {
    invocation.chars().count()
}

struct Rendered {
    invocation: String,
    help: &'static str,
    subactions: Vec<(String, &'static str)>,
}

impl Rendered {
    fn positional_command() -> Self {
        Self {
            invocation: "command".to_owned(),
            help: "",
            subactions: crate::parser::COMMANDS
                .iter()
                .map(|command| (command.name.to_owned(), command.help))
                .collect(),
        }
    }

    fn option(option: &OptionSpec) -> Self {
        Self {
            invocation: option_invocation(option),
            help: option.help,
            subactions: Vec::new(),
        }
    }
}

fn max_help_position(width: usize) -> usize {
    width.saturating_sub(20).clamp(4, 24)
}

fn render_action(action: &Rendered, indent: usize, action_max: usize, width: usize) -> String {
    let help_position = std::cmp::min(action_max + 2, max_help_position(width));
    let help_width = std::cmp::max(width.saturating_sub(help_position), 11);
    let action_width = help_position.saturating_sub(indent + 2);
    let mut out = String::new();
    let invocation_len = action.invocation.chars().count();
    if action.help.is_empty() {
        out.push_str(&" ".repeat(indent));
        out.push_str(&action.invocation);
        out.push('\n');
    } else if invocation_len <= action_width {
        let help_lines = wrap(action.help, help_width);
        out.push_str(&" ".repeat(indent));
        out.push_str(&action.invocation);
        out.push_str(&" ".repeat(action_width - invocation_len + 2));
        out.push_str(help_lines.first().map_or("", String::as_str));
        out.push('\n');
        for line in help_lines.iter().skip(1) {
            out.push_str(&" ".repeat(help_position));
            out.push_str(line);
            out.push('\n');
        }
    } else {
        let help_lines = wrap(action.help, help_width);
        out.push_str(&" ".repeat(indent));
        out.push_str(&action.invocation);
        out.push('\n');
        for line in &help_lines {
            out.push_str(&" ".repeat(help_position));
            out.push_str(line);
            out.push('\n');
        }
    }
    for (invocation, help) in &action.subactions {
        let sub = Rendered {
            invocation: invocation.clone(),
            help,
            subactions: Vec::new(),
        };
        out.push_str(&render_action(&sub, indent + 2, action_max, width));
    }
    out
}

fn fill(text: &str, width: usize) -> String {
    let normalized = normalize_space(text);
    let lines = wrap(&normalized, width);
    format!("{}\n\n", lines.join("\n"))
}

fn normalize_space(text: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(character);
            in_space = false;
        }
    }
    out.trim().to_owned()
}

fn normalize(body: &str) -> String {
    let mut collapsed = String::new();
    let mut newlines = 0usize;
    for character in body.chars() {
        if character == '\n' {
            newlines += 1;
            if newlines <= 2 {
                collapsed.push('\n');
            }
        } else {
            newlines = 0;
            collapsed.push(character);
        }
    }
    format!("{}\n", collapsed.trim_matches('\n'))
}

fn trim_to_one(block: String) -> String {
    format!("{}\n", block.trim_end_matches('\n'))
}

// ---------------------------------------------------------------------------
// textwrap.wrap (break_on_hyphens = true, break_long_words = true)
// ---------------------------------------------------------------------------

fn wrap(text: &str, width: usize) -> Vec<String> {
    let chunks = split_chunks(text);
    wrap_chunks(chunks, width)
}

fn split_chunks(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut index = 0usize;
    while index < characters.len() {
        if is_space(characters[index]) {
            let start = index;
            while index < characters.len() && is_space(characters[index]) {
                index += 1;
            }
            chunks.push(characters[start..index].iter().collect());
            continue;
        }
        let start = index;
        while index < characters.len() && !is_space(characters[index]) {
            if characters[index] == '-'
                && index + 1 < characters.len()
                && hyphen_break(&characters, index)
            {
                index += 1;
                break;
            }
            index += 1;
        }
        chunks.push(characters[start..index].iter().collect());
    }
    chunks
}

fn is_space(character: char) -> bool {
    character.is_whitespace()
}

fn is_letter(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

/// The `break_on_hyphens` rule: split after `-` when the characters before it
/// are `letter letter` (or `letter - letter -`) and the characters after it
/// are `letter (- letter)?`.
fn hyphen_break(characters: &[char], index: usize) -> bool {
    let before = index >= 2 && is_letter(characters[index - 1]) && is_letter(characters[index - 2]);
    let chain = index >= 3
        && is_letter(characters[index - 1])
        && characters[index - 2] == '-'
        && is_letter(characters[index - 3]);
    if !(before || chain) {
        return false;
    }
    let next = index + 1;
    if next >= characters.len() || !is_letter(characters[next]) {
        return false;
    }
    match characters.get(next + 1) {
        None => false,
        Some('-') => characters
            .get(next + 2)
            .is_some_and(|value| is_letter(*value)),
        Some(value) => is_letter(*value),
    }
}

fn wrap_chunks(mut chunks: Vec<String>, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    chunks.reverse();
    while !chunks.is_empty() {
        let mut current: Vec<String> = Vec::new();
        let mut current_len = 0usize;
        if chunks.last().is_some_and(|chunk| chunk.trim().is_empty()) && !lines.is_empty() {
            chunks.pop();
        }
        while let Some(chunk) = chunks.last() {
            let length = chunk.chars().count();
            if current_len + length <= width {
                current.push(chunks.pop().expect("checked"));
                current_len += length;
            } else {
                break;
            }
        }
        if let Some(chunk) = chunks.last() {
            if chunk.chars().count() > width {
                handle_long_word(&mut chunks, &mut current, current_len, width);
            }
        }
        if current.last().is_some_and(|chunk| chunk.trim().is_empty()) {
            current.pop();
        }
        if !current.is_empty() {
            lines.push(current.concat());
        }
    }
    lines
}

fn handle_long_word(
    chunks: &mut Vec<String>,
    current: &mut Vec<String>,
    current_len: usize,
    width: usize,
) {
    let space_left = if width < 1 {
        1
    } else {
        width.saturating_sub(current_len)
    };
    if space_left == 0 {
        if current.is_empty() {
            if let Some(chunk) = chunks.pop() {
                current.push(chunk);
            }
        }
        return;
    }
    let Some(chunk) = chunks.pop() else {
        return;
    };
    let characters: Vec<char> = chunk.chars().collect();
    let mut end = space_left.min(characters.len());
    if characters.len() > space_left {
        if let Some(hyphen) = characters[..space_left]
            .iter()
            .rposition(|character| *character == '-')
        {
            if hyphen > 0 && characters[..hyphen].iter().any(|value| *value != '-') {
                end = hyphen + 1;
            }
        }
    }
    current.push(characters[..end].iter().collect());
    if end < characters.len() {
        chunks.push(characters[end..].iter().collect());
    }
}
