use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::process::{ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::commands::{self, Outcome, Session};
use crate::protocol::{self, FAILURE, SUCCESS, VERSION_TAG};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// `stax serve`: starts the daemon in the background and waits until it accepts connections.
pub fn serve() -> io::Result<ExitCode> {
    let socket = protocol::socket_path()?;
    if UnixStream::connect(&socket).is_ok() {
        println!("stax daemon already running ({})", socket.display());
        return Ok(ExitCode::SUCCESS);
    }
    protocol::create_private_dir(&protocol::state_dir()?)?;
    protocol::create_private_dir(&protocol::socket_dir()?)?;
    let log_path = protocol::log_path()?;
    let log = OpenOptions::new().create(true).append(true).open(&log_path)?;

    // In its own process group, so signals aimed at the shell's job (e.g. Ctrl-C) don't reach it.
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0)
        .spawn()?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if UnixStream::connect(&socket).is_ok() {
            println!("stax daemon started (pid {})", child.id());
            return Ok(ExitCode::SUCCESS);
        }
        if let Some(status) = child.try_wait()? {
            eprintln!("stax daemon exited during startup ({status}); see {}", log_path.display());
            return Ok(ExitCode::FAILURE);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    eprintln!("stax daemon did not start within {STARTUP_TIMEOUT:?}; see {}", log_path.display());
    Ok(ExitCode::FAILURE)
}

/// `stax __daemon`: serves requests until `stax shutdown`.
pub fn run() -> io::Result<ExitCode> {
    let socket = protocol::socket_path()?;
    protocol::create_private_dir(&protocol::socket_dir()?)?;
    if socket.exists() {
        if UnixStream::connect(&socket).is_ok() {
            eprintln!("stax daemon already running ({})", socket.display());
            return Ok(ExitCode::FAILURE);
        }
        // Left behind by a daemon that didn't shut down cleanly.
        std::fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    eprintln!("stax daemon {VERSION_TAG} started (pid {}, socket {})", std::process::id(), socket.display());

    let session = Arc::new(Mutex::new(Session::default()));
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let session = session.clone();
                let socket = socket.clone();
                std::thread::spawn(move || handle(stream, &session, &socket));
            }
            Err(e) => eprintln!("accept failed: {e}"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn handle(mut stream: UnixStream, session: &Mutex<Session>, socket: &Path) {
    let (version, args) = match protocol::read_request(&mut stream) {
        Ok(request) => request,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return,
        Err(e) => {
            eprintln!("bad request: {e}");
            return;
        }
    };
    // `shutdown` is accepted from any version, so a mismatched daemon can always be stopped.
    let (status, text, shutdown) = if version != VERSION_TAG && args != [OsString::from("shutdown")] {
        let text = format!(
            "the running daemon is {VERSION_TAG} but this client is {version}; restart it with `stax shutdown && stax serve`"
        );
        (FAILURE, text, false)
    } else {
        match Cli::try_parse_from(std::iter::once(OsString::from("stax")).chain(args)) {
            Ok(Cli { command: Command::Shutdown }) => (SUCCESS, "stax daemon stopped".to_string(), true),
            // A panic is reported to the client instead of dropping the connection; the panic
            // message goes to the daemon log.
            Ok(cli) => match std::panic::catch_unwind(AssertUnwindSafe(|| execute(cli.command, session))) {
                Ok(Ok(text)) => (SUCCESS, text, false),
                Ok(Err(text)) => (FAILURE, text, false),
                Err(_) => (FAILURE, "internal error in the stax daemon; see ~/.stax/daemon.log".to_string(), false),
            },
            Err(e) => (FAILURE, e.to_string(), false),
        }
    };
    if let Err(e) = protocol::write_response(&mut stream, status, &text) {
        eprintln!("failed to send response: {e}");
    }
    if shutdown {
        drop(stream);
        let _ = std::fs::remove_file(socket);
        eprintln!("stax daemon stopped");
        std::process::exit(0);
    }
}

/// Runs a query command against the session. The lock is held only to look up and register
/// graphs, so slow commands (parsing, large graph walks) don't block others.
fn execute(command: Command, session: &Mutex<Session>) -> Outcome {
    let lock = || session.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match command {
        Command::Load { paths } => {
            let mut graphs = Vec::new();
            for path in &paths {
                if !path.is_absolute() {
                    return Err(format!("{}: path is not absolute", path.display()));
                }
                graphs.extend(commands::parse_profile(path)?);
            }
            lock().load(graphs)
        }
        Command::Unload { graph } => lock().unload(graph),
        Command::List => lock().list(),
        Command::Search { graph, pattern, limit, offset, under } => {
            let (name, g) = lock().get(graph)?;
            commands::search(&name, &g, &pattern, limit, offset, under)
        }
        Command::Node { graph, node, parent_depth } => {
            let (name, g) = lock().get(graph)?;
            commands::node_details(&name, &g, node, parent_depth)
        }
        Command::Children { graph, node, limit } => {
            let (name, g) = lock().get(graph)?;
            commands::children(&name, &g, node, limit)
        }
        Command::Aggregate { graph, target } => {
            let (name, g) = lock().get(graph)?;
            let (new_name, merged, note) = commands::aggregate(&name, &g, &target)?;
            let node_count = merged.node_count();
            let id = lock().register(new_name.clone(), merged);
            Ok(commands::created("merged", id, &new_name, node_count, &note))
        }
        Command::Icicle { graph, target } => {
            let (name, g) = lock().get(graph)?;
            let (new_name, icicle) = commands::icicle(&name, &g, &target)?;
            let node_count = icicle.node_count();
            let id = lock().register(new_name.clone(), icicle);
            Ok(commands::created("icicle", id, &new_name, node_count, ""))
        }
        Command::RawName { graph, node } => {
            let (_, g) = lock().get(graph)?;
            commands::raw_name(&g, node)
        }
        Command::HotPath { graph, node, depth_limit, max_lines, no_collapse } => {
            let (name, g) = lock().get(graph)?;
            commands::hot_path(&name, &g, node, depth_limit, max_lines, !no_collapse)
        }
        Command::Subtree { graph, node, max_lines, no_collapse } => {
            let (name, g) = lock().get(graph)?;
            commands::subtree(&name, &g, node, max_lines, !no_collapse)
        }
        Command::Callers { graph, target, max_lines } => {
            let (name, g) = lock().get(graph)?;
            commands::callers(&name, &g, &target, max_lines)
        }
        Command::Serve | Command::Shutdown | Command::Daemon => Err("not a daemon command".to_string()),
    }
}
