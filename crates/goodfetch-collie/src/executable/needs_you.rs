//! `needs-you answer` (spec #1937; ADRs 0042, 0069 and 0149): taking a
//! person at a terminal through what waits for them, each item answered by
//! its kind's prompt, in place. It is written by hand, not generated: no
//! operation answers an item, since each kind's prompt calls the person-only
//! operations the item's `actions` name. Under `--json`, or when stdin is no
//! terminal, as under a script, an agent or CI, it is refused with
//! `person_required` before any engine call, while `needs-you list` reads
//! normally.
//!
//! Which prompt a kind gets is data: [`ItemPrompt`]s, Collie's kinds' in the
//! `kinds` module and another Product's passed by its executable among its
//! [`OwnCommands`](super::OwnCommands), so the command names no kind of
//! another Product's. A kind with no prompt is listed with what can be done
//! about it ([`list`]). The session itself is [`session`]'s.

use std::cell::Cell;
use std::process::ExitCode;

use clap::Subcommand;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::needs_you::{NEEDS_YOU_LIST, NeedsYouItem};
use serde_json::Value;

use super::catalog_commands::{Joined, Joining};
use super::daemon::Machine;
use super::engine_choice::{self, EngineChoice, EngineFlags, Link};
use super::operation::failed;
use super::output::Human;
use super::prompts;
use kinds::Kinds;
use session::Session;

mod kinds;
mod list;
mod session;

/// The prompt of one kind of "Needs you" item: the answers a person gives
/// there, and what they do elsewhere.
pub struct ItemPrompt {
    /// The kind, as its source names it.
    pub kind: &'static str,
    /// The answers, each offered when the item's `actions` hold its
    /// operation.
    pub answers: &'static [ItemAnswer],
    /// What a person does about an item outside the engine, as a phrase
    /// such as "free space on /data"; none where its answers are all there
    /// is.
    pub outside: fn(&NeedsYouItem) -> Option<String>,
}

/// One answer to an item: a person-only operation, asked once.
pub struct ItemAnswer {
    /// The answer as the person chooses it, such as "Raise the Limit".
    pub label: &'static str,
    /// The operation a yes calls.
    pub operation: &'static Operation,
    /// The question whose typed answer the confirmation takes, such as a
    /// Limit's new amount; none for an answer that takes none to an item.
    pub input: fn(&NeedsYouItem) -> Option<String>,
    /// The confirmation asked for an item and what the person typed, empty
    /// for an answer that takes nothing, or why it cannot be asked.
    pub confirmation: fn(&NeedsYouItem, &str) -> Result<Confirmation, String>,
}

/// What a person confirms before an answer is given.
pub struct Confirmation {
    /// The yes or no question.
    pub question: String,
    /// What to warn of first, for an answer that widens what the person
    /// takes on (ADR 0052).
    pub warning: Option<String>,
    /// The params the answer's operation is called with.
    pub params: Value,
    /// What the person may do once it is given, if anything.
    pub then: Option<String>,
}

// The `needs-you` group's commands: the catalog's `needs_you` commands,
// listed first, then its own. No doc comment: clap would make it the
// group's help.
#[derive(Subcommand)]
pub(super) enum NeedsYouCommand {
    #[command(flatten)]
    Generated(Joining<OwnNeedsYouCommand>),
    #[command(flatten)]
    Own(OwnNeedsYouCommand),
}

// The `needs-you` group's hand-written commands; no doc comment either.
#[derive(Subcommand)]
pub(super) enum OwnNeedsYouCommand {
    /// Takes a person at the terminal through what waits for them, each item
    /// answered by its kind's prompt; refused with `person_required` under
    /// `--json` or when stdin is no terminal.
    Answer {
        /// The ID of the one item to answer; the whole list when not given.
        item: Option<String>,
        /// Refused: a person answers at a terminal, never under `--json`.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineFlags,
    },
}

impl Joined for OwnNeedsYouCommand {
    const NAMESPACE: &'static str = "needs_you";
}

/// Runs the `needs-you` command `command`, whose session asks the prompts
/// of Collie's kinds and of `own`, the executable's own.
pub(super) fn run(
    command: NeedsYouCommand,
    (engine, dependencies): (&Engine, Dependencies),
    machine: &Machine<'_>,
    own: &'static [ItemPrompt],
    human: &Human,
) -> ExitCode {
    let OwnNeedsYouCommand::Answer {
        item,
        json,
        engine: flags,
    } = match command {
        NeedsYouCommand::Generated(joining) => {
            return joining.call().run(engine, dependencies, machine, human);
        }
        NeedsYouCommand::Own(command) => command,
    };
    let product = engine.identity().product;
    if json || !human.can_ask() {
        let executable = product.executable_name();
        let item = item.map(|item| format!(" {item}")).unwrap_or_default();
        return prompts::refused(
            &format!("{executable} needs-you answer{item}"),
            Some("answer it in the Kennel"),
            None,
            (json, human),
        );
    }
    let warn = |text: &str| human.warning(text);
    let reach = EngineChoice::passing(flags.choice(), machine, &warn);
    let kinds = Kinds { product, own };
    // The operation of the call in flight, which a failure names.
    let calling = Cell::new(NEEDS_YOU_LIST.name);
    let ended = engine_choice::converse(
        engine,
        dependencies,
        product,
        &reach,
        async |link: &mut Link| {
            Session::new(link, kinds, &calling, human)
                .run(item.as_deref())
                .await
        },
    );
    match ended {
        Ok(exit) => exit,
        // A person's session prints its failure on stderr.
        Err(failure) => failed(failure, calling.get(), false, human),
    }
}
