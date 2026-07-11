use std::collections::HashSet;
use std::future::Future;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use clap::{Arg, ArgMatches, Command};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command as ProcessCommand};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use super::{CommandExit, CommandInvocation, CommandRegistrar, CommandRegistry};
use crate::foundation::{Error, Result};
use crate::support::CommandId;

const DEV_COMMAND_NAME: &str = "dev";
const DEV_COMMAND: CommandId = CommandId::new(DEV_COMMAND_NAME);
const PROCESS_ENV: &str = "PROCESS";
const DEFAULT_RESTART_BACKOFF_MS: u64 = 1_000;
const MIN_RESTART_BACKOFF_MS: u64 = 100;
const MAX_RESTART_BACKOFF_MS: u64 = 60_000;
const MAX_RESTARTS: u32 = 100;
const MAX_EFFECTIVE_BACKOFF: Duration = Duration::from_millis(MAX_RESTART_BACKOFF_MS);
const DEFAULT_CHILD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DevProcess {
    Http,
    Worker,
    Scheduler,
    Websocket,
}

impl DevProcess {
    const ALL: [Self; 4] = [Self::Http, Self::Worker, Self::Scheduler, Self::Websocket];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Worker => "worker",
            Self::Scheduler => "scheduler",
            Self::Websocket => "websocket",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "http" => Ok(Self::Http),
            "worker" => Ok(Self::Worker),
            "scheduler" => Ok(Self::Scheduler),
            "websocket" => Ok(Self::Websocket),
            _ => Err(Error::message(format!(
                "unsupported dev process `{value}`; expected http, worker, scheduler, or websocket"
            ))),
        }
    }
}

impl std::fmt::Display for DevProcess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DevOptions {
    processes: Vec<DevProcess>,
    max_restarts: u32,
    restart_backoff: Duration,
}

