//! One `needs-you answer` session: on one Connection it subscribes to the
//! engine's events, shows the list, and asks the person which item to answer
//! and how, saying what leaves the list or joins it while it waits for them.
//!
//! The subscription starts after a position no engine instance gave, so its
//! first event is `events.resync`, the whole list as of the stream's start:
//! no change falls between a read of the list and the subscription.

use std::cell::Cell;
use std::process::ExitCode;

use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::{ErrorGroup, Operation};
use goodfetch_protocol::connection::EngineInstance;
use goodfetch_protocol::events::{
    EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventEnvelope, EventsSubscribeParams, LastRead, Position,
    RESYNC, Resync,
};
use goodfetch_protocol::jsonrpc::{ErrorObject, Message, Notification, ProtocolFailure, RequestId};
use goodfetch_protocol::needs_you::{
    NEEDS_YOU_CHANGED, NEEDS_YOU_LIST, NeedsYouChange, NeedsYouItem,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::super::engine_choice::{Failed, Link, ReadValues};
use super::super::output::{self, Human};
use super::super::{exit_codes, prompts};
use super::kinds::Kinds;
use super::list::List;
use super::{Confirmation, ItemAnswer};

/// The id of the session's subscription; its other calls count up from it.
const SUBSCRIPTION: i64 = 1;

/// How many events the engine may send beyond those taken.
const CREDITS: u32 = 64;

/// An engine instance no engine has: every one draws its own.
const NO_INSTANCE: &str = "needs-you-answer";

/// What the person did at a prompt.
enum Typed {
    /// Typed this line.
    Line(String),
    /// Ended stdin.
    End,
    /// Nothing: what they were asked about left the list, or changed.
    Left,
}

/// How answering one item ended.
enum Ended {
    /// The session goes on, as it ended well.
    Done,
    /// The person ended stdin.
    End,
    /// The answer failed, which was printed, with this exit code.
    Failed(ExitCode),
}

/// One session, on an opened Connection.
pub(super) struct Session<'a> {
    link: &'a mut Link,
    kinds: Kinds,
    /// The operation of the call in flight, which a failure names.
    calling: &'a Cell<&'static str>,
    human: &'a Human,
    list: List,
    /// Whether the subscription gave the list.
    synced: bool,
    /// What changed, not said yet.
    news: Vec<String>,
    /// What the person types.
    typed: ReadValues,
    /// The id of the last call made.
    id: i64,
}

impl<'a> Session<'a> {
    /// A session over `link` with a person, whose items `kinds` prompts for.
    pub(super) fn new(
        link: &'a mut Link,
        kinds: Kinds,
        calling: &'a Cell<&'static str>,
        human: &'a Human,
    ) -> Self {
        Self {
            link,
            kinds,
            calling,
            human,
            list: List::default(),
            synced: false,
            news: Vec::new(),
            typed: ReadValues::new(|sender| {
                while let Some(line) = output::read_answer() {
                    if sender.blocking_send(Value::String(line)).is_err() {
                        return;
                    }
                }
            }),
            id: SUBSCRIPTION,
        }
    }

    /// Takes the person to the item `item`, or through the whole list, and
    /// answers the command's exit code.
    pub(super) async fn run(mut self, item: Option<&str>) -> Result<ExitCode, Failed> {
        self.subscribe().await?;
        let readers = TaskOwner::root(BlockingPolicy::Threads);
        self.typed.start(&readers);
        match item {
            Some(id) => self.answer_one(id).await,
            None => self.answer_all().await,
        }
    }

    /// Subscribes to the engine's events, and takes the list.
    async fn subscribe(&mut self) -> Result<(), Failed> {
        // Answers once every source was read, so that the resync of an
        // engine that just started holds their items.
        if let Err(error) = self.call(&NEEDS_YOU_LIST, json!({})).await? {
            return Err(Failed::Error(error));
        }
        self.calling.set(EVENTS_SUBSCRIBE.name);
        let params = EventsSubscribeParams {
            after: Some(LastRead {
                engine_instance: EngineInstance::new(NO_INSTANCE),
                position: Position::default(),
            }),
            system_info: false,
        };
        self.link
            .begin(SUBSCRIPTION, EVENTS_SUBSCRIBE.name, json!(params))
            .await?;
        self.link.grant(SUBSCRIPTION, CREDITS).await?;
        while !self.synced {
            let message = self.link.heard().await?;
            self.take(message).await?;
        }
        // The list as the session starts is shown whole, not as news.
        self.news.clear();
        Ok(())
    }

