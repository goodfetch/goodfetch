//! An Attached engine a test drives over stdio (ADRs 0024 and 0062): a
//! Product's executable run as `attached-engine` under the test's
//! environment, its stdout read through `StrictStdout`, so a log line there
//! fails the test, and each message it sends read as one of the protocol's
//! (ADR 0038). When it ends without exiting by itself, killed or left to the
//! test's teardown, it records the calls it read answered in the test's
//! answer record: the killed engine's log loses the dispatch records its
//! writer's thread had not written yet (ADR 0122), which the exercise audit
//! reads (#1779).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::time::Duration;

use goodfetch_protocol::events::EVENTS_SUBSCRIBE;
use goodfetch_protocol::foxhound::crawl::{CRAWL_REQUEST_HANDLE, CRAWL_START};
use goodfetch_protocol::jsonrpc::Message;
use goodfetch_test_support::{
    AnsweredCalls, MESSAGE_DEADLINE, Process, StrictStdout, TestEnv, record_command_in,
};
use serde_json::{Value, json};

use crate::hello::Hello;

/// The hidden subcommand that runs a Product's executable as an Attached
/// engine.
const ATTACHED_ENGINE: &str = "attached-engine";

/// A running Attached engine, started in a test's setup.
pub struct Attached {
    process: Process,
    stdin: Option<ChildStdin>,
    stdout: StrictStdout,
    /// The engine instance its `hello` named, once [`Attached::open`] said
    /// it.
    engine_instance: Value,
    /// The Datasets a `crawl.start` that declares none declares, if any.
    declared_by_default: Vec<String>,
    /// The test's folder, which holds its answer and command records.
    folder: PathBuf,
    answered: AnsweredCalls,
    /// Whether the engine exited by itself, its log written whole.
    exited: bool,
}

impl Attached {
    /// Starts `executable`'s Attached engine, its `log.level` at `trace`,
    /// each message awaited for up to [`MESSAGE_DEADLINE`].
    pub fn start(env: &TestEnv, executable: impl AsRef<OsStr>) -> Self {
        Self::start_with(env, executable, MESSAGE_DEADLINE, |_| {})
    }

    /// Starts it as [`Attached::start`] does after `configure` changed its
    /// command, such as its environment, each message awaited for up to
    /// `deadline`, as a call that launches a browser needs.
    pub fn start_with(
        env: &TestEnv,
        executable: impl AsRef<OsStr>,
        deadline: Duration,
        configure: impl FnOnce(&mut Command),
    ) -> Self {
        Self::start_logging(env, executable, deadline, "trace", configure)
    }

    /// Starts it as [`Attached::start_with`] does, its `log.level` at
    /// `level` rather than `trace`.
    pub fn start_logging(
        env: &TestEnv,
        executable: impl AsRef<OsStr>,
        deadline: Duration,
        level: &str,
        configure: impl FnOnce(&mut Command),
    ) -> Self {
        env.log_level(level);
        let mut command = env.process_command(executable);
        command.arg(ATTACHED_ENGINE);
        configure(&mut command);
        Self::spawned(env, deadline, &mut command)
    }

    /// Starts `command`, which runs a Product's `attached-engine` as its
    /// last arguments, such as inside a cgroup a service manager made or a
    /// mount namespace of its own, each message awaited for up to
    /// `deadline`.
    ///
    /// # Panics
    ///
    /// When the command does not start.
    pub fn spawned(env: &TestEnv, deadline: Duration, command: &mut Command) -> Self {
        command.stderr(Stdio::piped());
        let process = env.spawn(command).expect("the Attached engine starts");
        let stdin = process.child().stdin.take();
        let stdout = StrictStdout::of(&process)
            .expect("the Attached engine's stdout and stderr are piped")
            .within(deadline);
        Self {
            process,
            stdin,
            stdout,
            engine_instance: Value::Null,
            declared_by_default: Vec::new(),
            folder: env.folder().to_owned(),
            answered: AnsweredCalls::default(),
            exited: false,
        }
    }

    /// The same engine, a `crawl.start` sent through it that declares no
    /// Dataset declaring `datasets`, with no schema or key (ADR 0103).
    #[must_use]
    pub fn declaring_by_default(mut self, datasets: &[&str]) -> Self {
        self.declared_by_default = datasets.iter().map(|name| (*name).to_owned()).collect();
        self
    }

    /// Says `hello` and answers the engine's reply, keeping the engine
    /// instance it names for [`Attached::subscribe_from_start`].
    ///
    /// # Panics
    ///
    /// When the engine closes before replying.
    pub fn open(&mut self, hello: &Hello) -> Value {
        self.send(&hello.message());
        let reply = self.next().expect("the engine replies to hello");
        self.engine_instance = reply["result"]["engine_instance"].clone();
        reply
    }

    /// Sends `message`.
    ///
    /// # Panics
    ///
    /// When the engine's input is closed already.
    pub fn send(&mut self, message: &Value) {
        let mut message = message.clone();
        if !self.declared_by_default.is_empty()
            && message["method"] == CRAWL_START.name
            && message["params"].get("datasets").is_none()
        {
            message["params"]["datasets"] = self
                .declared_by_default
                .iter()
                .map(|name| json!({ "name": name }))
                .collect();
        }
        self.answered.sent(&message);
        let stdin = self.stdin.as_mut().expect("the engine's input is open");
        let mut line = serde_json::to_vec(&message).expect("invariant: a message serializes");
        line.push(b'\n');
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .expect("the engine reads its input");
    }

