//! Native process fixture used by `kala-process` integration tests.
//!
//! This is a test fixture, not a product entry point. It deliberately uses only
//! direct argv parsing so tests never depend on a shell.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 2 {
        std::process::exit(2);
    }
    let result = match arguments[1].as_str() {
        "echo" => {
            let text = arguments.get(2).cloned().unwrap_or_default();
            write_stdout(text.as_bytes());
            0
        }
        "arg-echo" => {
            let joined = arguments[2..].join("\u{1f}");
            write_stdout(joined.as_bytes());
            0
        }
        "echo-stdin" => {
            let mut payload = Vec::new();
            if std::io::stdin().read_to_end(&mut payload).is_err() {
                exit(3);
            }
            write_stdout(&payload);
            0
        }
        "stdin-hold" => {
            let milliseconds = parse_millis(arguments.get(2));
            std::thread::sleep(Duration::from_millis(milliseconds));
            0
        }
        "exit" => arguments
            .get(2)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0),
        "self-signal" => {
            #[cfg(unix)]
            {
                let signal = arguments
                    .get(2)
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(9);
                // SAFETY: sending a signal to our own process is safe.
                unsafe {
                    libc::kill(libc::getpid(), signal);
                }
                // If the signal is blocked or ignored, do not report success.
                std::thread::sleep(Duration::from_millis(100));
                125
            }
            #[cfg(not(unix))]
            {
                125
            }
        }
        "sleep" => {
            std::thread::sleep(Duration::from_millis(parse_millis(arguments.get(2))));
            0
        }
        "sleep-then-write" => {
            let milliseconds = parse_millis(arguments.get(2));
            let path = arguments.get(3).cloned().unwrap_or_default();
            std::thread::sleep(Duration::from_millis(milliseconds));
            if std::fs::write(&path, b"late side effect").is_err() {
                exit(5);
            }
            0
        }
        "spawn-holder" => {
            let delay = parse_millis(arguments.get(2));
            let marker = arguments.get(3).cloned().unwrap_or_default();
            spawn_holder(delay, &marker);
            std::thread::sleep(Duration::from_millis(parse_millis(arguments.get(4))));
            0
        }
        "linger" => {
            let delay = parse_millis(arguments.get(2));
            let marker = arguments.get(3).cloned().unwrap_or_default();
            spawn_holder(delay, &marker);
            0
        }
        "escapee" => {
            let delay = parse_millis(arguments.get(2));
            let marker = arguments.get(3).cloned().unwrap_or_default();
            spawn_escapee(delay, &marker);
            0
        }
        "escapee-flood" => {
            spawn_escapee_flood();
            0
        }
        "flood" => {
            let bytes = arguments
                .get(2)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1_048_576);
            let stream = arguments.get(3).map(String::as_str).unwrap_or("stdout");
            let chunk = vec![b'x'; 16 * 1024];
            let mut written = 0usize;
            while written < bytes {
                let amount = chunk.len().min(bytes - written);
                if matches!(stream, "stdout" | "both")
                    && std::io::stdout().write_all(&chunk[..amount]).is_err()
                {
                    break;
                }
                if matches!(stream, "stderr" | "both")
                    && std::io::stderr().write_all(&chunk[..amount]).is_err()
                {
                    break;
                }
                written += amount;
            }
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            0
        }
        "flood-forever" => {
            // Used to prove the supervisor revisits its deadline/cancellation
            // decisions while a producer is always ready.
            let chunk = [b'x'; 16 * 1024];
            loop {
                if std::io::stdout().write_all(&chunk).is_err() {
                    break;
                }
            }
            let _ = std::io::stdout().flush();
            0
        }
        "invalid-utf8" => {
            write_stdout(&[0xff, 0xfe, 0x00, b'b', b'a', b'd']);
            0
        }
        "binary-stderr" => {
            let _ = std::io::stderr().write_all(b"\x00\x01binary\xff");
            let _ = std::io::stderr().flush();
            0
        }
        "stderr-lines" => {
            let workspace = arguments.get(2).cloned().unwrap_or_default();
            let payload = format!("first line\nmiddle line\nlast line at {workspace}/mod.py\n");
            let _ = std::io::stderr().write_all(payload.as_bytes());
            let _ = std::io::stderr().flush();
            0
        }
        "env" => {
            let mut entries: Vec<String> = std::env::vars()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            entries.sort();
            write_stdout(entries.join("\n").as_bytes());
            0
        }
        "cwd" => match std::env::current_dir() {
            Ok(path) => {
                write_stdout(path.to_string_lossy().as_bytes());
                0
            }
            Err(_) => 6,
        },
        "write" => {
            let path = arguments.get(2).cloned().unwrap_or_default();
            let text = arguments.get(3).cloned().unwrap_or_default();
            match std::fs::write(path, text.as_bytes()) {
                Ok(()) => 0,
                Err(_) => 7,
            }
        }
        "read" => {
            let path = arguments.get(2).cloned().unwrap_or_default();
            match std::fs::read(path) {
                Ok(payload) => {
                    write_stdout(&payload);
                    0
                }
                Err(_) => 42,
            }
        }
        "connect" => match std::net::TcpStream::connect_timeout(
            &"8.8.8.8:53".parse().expect("static address"),
            Duration::from_millis(500),
        ) {
            Ok(_) => 0,
            Err(_) => 43,
        },
        "hold-stdout-and-sleep" => {
            let milliseconds = parse_millis(arguments.get(2));
            write_stdout(b"partial");
            std::thread::sleep(Duration::from_millis(milliseconds));
            0
        }
        _ => 2,
    };
    exit(result);
}

fn write_stdout(payload: &[u8]) {
    let _ = std::io::stdout().write_all(payload);
    let _ = std::io::stdout().flush();
}

fn parse_millis(value: Option<&String>) -> u64 {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000)
}

fn spawn_escapee(delay: u64, marker: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let executable = std::env::current_exe().expect("current executable");
        let mut command = Command::new(executable);
        command
            .arg("sleep-then-write")
            .arg(delay.to_string())
            .arg(marker)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        command.process_group(0);
        let _ = command.spawn();
    }
    #[cfg(not(unix))]
    {
        let _ = (delay, marker);
    }
}

fn spawn_escapee_flood() {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let executable = std::env::current_exe().expect("current executable");
        let mut command = Command::new(executable);
        command
            .arg("flood-forever")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        command.process_group(0);
        let _ = command.spawn();
    }
}

fn spawn_holder(delay: u64, marker: &str) {
    let executable = std::env::current_exe().expect("current executable");
    let mut command = Command::new(executable);
    command
        .arg("sleep-then-write")
        .arg(delay.to_string())
        .arg(marker)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let _ = command.spawn();
}

fn exit(code: i32) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code)
}

#[allow(dead_code)]
fn unused_path_reference(_: &Path) {}
