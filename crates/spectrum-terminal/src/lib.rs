//! App-neutral pseudo-terminal transport for Spectrum clients.
//!
//! Applications provide a working directory and environment explicitly. The
//! transport never interpolates project values into shell source, so document
//! names and paths remain data even when they contain shell metacharacters.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const OUTPUT_CHUNK_BYTES: usize = 8 * 1024;
const OUTPUT_QUEUE_CHUNKS: usize = 128;
const POLL_BYTE_BUDGET: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl TerminalSize {
    pub const fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }

    fn pty(self) -> PtySize {
        PtySize {
            rows: self.rows.max(1),
            cols: self.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self::new(24, 100)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalContext {
    working_directory: PathBuf,
    environment: BTreeMap<OsString, OsString>,
}

impl TerminalContext {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            working_directory: working_directory.into(),
            environment: BTreeMap::new(),
        }
    }

    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    pub fn with_cli_directory(mut self, directory: &Path) -> Self {
        self.environment
            .insert("SPECTRUM_CLI_DIR".into(), directory.as_os_str().into());
        let paths = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default();
        let joined = std::env::join_paths(std::iter::once(directory.to_owned()).chain(paths));
        if let Ok(path) = joined {
            self.environment.insert("PATH".into(), path);
        }
        self
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn environment(&self, key: impl AsRef<OsStr>) -> Option<&OsStr> {
        self.environment.get(key.as_ref()).map(OsString::as_os_str)
    }

    fn command(&self) -> CommandBuilder {
        let mut command = shell_command();
        command.cwd(&self.working_directory);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        for (key, value) in &self.environment {
            command.env(key, value);
        }
        command
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalExit {
    pub code: u32,
    pub signal: Option<String>,
}

impl TerminalExit {
    pub fn success(&self) -> bool {
        self.signal.is_none() && self.code == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    Output(Vec<u8>),
    Exited(TerminalExit),
    Error(String),
}

/// The transport contract used by a byte-stream terminal client.
///
/// Spectrum's portable terminal renderer consumes [`TerminalEvent::Output`]
/// with a VT parser. Platform-native terminal surfaces (for example, a
/// Ghostty-owned AppKit view) should remain in the owning application instead
/// of forcing native rendering concerns through this app-neutral contract.
pub trait TerminalSessionBackend: Send {
    fn context(&self) -> &TerminalContext;
    fn process_id(&self) -> Option<u32>;
    fn write(&mut self, bytes: &[u8]) -> Result<()>;
    fn resize(&self, size: TerminalSize) -> Result<()>;
    fn poll(&mut self) -> Vec<TerminalEvent>;
    fn is_running(&mut self) -> bool;
    fn terminate(&mut self) -> Result<()>;
}

/// A terminal session presented through Spectrum's portable byte-stream API.
///
/// [`TerminalSession::spawn`] deliberately keeps the existing portable PTY
/// backend as the default on every platform. Applications can inject another
/// byte-stream implementation with [`TerminalSession::from_backend`], while a
/// native rendered surface remains a separate, platform-gated UI concern.
pub struct TerminalSession {
    backend: Box<dyn TerminalSessionBackend>,
}

impl TerminalSession {
    pub fn spawn(context: TerminalContext, size: TerminalSize) -> Result<Self> {
        PortablePtySession::spawn(context, size).map(Self::from_backend)
    }

    pub fn from_backend(backend: impl TerminalSessionBackend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
        }
    }

    pub fn context(&self) -> &TerminalContext {
        self.backend.context()
    }

    pub fn process_id(&self) -> Option<u32> {
        self.backend.process_id()
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.backend.write(bytes)
    }

    pub fn resize(&self, size: TerminalSize) -> Result<()> {
        self.backend.resize(size)
    }

    pub fn poll(&mut self) -> Vec<TerminalEvent> {
        self.backend.poll()
    }

    pub fn is_running(&mut self) -> bool {
        self.backend.is_running()
    }

    pub fn terminate(&mut self) -> Result<()> {
        self.backend.terminate()
    }
}

struct PortablePtySession {
    context: TerminalContext,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<TerminalEvent>,
    exit_reported: bool,
}

impl PortablePtySession {
    fn spawn(context: TerminalContext, size: TerminalSize) -> Result<Self> {
        if !context.working_directory().is_dir() {
            bail!(
                "terminal working directory does not exist: {}",
                context.working_directory().display()
            );
        }
        let pair = native_pty_system()
            .openpty(size.pty())
            .context("could not open a pseudo-terminal")?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("could not read from the pseudo-terminal")?;
        let writer = pair
            .master
            .take_writer()
            .context("could not write to the pseudo-terminal")?;
        let child = pair
            .slave
            .spawn_command(context.command())
            .context("could not start the terminal shell")?;
        drop(pair.slave);

        // A bounded queue intentionally applies PTY backpressure when a client
        // is hidden or stalled. At the reader's fixed chunk size this caps
        // queued output at 1 MiB instead of allowing a noisy child to consume
        // memory without limit.
        let (sender, output) = mpsc::sync_channel(OUTPUT_QUEUE_CHUNKS);
        std::thread::Builder::new()
            .name("spectrum-terminal-reader".into())
            .spawn(move || {
                let mut buffer = [0_u8; OUTPUT_CHUNK_BYTES];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            if sender
                                .send(TerminalEvent::Output(buffer[..read].to_vec()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => break,
                        Err(error) => {
                            let _ = sender.send(TerminalEvent::Error(format!(
                                "terminal output stopped: {error}"
                            )));
                            break;
                        }
                    }
                }
            })
            .context("could not start the terminal output reader")?;

        Ok(Self {
            context,
            master: pair.master,
            writer,
            child,
            output,
            exit_reported: false,
        })
    }
}

impl TerminalSessionBackend for PortablePtySession {
    fn context(&self) -> &TerminalContext {
        &self.context
    }

    fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if self.exit_reported {
            bail!("terminal process has exited");
        }
        self.writer
            .write_all(bytes)
            .context("could not send input to the terminal")?;
        self.writer
            .flush()
            .context("could not flush terminal input")
    }

    fn resize(&self, size: TerminalSize) -> Result<()> {
        self.master
            .resize(size.pty())
            .context("could not resize the terminal")
    }

    fn poll(&mut self) -> Vec<TerminalEvent> {
        // Keep one UI tick bounded even when a build or other noisy process
        // has filled the queue. Remaining chunks are drained by later ticks.
        let mut events = drain_events(&self.output, POLL_BYTE_BUDGET);
        if !self.exit_reported {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.exit_reported = true;
                    events.push(TerminalEvent::Exited(TerminalExit {
                        code: status.exit_code(),
                        signal: status.signal().map(str::to_owned),
                    }));
                }
                Ok(None) => {}
                Err(error) => {
                    self.exit_reported = true;
                    events.push(TerminalEvent::Error(format!(
                        "could not inspect terminal process: {error}"
                    )));
                }
            }
        }
        events
    }

    fn is_running(&mut self) -> bool {
        if self.exit_reported {
            return false;
        }
        match self.child.try_wait() {
            Ok(Some(_)) | Err(_) => {
                self.exit_reported = true;
                false
            }
            Ok(None) => true,
        }
    }

    fn terminate(&mut self) -> Result<()> {
        if self.is_running() {
            self.child
                .kill()
                .context("could not stop the terminal process")?;
            self.child
                .wait()
                .context("could not reap the terminal process")?;
            self.exit_reported = true;
        }
        Ok(())
    }
}