    /// The next message, read as one of the protocol's (ADR 0038); `None`
    /// once the engine closed its stdout.
    ///
    /// # Panics
    ///
    /// When nothing comes within the deadline, a line is no JSON or no
    /// protocol message, or the engine logged on stdout.
    #[expect(
        clippy::should_implement_trait,
        reason = "each read waits up to the deadline and panics past it, which an Iterator's `next` would hide"
    )]
    pub fn next(&mut self) -> Option<Value> {
        let message = self
            .stdout
            .try_next()
            .unwrap_or_else(|error| panic!("{error}"))?;
        serde_json::from_value::<Message>(message.clone())
            .unwrap_or_else(|error| panic!("not a protocol message ({error}): {message}"));
        self.answered.read(&message);
        Some(message)
    }

    /// Sends the call `id` of `method` with `params` and answers the next
    /// message, whatever it is.
    ///
    /// # Panics
    ///
    /// When the engine closes first.
    pub fn ask(&mut self, id: impl Into<Value>, method: &str, params: &Value) -> Value {
        self.send(
            &json!({ "jsonrpc": "2.0", "id": id.into(), "method": method, "params": params }),
        );
        self.next().expect("the engine answers the call")
    }

    /// Sends each of `calls`, its id, its method's name and its params,
    /// before reading any answer, so the engine runs them at the same time;
    /// answers each one's reply, in the order of `calls`.
    ///
    /// # Panics
    ///
    /// When the engine closes before replying to every call.
    pub fn at_once(&mut self, calls: &[(i64, &str, Value)]) -> Vec<Value> {
        for (id, method, params) in calls {
            self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        }
        let mut replies = BTreeMap::new();
        while replies.len() < calls.len() {
            let message = self.next().expect("the engine answers every call");
            if let Some(id) = message["id"].as_i64() {
                replies.insert(id, message);
            }
        }
        calls
            .iter()
            .map(|(id, ..)| replies.remove(id).expect("one reply per call"))
            .collect()
    }

    /// Sends the call `id`, answers each `crawl_request.handle` request
    /// with what `handler` makes of its params (none when it gives none),
    /// skips every other message, and answers the call's reply.
    ///
    /// # Panics
    ///
    /// When the engine closes before replying.
    pub fn call(
        &mut self,
        id: &str,
        method: &str,
        params: &Value,
        handler: &mut impl FnMut(&Value) -> Option<Value>,
    ) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next().expect("the engine closed before replying");
            if message["method"] == CRAWL_REQUEST_HANDLE.name {
                if let Some(mut reply) = handler(&message["params"]) {
                    reply["jsonrpc"] = json!("2.0");
                    reply["id"] = message["id"].clone();
                    self.send(&reply);
                }
            } else if message["id"] == id {
                return message;
            }
        }
    }

    /// Whether the engine writes nothing on its stdout for `quiet`; a
    /// message written meanwhile is kept for the next read.
    pub fn is_quiet_for(&mut self, quiet: Duration) -> bool {
        self.stdout.is_quiet_for(quiet)
    }

    /// Subscribes to the event stream as the call `id`, from the start of
    /// the engine instance its `hello` named: it reads every event, even
    /// those of calls that answered before it was armed.
    pub fn subscribe_from_start(&mut self, id: &str) {
        let after = json!({ "engine_instance": self.engine_instance, "position": 0 });
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": EVENTS_SUBSCRIBE.name, "params": { "after": after },
        }));
    }

    /// The engine's process ID, which names its log file.
    pub fn pid(&self) -> u32 {
        self.process.child().id()
    }

    /// Kills the engine (`kill -9`, `TerminateProcess` on Windows) and
    /// waits for it to end, as a power cut or a crash ends it; the calls it
    /// answered go to the test's answer record.
    ///
    /// # Panics
    ///
    /// When it cannot be killed or reaped.
    pub fn kill(self) {
        self.process
            .stop()
            .expect("the engine is killed and reaped");
    }

    /// Ends the input and waits for the engine to exit, answering whatever
    /// it still sent and whether it exited successfully. Records
    /// `attached-engine` and its exit code in the test's command record,
    /// for the exercise audit (ADR 0072).
    pub fn finish(mut self) -> (Vec<Value>, bool) {
        self.end()
    }

    /// As [`Attached::finish`], answering only whether it exited
    /// successfully.
    pub fn finished(mut self) -> bool {
        self.end().1
    }

    /// As [`Attached::finish`], answering what it still sent and
    /// everything it wrote on stderr.
    pub fn finish_with_stderr(mut self) -> (Vec<Value>, String) {
        let (rest, _) = self.end();
        (rest, self.stdout.stderr_to_end())
    }

    fn end(&mut self) -> (Vec<Value>, bool) {
        drop(self.stdin.take());
        let mut rest = Vec::new();
        while let Some(message) = self.next() {
            rest.push(message);
        }
        let status = self.process.child().wait().expect("the engine is reaped");
        self.exited = status.success();
        record_command_in(&self.folder, &[ATTACHED_ENGINE], status.code())
            .expect("the test's command record is written");
        (rest, self.exited)
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        // A folder already gone means the test's teardown ran, copying the
        // record that nothing can add to any more.
        if self.exited || !self.folder.is_dir() {
            return;
        }
        if let Err(error) = self.answered.record_in(&self.folder)
            && !std::thread::panicking()
        {
            panic!("the test's answer record cannot be written: {error}");
        }
    }
}