    /// Takes the person through the list, until they end it or nothing
    /// waits any more.
    async fn answer_all(&mut self) -> Result<ExitCode, Failed> {
        loop {
            self.say_news();
            if self.list.is_empty() {
                output::line("Nothing waits for a person.");
                return Ok(ExitCode::from(exit_codes::SUCCESS));
            }
            for row in self.list.rows() {
                for line in row.lines(self.kinds) {
                    output::line(&line);
                }
            }
            let which = "Which item? Its number, or Enter to end:";
            let text = match self.ask(which, |list| !list.is_empty()).await? {
                Typed::Line(text) if text.is_empty() => {
                    return Ok(ExitCode::from(exit_codes::SUCCESS));
                }
                Typed::Line(text) => text,
                Typed::End => return Ok(ExitCode::from(exit_codes::SUCCESS)),
                Typed::Left => continue,
            };
            let Some(item) = self.list.numbered(&text).cloned() else {
                output::line(&format!("No item {text} is listed."));
                continue;
            };
            if self.kinds.answers(&item).is_empty() {
                output::line("No answer is given here.");
                continue;
            }
            match self.answer(&item, "for the list").await? {
                Ended::Done | Ended::Failed(_) => {}
                Ended::End => return Ok(ExitCode::from(exit_codes::SUCCESS)),
            }
        }
    }

    /// Takes the person to the item `id`, and ends once it is answered.
    async fn answer_one(&mut self, id: &str) -> Result<ExitCode, Failed> {
        let Some(item) = self.list.identified(id).cloned() else {
            self.human.failure(
                &format!("No item `{id}` waits for a person: `needs-you list` shows those that do"),
                &[("code", ProtocolFailure::InvalidParams.name().to_owned())],
            );
            return Ok(refused());
        };
        for row in self.list.rows().iter().filter(|row| row.item.id == id) {
            for line in row.lines(self.kinds) {
                output::line(&line);
            }
        }
        if self.kinds.answers(&item).is_empty() {
            return Ok(ExitCode::from(exit_codes::SUCCESS));
        }
        let ended = self.answer(&item, "to end").await?;
        self.say_news();
        Ok(match ended {
            Ended::Done | Ended::End => ExitCode::from(exit_codes::SUCCESS),
            Ended::Failed(exit) => exit,
        })
    }

    /// Asks the person how to answer `item` and gives the answer; an empty
    /// line at a choice of answers goes back, as `back` says.
    async fn answer(&mut self, item: &NeedsYouItem, back: &str) -> Result<Ended, Failed> {
        let answers = self.kinds.answers(item);
        let answer = match answers.as_slice() {
            [answer] => *answer,
            answers => match self.choose(item, answers, back).await? {
                Ok(answer) => answer,
                Err(ended) => return Ok(ended),
            },
        };
        let mut typed = String::new();
        if let Some(input) = answer.input {
            typed = match self.ask(&input(item), |list| list.holds(item)).await? {
                Typed::Line(text) => text,
                Typed::End => return Ok(Ended::End),
                Typed::Left => return Ok(Ended::Done),
            };
        }
        let confirmation = match (answer.confirmation)(item, &typed) {
            Ok(confirmation) => confirmation,
            Err(why) => {
                self.human.failure(
                    &why,
                    &[("code", ProtocolFailure::InvalidParams.name().to_owned())],
                );
                return Ok(Ended::Failed(refused()));
            }
        };
        self.confirm(item, answer, confirmation).await
    }

    /// Asks the person which of `answers` to give `item`.
    async fn choose(
        &mut self,
        item: &NeedsYouItem,
        answers: &[&'static ItemAnswer],
        back: &str,
    ) -> Result<Result<&'static ItemAnswer, Ended>, Failed> {
        for (at, answer) in answers.iter().enumerate() {
            output::line(&format!("   {}. {}", at + 1, answer.label));
        }
        let which = format!("Which answer? Its number, or Enter {back}:");
        loop {
            let text = match self.ask(&which, |list| list.holds(item)).await? {
                Typed::Line(text) if text.is_empty() => return Ok(Err(Ended::Done)),
                Typed::Line(text) => text,
                Typed::End => return Ok(Err(Ended::End)),
                Typed::Left => return Ok(Err(Ended::Done)),
            };
            let chosen = text
                .parse::<usize>()
                .ok()
                .and_then(|number| answers.get(number.checked_sub(1)?));
            match chosen {
                Some(answer) => return Ok(Ok(answer)),
                None => output::line(&format!("No answer {text} is offered.")),
            }
        }
    }