fn drain_events(output: &Receiver<TerminalEvent>, byte_budget: usize) -> Vec<TerminalEvent> {
    let mut events = Vec::new();
    let mut output_bytes = 0;
    while output_bytes < byte_budget {
        let Ok(event) = output.try_recv() else {
            break;
        };
        if let TerminalEvent::Output(bytes) = &event {
            output_bytes = output_bytes.saturating_add(bytes.len());
        }
        events.push(event);
    }
    events
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.backend.terminate();
    }
}

#[cfg(unix)]
fn shell_command() -> CommandBuilder {
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh"));
    let mut command = CommandBuilder::new(shell);
    command.arg("-i");
    command
}

#[cfg(windows)]
fn shell_command() -> CommandBuilder {
    CommandBuilder::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct TestBackend {
        context: TerminalContext,
        writes: Vec<Vec<u8>>,
        size: TerminalSize,
        events: Vec<TerminalEvent>,
        running: bool,
    }

    impl TerminalSessionBackend for TestBackend {
        fn context(&self) -> &TerminalContext {
            &self.context
        }

        fn process_id(&self) -> Option<u32> {
            Some(42)
        }

        fn write(&mut self, bytes: &[u8]) -> Result<()> {
            self.writes.push(bytes.to_vec());
            Ok(())
        }

        fn resize(&self, size: TerminalSize) -> Result<()> {
            assert_eq!(size, self.size);
            Ok(())
        }

        fn poll(&mut self) -> Vec<TerminalEvent> {
            std::mem::take(&mut self.events)
        }

        fn is_running(&mut self) -> bool {
            self.running
        }

        fn terminate(&mut self) -> Result<()> {
            self.running = false;
            Ok(())
        }
    }

    #[test]
    fn context_values_are_never_interpolated_into_shell_source() {
        let context = TerminalContext::new("a working directory")
            .with_env("SPECTRUM_DOCUMENT", "$(touch should-not-run); artwork")
            .with_env("PRISM_PROJECT", "a project with spaces.prism");
        let command = context.command();

        assert_eq!(
            command.get_cwd().map(OsString::as_os_str),
            Some(OsStr::new("a working directory"))
        );
        assert_eq!(
            command.get_env("SPECTRUM_DOCUMENT"),
            Some(OsStr::new("$(touch should-not-run); artwork"))
        );
        assert_eq!(
            command.get_env("PRISM_PROJECT"),
            Some(OsStr::new("a project with spaces.prism"))
        );
    }

    #[test]
    fn packaged_cli_directory_is_first_on_path() {
        let cli_directory = std::env::current_dir().unwrap().join("packaged-cli");
        let context = TerminalContext::new(std::env::current_dir().unwrap())
            .with_cli_directory(&cli_directory);
        let path = context.environment("PATH").unwrap();
        let first = std::env::split_paths(path).next();

        assert_eq!(first.as_deref(), Some(cli_directory.as_path()));
        assert_eq!(
            context.environment("SPECTRUM_CLI_DIR"),
            Some(cli_directory.as_os_str())
        );
    }

    #[test]
    fn session_contract_delegates_without_changing_the_portable_api() {
        let context = TerminalContext::new("backend working directory");
        let size = TerminalSize::new(40, 120);
        let mut session = TerminalSession::from_backend(TestBackend {
            context: context.clone(),
            writes: Vec::new(),
            size,
            events: vec![TerminalEvent::Output(b"ready".to_vec())],
            running: true,
        });

        assert_eq!(session.context(), &context);
        assert_eq!(session.process_id(), Some(42));
        assert!(session.is_running());
        session.write(b"input").unwrap();
        session.resize(size).unwrap();
        assert_eq!(
            session.poll(),
            vec![TerminalEvent::Output(b"ready".to_vec())]
        );
        session.terminate().unwrap();
        assert!(!session.is_running());
    }

    #[test]
    fn pseudo_terminal_runs_a_persistent_shell() {
        let mut session = TerminalSession::spawn(
            TerminalContext::new(std::env::current_dir().unwrap()),
            TerminalSize::default(),
        )
        .unwrap();
        session.write(b"echo SPECTRUM_TERMINAL_SMOKE\n").unwrap();

        let started = Instant::now();
        let mut output = Vec::new();
        while started.elapsed() < Duration::from_secs(3) {
            for event in session.poll() {
                if let TerminalEvent::Output(bytes) = event {
                    output.extend(bytes);
                }
            }
            if String::from_utf8_lossy(&output).contains("SPECTRUM_TERMINAL_SMOKE") {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "terminal did not echo smoke marker: {}",
            String::from_utf8_lossy(&output)
        );
    }

    #[test]
    fn terminating_a_terminal_reaps_its_child() {
        let mut session = TerminalSession::spawn(
            TerminalContext::new(std::env::current_dir().unwrap()),
            TerminalSize::default(),
        )
        .unwrap();

        assert!(session.is_running());
        session.terminate().unwrap();
        assert!(!session.is_running());
    }

    #[test]
    fn event_drain_leaves_noisy_output_for_later_polls() {
        let (sender, receiver) = mpsc::sync_channel(8);
        for marker in 0..4_u8 {
            sender.send(TerminalEvent::Output(vec![marker; 8])).unwrap();
        }
        let first = drain_events(&receiver, 10);
        assert_eq!(first.len(), 2);
        assert_eq!(receiver.try_iter().count(), 2);
    }

    #[test]
    fn output_queue_has_a_hard_one_megabyte_bound() {
        assert_eq!(OUTPUT_CHUNK_BYTES * OUTPUT_QUEUE_CHUNKS, 1024 * 1024);
    }
}