impl DevOptions {
    fn from_matches(matches: &ArgMatches) -> Result<Self> {
        let processes = match matches.get_many::<String>("process") {
            Some(values) => {
                let mut selected = Vec::new();
                let mut seen = HashSet::new();
                for value in values {
                    let process = DevProcess::parse(value)?;
                    if !seen.insert(process) {
                        return Err(Error::message(format!(
                            "dev process `{process}` was selected more than once"
                        )));
                    }
                    selected.push(process);
                }
                selected
            }
            None => DevProcess::ALL.to_vec(),
        };

        let max_restarts = matches
            .get_one::<u32>("max-restarts")
            .copied()
            .unwrap_or_default();
        if max_restarts > MAX_RESTARTS {
            return Err(Error::message(format!(
                "--max-restarts cannot exceed {MAX_RESTARTS}"
            )));
        }

        let restart_backoff_ms = matches
            .get_one::<u64>("restart-backoff-ms")
            .copied()
            .unwrap_or(DEFAULT_RESTART_BACKOFF_MS);
        if !(MIN_RESTART_BACKOFF_MS..=MAX_RESTART_BACKOFF_MS).contains(&restart_backoff_ms) {
            return Err(Error::message(format!(
                "--restart-backoff-ms must be between {MIN_RESTART_BACKOFF_MS} and {MAX_RESTART_BACKOFF_MS}"
            )));
        }

        Ok(Self {
            processes,
            max_restarts,
            restart_backoff: Duration::from_millis(restart_backoff_ms),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DevProcessExit {
    success: bool,
    code: Option<i32>,
}

impl DevProcessExit {
    fn from_status(status: std::process::ExitStatus) -> Self {
        Self {
            success: status.success(),
            code: status.code(),
        }
    }

    fn description(self) -> String {
        self.code
            .map(|code| format!("status {code}"))
            .unwrap_or_else(|| "a termination signal".to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DevRunOutcome {
    Exited(DevProcessExit),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SupervisorOutcome {
    Completed,
    Cancelled,
    RestartLimitReached,
}

#[derive(Clone, Copy)]
enum ChildStream {
    Stdout,
    Stderr,
}

#[async_trait]
trait DevProcessRunner: Send + Sync {
    async fn run_once(
        &self,
        process: DevProcess,
        invocation: CommandInvocation,
        cancellation: watch::Receiver<bool>,
    ) -> Result<DevRunOutcome>;
}

struct TokioDevProcessRunner;

#[async_trait]
impl DevProcessRunner for TokioDevProcessRunner {
    async fn run_once(
        &self,
        process: DevProcess,
        invocation: CommandInvocation,
        mut cancellation: watch::Receiver<bool>,
    ) -> Result<DevRunOutcome> {
        let executable = std::env::current_exe().map_err(|error| {
            Error::message(format!(
                "could not resolve the current executable for `{process}`: {error}"
            ))
        })?;
        let mut child = ProcessCommand::new(&executable)
            .env(PROCESS_ENV, process.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                Error::message(format!(
                    "could not start `{}` with {PROCESS_ENV}={process}: {error}",
                    executable.display()
                ))
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::message(format!("could not capture stdout for `{process}`")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::message(format!("could not capture stderr for `{process}`")))?;

        let stdout_task =
            spawn_output_relay(stdout, process, ChildStream::Stdout, invocation.clone());
        let stderr_task =
            spawn_output_relay(stderr, process, ChildStream::Stderr, invocation.clone());

        let wait_result = tokio::select! {
            status = child.wait() => Some(status),
            _ = wait_for_cancellation(&mut cancellation) => None,
        };

        let outcome = match wait_result {
            Some(status) => status
                .map(DevProcessExit::from_status)
                .map(DevRunOutcome::Exited)
                .map_err(|error| {
                    Error::message(format!("failed while waiting for `{process}`: {error}"))
                }),
            None => stop_child(
                &mut child,
                process,
                child_shutdown_timeout(&invocation, process),
            )
            .await
            .map(|()| DevRunOutcome::Cancelled),
        };

        let (stdout_result, stderr_result) = tokio::join!(
            join_output_relay(stdout_task, process, ChildStream::Stdout),
            join_output_relay(stderr_task, process, ChildStream::Stderr),
        );
        let outcome = outcome?;
        stdout_result?;
        stderr_result?;

        Ok(outcome)
    }
}

pub(crate) fn dev_cli_registrar() -> CommandRegistrar {
    dev_cli_registrar_with_runner(Arc::new(TokioDevProcessRunner))
}

fn dev_cli_registrar_with_runner(runner: Arc<dyn DevProcessRunner>) -> CommandRegistrar {
    Arc::new(move |registry: &mut CommandRegistry| {
        let runner = runner.clone();
        registry.command_with_exit(DEV_COMMAND, dev_command(), move |invocation| {
            let runner = runner.clone();
            async move {
                let options = DevOptions::from_matches(invocation.matches())?;
                run_dev_with_runner(
                    invocation,
                    options,
                    runner,
                    crate::kernel::shutdown::shutdown_signal(),
                )
                .await
            }
        })?;
        Ok(())
    })
}

fn dev_command() -> Command {
    Command::new(DEV_COMMAND_NAME)
        .about("Run application processes together for local development")
        .long_about(
            "Run selected processes from the current application executable. Each child receives \
             PROCESS=http, worker, scheduler, or websocket; this command does not generate or install \
             an application.",
        )
        .arg(
            Arg::new("process")
                .value_name("PROCESS")
                .num_args(0..)
                .value_parser(["http", "worker", "scheduler", "websocket"])
                .help("Processes to run (all four by default)"),
        )
        .arg(
            Arg::new("max-restarts")
                .long("max-restarts")
                .value_name("COUNT")
                .value_parser(clap::value_parser!(u32))
                .default_value("0")
                .help("Maximum restart attempts per failed process (0-100)"),
        )
        .arg(
            Arg::new("restart-backoff-ms")
                .long("restart-backoff-ms")
                .value_name("MILLISECONDS")
                .value_parser(clap::value_parser!(u64))
                .default_value("1000")
                .help("Initial restart delay; exponential backoff is capped at 60000 ms"),
        )
}

async fn run_dev_with_runner<F>(
    invocation: CommandInvocation,
    options: DevOptions,
    runner: Arc<dyn DevProcessRunner>,
    shutdown: F,
) -> Result<CommandExit>
where
    F: Future<Output = ()> + Send,
{
    if options.processes.is_empty() {
        return Err(Error::message(
            "dev requires at least one process; omit process names to select all four",
        ));
    }

    invocation.line(format!(
        "[dev] supervising {} process(es): {}",
        options.processes.len(),
        options
            .processes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ))?;

    let (cancel, cancellation) = watch::channel(false);
    let mut supervisors = JoinSet::new();
    for process in options.processes.iter().copied() {
        supervisors.spawn(supervise_process(
            process,
            options.clone(),
            runner.clone(),
            invocation.clone(),
            cancellation.clone(),
        ));
    }

    tokio::pin!(shutdown);
    let mut cancellation_started = false;
    let mut failed = false;

    loop {
        tokio::select! {
            _ = &mut shutdown, if !cancellation_started => {
                invocation.line("[dev] shutdown signal received; stopping child processes")?;
                cancellation_started = true;
                let _ = cancel.send(true);
            }
            result = supervisors.join_next() => {
                let Some(result) = result else {
                    break;
                };
                match result {
                    Ok(Ok(SupervisorOutcome::Completed)) => {
                        if !cancellation_started && !supervisors.is_empty() {
                            invocation.line(
                                "[dev] a process exited successfully; stopping remaining processes",
                            )?;
                            cancellation_started = true;
                            let _ = cancel.send(true);
                        }
                    }
                    Ok(Ok(SupervisorOutcome::Cancelled)) => {}
                    Ok(Ok(SupervisorOutcome::RestartLimitReached)) => {
                        failed = true;
                        if !cancellation_started {
                            invocation.error(
                                "[dev] a process exhausted its restart limit; stopping remaining processes",
                            )?;
                            cancellation_started = true;
                            let _ = cancel.send(true);
                        }
                    }
                    Ok(Err(error)) => {
                        failed = true;
                        invocation.error(format!("[dev] process supervisor failed: {error}"))?;
                        if !cancellation_started {
                            cancellation_started = true;
                            let _ = cancel.send(true);
                        }
                    }
                    Err(error) => {
                        failed = true;
                        invocation.error(format!("[dev] process supervisor task failed: {error}"))?;
                        if !cancellation_started {
                            cancellation_started = true;
                            let _ = cancel.send(true);
                        }
                    }
                }
            }
        }
    }

    if cancellation_started {
        invocation.line("[dev] all child processes stopped")?;
    } else {
        invocation.line("[dev] all child processes exited")?;
    }

    Ok(if failed {
        CommandExit::FAILURE
    } else {
        CommandExit::SUCCESS
    })
}

async fn supervise_process(
    process: DevProcess,
    options: DevOptions,
    runner: Arc<dyn DevProcessRunner>,
    invocation: CommandInvocation,
    mut cancellation: watch::Receiver<bool>,
) -> Result<SupervisorOutcome> {
    let mut restarts = 0;

    loop {
        if *cancellation.borrow() {
            return Ok(SupervisorOutcome::Cancelled);
        }

        invocation.line(format!(
            "[dev] starting {process} ({PROCESS_ENV}={process})"
        ))?;
        let run_result = runner
            .run_once(process, invocation.clone(), cancellation.clone())
            .await;

        if *cancellation.borrow() {
            if let Err(error) = run_result {
                return Err(error);
            }
            invocation.line(format!("[dev] {process} stopped"))?;
            return Ok(SupervisorOutcome::Cancelled);
        }

        match run_result {
            Ok(DevRunOutcome::Cancelled) => {
                invocation.line(format!("[dev] {process} stopped"))?;
                return Ok(SupervisorOutcome::Cancelled);
            }
            Ok(DevRunOutcome::Exited(exit)) if exit.success => {
                invocation.line(format!("[dev] {process} exited successfully"))?;
                return Ok(SupervisorOutcome::Completed);
            }
            Ok(DevRunOutcome::Exited(exit)) => {
                invocation.error(format!(
                    "[dev] {process} exited with {}",
                    exit.description()
                ))?;
            }
            Err(error) => {
                invocation.error(format!("[dev] {process} failed: {error}"))?;
            }
        }

        if restarts >= options.max_restarts {
            invocation.error(format!(
                "[dev] {process} exhausted its restart limit ({})",
                options.max_restarts
            ))?;
            return Ok(SupervisorOutcome::RestartLimitReached);
        }

        restarts += 1;
        let delay = restart_delay(options.restart_backoff, restarts);
        invocation.line(format!(
            "[dev] restarting {process} ({restarts}/{}) in {} ms",
            options.max_restarts,
            delay.as_millis()
        ))?;

        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = wait_for_cancellation(&mut cancellation) => {
                return Ok(SupervisorOutcome::Cancelled);
            }
        }
    }
}

fn restart_delay(base: Duration, restart: u32) -> Duration {
    let exponent = restart.saturating_sub(1).min(31);
    let multiplier = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
    base.checked_mul(multiplier)
        .unwrap_or(MAX_EFFECTIVE_BACKOFF)
        .min(MAX_EFFECTIVE_BACKOFF)
}

async fn wait_for_cancellation(cancellation: &mut watch::Receiver<bool>) {
    while !*cancellation.borrow() {
        if cancellation.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn spawn_output_relay<R>(
    reader: R,
    process: DevProcess,
    stream: ChildStream,
    invocation: CommandInvocation,
) -> JoinHandle<Result<()>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(relay_output(reader, process, stream, invocation))
}

async fn relay_output<R>(
    reader: R,
    process: DevProcess,
    stream: ChildStream,
    invocation: CommandInvocation,
) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let read = reader
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(|error| Error::message(format!("failed to read {process} output: {error}")))?;
        if read == 0 {
            return Ok(());
        }
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        write_child_line(
            &invocation,
            process,
            stream,
            String::from_utf8_lossy(&bytes),
        )?;
    }
}

fn write_child_line(
    invocation: &CommandInvocation,
    process: DevProcess,
    stream: ChildStream,
    line: impl AsRef<str>,
) -> Result<()> {
    let line = format!("[{process}] {}", line.as_ref());
    match stream {
        ChildStream::Stdout => invocation.line(line),
        ChildStream::Stderr => invocation.error(line),
    }
}

async fn join_output_relay(
    relay: JoinHandle<Result<()>>,
    process: DevProcess,
    stream: ChildStream,
) -> Result<()> {
    relay.await.map_err(|error| {
        let stream = match stream {
            ChildStream::Stdout => "stdout",
            ChildStream::Stderr => "stderr",
        };
        Error::message(format!("{process} {stream} relay task failed: {error}"))
    })?
}

fn child_shutdown_timeout(invocation: &CommandInvocation, process: DevProcess) -> Duration {
    let config = invocation.app().config();
    let timeout_ms = match process {
        DevProcess::Worker => config.jobs().map(|config| config.shutdown_timeout_ms),
        DevProcess::Scheduler => config.scheduler().map(|config| config.shutdown_timeout_ms),
        DevProcess::Http | DevProcess::Websocket => config
            .app()
            .map(|config| config.background_shutdown_timeout_ms),
    };
    timeout_ms
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_CHILD_SHUTDOWN_TIMEOUT)
}

async fn stop_child(
    child: &mut Child,
    process: DevProcess,
    shutdown_timeout: Duration,
) -> Result<()> {
    if child
        .try_wait()
        .map_err(|error| Error::message(format!("could not inspect `{process}`: {error}")))?
        .is_some()
    {
        return Ok(());
    }

    request_graceful_stop(child).await;
    match tokio::time::timeout(shutdown_timeout, child.wait()).await {
        Ok(result) => {
            result.map_err(|error| {
                Error::message(format!("failed while stopping `{process}`: {error}"))
            })?;
            Ok(())
        }
        Err(_) => {
            child.start_kill().map_err(|error| {
                Error::message(format!(
                    "`{process}` did not stop within {} ms and could not be killed: {error}",
                    shutdown_timeout.as_millis()
                ))
            })?;
            child.wait().await.map_err(|error| {
                Error::message(format!("failed while force-stopping `{process}`: {error}"))
            })?;
            Ok(())
        }
    }
}

#[cfg(unix)]
async fn request_graceful_stop(child: &mut Child) {
    let Some(id) = child.id() else {
        return;
    };
    let _ = ProcessCommand::new("/bin/kill")
        .arg("-TERM")
        .arg(id.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

#[cfg(not(unix))]
async fn request_graceful_stop(child: &mut Child) {
    let _ = child.start_kill();
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use tokio::sync::mpsc;

    use super::*;
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::testing::CommandIoFake;
    use crate::validation::RuleRegistry;

    #[derive(Clone)]
    enum FakeStep {
        Exit {
            code: i32,
            stdout: &'static str,
            stderr: &'static str,
        },
        ExitWhenNotified {
            notify: Arc<tokio::sync::Notify>,
            code: i32,
        },
        WaitForCancellation,
    }

    #[derive(Default)]
    struct FakeState {
        scripts: HashMap<DevProcess, VecDeque<FakeStep>>,
        calls: Vec<DevProcess>,
        cancelled: Vec<DevProcess>,
    }

    struct FakeRunner {
        state: Arc<Mutex<FakeState>>,
        started: Option<mpsc::UnboundedSender<DevProcess>>,
    }

    impl FakeRunner {
        fn new(scripts: impl IntoIterator<Item = (DevProcess, Vec<FakeStep>)>) -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeState {
                    scripts: scripts
                        .into_iter()
                        .map(|(process, steps)| (process, steps.into()))
                        .collect(),
                    ..FakeState::default()
                })),
                started: None,
            }
        }

        fn with_started_sender(mut self, started: mpsc::UnboundedSender<DevProcess>) -> Self {
            self.started = Some(started);
            self
        }

        fn calls(&self) -> Vec<DevProcess> {
            self.state.lock().unwrap().calls.clone()
        }

        fn cancelled(&self) -> Vec<DevProcess> {
            self.state.lock().unwrap().cancelled.clone()
        }
    }

    #[async_trait]
    impl DevProcessRunner for FakeRunner {
        async fn run_once(
            &self,
            process: DevProcess,
            invocation: CommandInvocation,
            mut cancellation: watch::Receiver<bool>,
        ) -> Result<DevRunOutcome> {
            let step = {
                let mut state = self.state.lock().unwrap();
                state.calls.push(process);
                state
                    .scripts
                    .get_mut(&process)
                    .and_then(VecDeque::pop_front)
                    .unwrap_or(FakeStep::WaitForCancellation)
            };
            if let Some(started) = &self.started {
                let _ = started.send(process);
            }

            match step {
                FakeStep::Exit {
                    code,
                    stdout,
                    stderr,
                } => {
                    if !stdout.is_empty() {
                        write_child_line(&invocation, process, ChildStream::Stdout, stdout)?;
                    }
                    if !stderr.is_empty() {
                        write_child_line(&invocation, process, ChildStream::Stderr, stderr)?;
                    }
                    Ok(DevRunOutcome::Exited(DevProcessExit {
                        success: code == 0,
                        code: Some(code),
                    }))
                }
                FakeStep::ExitWhenNotified { notify, code } => {
                    notify.notified().await;
                    Ok(DevRunOutcome::Exited(DevProcessExit {
                        success: code == 0,
                        code: Some(code),
                    }))
                }
                FakeStep::WaitForCancellation => {
                    wait_for_cancellation(&mut cancellation).await;
                    self.state.lock().unwrap().cancelled.push(process);
                    Ok(DevRunOutcome::Cancelled)
                }
            }
        }
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn invocation(io: &CommandIoFake) -> CommandInvocation {
        CommandInvocation::new(test_app(), ArgMatches::default(), Arc::new(io.clone()))
    }

    #[test]
    fn parses_process_selection_and_enforces_restart_bounds() {
        let defaults = dev_command().try_get_matches_from(["dev"]).unwrap();
        assert_eq!(
            DevOptions::from_matches(&defaults).unwrap(),
            DevOptions {
                processes: DevProcess::ALL.to_vec(),
                max_restarts: 0,
                restart_backoff: Duration::from_millis(DEFAULT_RESTART_BACKOFF_MS),
            }
        );

        let selected = dev_command()
            .try_get_matches_from([
                "dev",
                "worker",
                "http",
                "--max-restarts",
                "3",
                "--restart-backoff-ms",
                "250",
            ])
            .unwrap();
        assert_eq!(
            DevOptions::from_matches(&selected).unwrap(),
            DevOptions {
                processes: vec![DevProcess::Worker, DevProcess::Http],
                max_restarts: 3,
                restart_backoff: Duration::from_millis(250),
            }
        );

        let duplicate = dev_command()
            .try_get_matches_from(["dev", "http", "http"])
            .unwrap();
        assert!(DevOptions::from_matches(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("selected more than once"));

        let too_many = dev_command()
            .try_get_matches_from(["dev", "--max-restarts", "101"])
            .unwrap();
        assert!(DevOptions::from_matches(&too_many)
            .unwrap_err()
            .to_string()
            .contains("cannot exceed 100"));

        let too_fast = dev_command()
            .try_get_matches_from(["dev", "--restart-backoff-ms", "99"])
            .unwrap();
        assert!(DevOptions::from_matches(&too_fast)
            .unwrap_err()
            .to_string()
            .contains("between 100 and 60000"));
    }

    #[test]
    fn restart_backoff_is_exponential_and_capped() {
        assert_eq!(
            restart_delay(Duration::from_millis(250), 1),
            Duration::from_millis(250)
        );
        assert_eq!(
            restart_delay(Duration::from_millis(250), 3),
            Duration::from_secs(1)
        );
        assert_eq!(
            restart_delay(Duration::from_secs(60), 100),
            MAX_EFFECTIVE_BACKOFF
        );
    }

    #[tokio::test]
    async fn output_relay_prefixes_each_complete_and_partial_line() {
        let io = CommandIoFake::new();
        let invocation = invocation(&io);

        relay_output(
            &b"first\nsecond\r\npartial"[..],
            DevProcess::Worker,
            ChildStream::Stdout,
            invocation.clone(),
        )
        .await
        .unwrap();
        relay_output(
            &b"warning\n"[..],
            DevProcess::Worker,
            ChildStream::Stderr,
            invocation,
        )
        .await
        .unwrap();

        io.assert_stdout("[worker] first\n[worker] second\n[worker] partial\n")
            .assert_stderr("[worker] warning\n");
    }

    #[tokio::test]
    async fn rejects_an_internal_empty_process_selection() {
        let io = CommandIoFake::new();
        let runner = Arc::new(FakeRunner::new(std::iter::empty::<(
            DevProcess,
            Vec<FakeStep>,
        )>()));
        let error = run_dev_with_runner(
            invocation(&io),
            DevOptions {
                processes: Vec::new(),
                max_restarts: 0,
                restart_backoff: Duration::from_secs(1),
            },
            runner.clone(),
            std::future::pending(),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("requires at least one process"));
        assert!(runner.calls().is_empty());
        io.assert_stdout("").assert_stderr("");
    }

    #[tokio::test]
    async fn prefixes_output_and_restarts_only_within_the_explicit_limit() {
        let io = CommandIoFake::new();
        let runner = Arc::new(FakeRunner::new([(
            DevProcess::Http,
            vec![
                FakeStep::Exit {
                    code: 1,
                    stdout: "boot one",
                    stderr: "crash one",
                },
                FakeStep::Exit {
                    code: 1,
                    stdout: "boot two",
                    stderr: "crash two",
                },
                FakeStep::Exit {
                    code: 0,
                    stdout: "ready",
                    stderr: "",
                },
            ],
        )]));
        let status = run_dev_with_runner(
            invocation(&io),
            DevOptions {
                processes: vec![DevProcess::Http],
                max_restarts: 2,
                restart_backoff: Duration::ZERO,
            },
            runner.clone(),
            std::future::pending(),
        )
        .await
        .unwrap();

        assert_eq!(status, CommandExit::SUCCESS);
        assert_eq!(runner.calls(), vec![DevProcess::Http; 3]);
        io.assert_stdout_contains("[http] boot one")
            .assert_stdout_contains("[dev] restarting http (1/2) in 0 ms")
            .assert_stdout_contains("[http] ready")
            .assert_stdout_contains("[dev] http exited successfully");
        io.assert_stderr_contains("[http] crash one")
            .assert_stderr_contains("[dev] http exited with status 1");
    }

    #[tokio::test]
    async fn exhausted_process_cancels_remaining_children() {
        let io = CommandIoFake::new();
        let runner = Arc::new(FakeRunner::new([
            (
                DevProcess::Http,
                vec![
                    FakeStep::Exit {
                        code: 1,
                        stdout: "",
                        stderr: "",
                    },
                    FakeStep::Exit {
                        code: 1,
                        stdout: "",
                        stderr: "",
                    },
                ],
            ),
            (DevProcess::Worker, vec![FakeStep::WaitForCancellation]),
        ]));
        let status = run_dev_with_runner(
            invocation(&io),
            DevOptions {
                processes: vec![DevProcess::Http, DevProcess::Worker],
                max_restarts: 1,
                restart_backoff: Duration::ZERO,
            },
            runner.clone(),
            std::future::pending(),
        )
        .await
        .unwrap();

        assert_eq!(status, CommandExit::FAILURE);
        assert_eq!(
            runner
                .calls()
                .into_iter()
                .filter(|process| *process == DevProcess::Http)
                .count(),
            2
        );
        assert!(runner.cancelled().contains(&DevProcess::Worker));
        io.assert_stderr_contains("http exhausted its restart limit (1)")
            .assert_stderr_contains("stopping remaining processes");
        io.assert_stdout_contains("[dev] all child processes stopped");
    }

    #[tokio::test]
    async fn clean_process_exit_stops_remaining_children_without_failing() {
        let io = CommandIoFake::new();
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let allow_http_exit = Arc::new(tokio::sync::Notify::new());
        let runner = Arc::new(
            FakeRunner::new([
                (
                    DevProcess::Http,
                    vec![FakeStep::ExitWhenNotified {
                        notify: allow_http_exit.clone(),
                        code: 0,
                    }],
                ),
                (DevProcess::Worker, vec![FakeStep::WaitForCancellation]),
            ])
            .with_started_sender(started_tx),
        );
        let task = tokio::spawn(run_dev_with_runner(
            invocation(&io),
            DevOptions {
                processes: vec![DevProcess::Http, DevProcess::Worker],
                max_restarts: 0,
                restart_backoff: Duration::from_secs(1),
            },
            runner.clone(),
            std::future::pending(),
        ));

        started_rx.recv().await.unwrap();
        started_rx.recv().await.unwrap();
        allow_http_exit.notify_one();
        let status = task.await.unwrap().unwrap();

        assert_eq!(status, CommandExit::SUCCESS);
        assert!(runner.cancelled().contains(&DevProcess::Worker));
        io.assert_stdout_contains("[dev] http exited successfully")
            .assert_stdout_contains(
                "[dev] a process exited successfully; stopping remaining processes",
            )
            .assert_stdout_contains("[dev] worker stopped")
            .assert_stdout_contains("[dev] all child processes stopped");
    }

    #[tokio::test]
    async fn shutdown_waits_for_every_selected_child_to_start_then_cleans_them_up() {
        let io = CommandIoFake::new();
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let runner = Arc::new(
            FakeRunner::new([
                (DevProcess::Http, vec![FakeStep::WaitForCancellation]),
                (DevProcess::Worker, vec![FakeStep::WaitForCancellation]),
            ])
            .with_started_sender(started_tx),
        );
        let shutdown = async move {
            started_rx.recv().await.unwrap();
            started_rx.recv().await.unwrap();
        };
        let status = run_dev_with_runner(
            invocation(&io),
            DevOptions {
                processes: vec![DevProcess::Http, DevProcess::Worker],
                max_restarts: 0,
                restart_backoff: Duration::from_secs(1),
            },
            runner.clone(),
            shutdown,
        )
        .await
        .unwrap();

        assert_eq!(status, CommandExit::SUCCESS);
        let mut cancelled = runner.cancelled();
        cancelled.sort_by_key(|process| process.as_str());
        assert_eq!(cancelled, vec![DevProcess::Http, DevProcess::Worker]);
        io.assert_stdout_contains("shutdown signal received")
            .assert_stdout_contains("[dev] http stopped")
            .assert_stdout_contains("[dev] worker stopped")
            .assert_stdout_contains("[dev] all child processes stopped");
    }

    #[test]
    fn command_help_distinguishes_orchestration_from_project_generation() {
        let mut help = Vec::new();
        dev_command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();

        assert!(help.contains("current application executable"));
        assert!(help.contains("does not generate or install an application"));
        assert!(help.contains("--max-restarts"));
    }
}