    /// Asks the person `confirmation` once, and calls `answer`'s operation
    /// when they answer yes.
    async fn confirm(
        &mut self,
        item: &NeedsYouItem,
        answer: &ItemAnswer,
        confirmation: Confirmation,
    ) -> Result<Ended, Failed> {
        if let Some(warning) = &confirmation.warning {
            self.human.warning(warning);
        }
        let question = prompts::confirmation(&confirmation.question);
        match self.ask(&question, |list| list.holds(item)).await? {
            Typed::Line(text) if prompts::yes(&text) => {}
            Typed::Line(_) => {
                output::line("Nothing was changed.");
                return Ok(Ended::Done);
            }
            Typed::End => return Ok(Ended::End),
            Typed::Left => return Ok(Ended::Done),
        }
        match self.call(answer.operation, confirmation.params).await? {
            Ok(result) => {
                output::fields(&result, self.human);
                if let Some(then) = confirmation.then {
                    output::line(&then);
                }
                self.list.answered(&item.id);
                Ok(Ended::Done)
            }
            Err(error) => Ok(Ended::Failed(output::fail(
                error,
                Some(answer.operation.name),
                false,
                self.human,
            ))),
        }
    }

    /// Asks the person `question` while the list, as the engine changes it,
    /// `stays` one it may be asked on; what changes meanwhile is said, and
    /// the question asked again.
    async fn ask(
        &mut self,
        question: &str,
        stays: impl Fn(&List) -> bool,
    ) -> Result<Typed, Failed> {
        loop {
            self.say_news();
            if !stays(&self.list) {
                return Ok(Typed::Left);
            }
            self.human.prompt(question);
            while self.news.is_empty() {
                tokio::select! {
                    typed = self.typed.next() => {
                        return Ok(match typed {
                            Some(Value::String(line)) => Typed::Line(line),
                            _ => Typed::End,
                        });
                    }
                    message = self.link.heard() => self.take(message?).await?,
                }
            }
            // The news starts after the question's line.
            output::line("");
        }
    }

    /// Calls `operation` with `params` and answers its reply.
    async fn call(
        &mut self,
        operation: &'static Operation,
        params: Value,
    ) -> Result<Result<Value, ErrorObject>, Failed> {
        self.calling.set(operation.name);
        self.id += 1;
        self.link.begin(self.id, operation.name, params).await?;
        let id = RequestId::Number(self.id);
        loop {
            match self.link.heard().await? {
                Message::Response(response) if response.id == id => {
                    return Ok(Ok(response.result));
                }
                // An error whose id is null answers the message the engine
                // could not read, this call.
                Message::Error(failure)
                    if failure.id.is_none() || failure.id.as_ref() == Some(&id) =>
                {
                    return Ok(Err(failure.error));
                }
                message => self.take(message).await?,
            }
        }
    }

    /// Takes a message no call waits for: the subscription's events, or its
    /// end, which ends the session.
    async fn take(&mut self, message: Message) -> Result<(), Failed> {
        match message {
            Message::Notification(Notification {
                method,
                params: Some(params),
                ..
            }) if method == EVENTS => {
                let batch: EventBatch = read(params)?;
                if batch.id != RequestId::Number(SUBSCRIPTION) {
                    return Ok(());
                }
                let credits = u32::try_from(batch.events.len()).unwrap_or(u32::MAX);
                for event in batch.events {
                    self.event(event)?;
                }
                self.link.grant(SUBSCRIPTION, credits).await
            }
            Message::Error(failure) => {
                self.calling.set(EVENTS_SUBSCRIBE.name);
                Err(Failed::Error(failure.error))
            }
            Message::Response(response) if response.id == RequestId::Number(SUBSCRIPTION) => {
                self.calling.set(EVENTS_SUBSCRIBE.name);
                Err(Failed::Error(exit_codes::internal(
                    "The engine ended the session's subscription.".to_owned(),
                )))
            }
            _ => Ok(()),
        }
    }

    /// Takes one event.
    fn event(&mut self, event: EventEnvelope) -> Result<(), Failed> {
        if event.event == RESYNC.name {
            let resync: Resync = read(event.data)?;
            self.news
                .extend(self.list.resync(resync.needs_you, self.kinds));
            self.synced = true;
        } else if event.event == NEEDS_YOU_CHANGED.name {
            let change: NeedsYouChange = read(event.data)?;
            self.news.extend(self.list.change(change, self.kinds));
        }
        Ok(())
    }

    /// Says what changed since it last did.
    fn say_news(&mut self) {
        for line in self.news.drain(..) {
            output::line(&line);
        }
    }
}

/// The exit code of an answer the person cannot give.
fn refused() -> ExitCode {
    ExitCode::from(exit_codes::of_error(
        ProtocolFailure::InvalidParams.name(),
        ErrorGroup::Program,
    ))
}

/// `value` as a `T`, which the engine's schema says it is.
fn read<T: DeserializeOwned>(value: Value) -> Result<T, Failed> {
    serde_json::from_value(value).map_err(|error| {
        Failed::Error(exit_codes::internal(format!(
            "The engine sent an event this executable cannot read: {error}."
        )))
    })
}
