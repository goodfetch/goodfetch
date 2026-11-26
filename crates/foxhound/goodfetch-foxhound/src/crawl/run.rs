//! A crawl's Run once started (ADRs 0093, 0109 and 0112): the [`CrawlRun`]
//! a crawl serves, the stream of its Crawl requests for a program that
//! answers them itself, each a [`CrawlCall`], and its Datasets' Records.

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll, ready};

use futures_util::Stream;
use goodfetch_collie_client::{
    Answer, BoxError, Call, ClientConnection, Error, EventStream, HandlerCall, Reply, Run,
};
use goodfetch_protocol::foxhound::progress::CrawlRunProgress;
use goodfetch_protocol::foxhound::report::CrawlRunReport;
use goodfetch_protocol::run::{RUN_REPORT, RunFinishParams, RunId, RunParams, RunStopParams};
use serde_json::Value;

use super::records::{Reach, ReadRecords, ReadRejectedRecords};
use super::{
    CrawlContext, CrawlId, Dataset, Delivered, Forget, answer, delivered, params_unread, sent,
};
use crate::crawl_requests::CrawlRequests;
use crate::sync::Mutex;

/// A crawl's Run, which [`Crawl::serve`](super::Crawl::serve) serves: the [`Run`] it is, and
/// the connection's Crawl requests its serve reads. Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct CrawlRun {
    pub(super) run: Run,
    pub(super) connection: ClientConnection,
    pub(super) requests: Arc<CrawlRequests>,
    /// The crawl that started the Run, whose Labels its requests name.
    pub(super) crawl: CrawlId,
    pub(super) declared: Arc<[String]>,
}

impl CrawlRun {
    /// The Run's ID.
    pub fn id(&self) -> &RunId {
        self.run.id()
    }

    /// The Run, with every member a Run has, such as `close`.
    pub fn run(&self) -> &Run {
        &self.run
    }

    /// Lets a Run finish and answers its Run report: a Run its program drives
    /// ends at once, `finished` and `complete`; a crawl ends once its work
    /// items are done, and a keep-alive crawl (`crawl.keep_alive`), which waits
    /// for new work while nothing is left, ends once this call lets it and its
    /// work items are done. A Run something stopped first answers its report
    /// saying `stopped`. A Run that ended `failed` fails the call with the
    /// error that failed it, and a write at its end that failed, such as its
    /// Plan file's, with that write's error. Cancelling the call, or its
    /// `timeout` passing, which fails it with `deadline_exceeded`, ends only
    /// the wait.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// As [`Run::finish`], with the report typed as a crawl's (ADRs 0102 and
    /// 0120): its request totals, its Records per Dataset and its
    /// Completeness, each reason naming the setting that capped it and the
    /// count it refused.
    ///
    /// # Errors
    ///
    /// Once sent, as [`Run::finish`]: a Run that ended `failed` fails with
    /// the error that failed it, and the call's timeout passing fails it
    /// with `deadline_exceeded`, ending only the wait.
    pub fn finish(&self) -> Call<Answer<CrawlRunReport>, RunFinishParams> {
        self.run.finish().typed()
    }

    /// Stops a Run at its next safe point with a Checkpoint, `stopped` and
    /// `incomplete` as an abort by its program, resumable by its Run ID, and
    /// answers its Run report; an ended Run's report is answered as it is. The
    /// wait is bounded by `timeout.close`, or the call's `timeout`: past it,
    /// the call fails with `deadline_exceeded` and the Run still stops at its
    /// next safe point. A Run that ended `failed` fails the call as
    /// `run.finish` does.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// As [`Run::stop`], with the report typed as a crawl's.
    ///
    /// # Errors
    ///
    /// Once sent, as [`Run::stop`].
    pub fn stop(&self) -> Call<Answer<CrawlRunReport>, RunStopParams> {
        self.run.stop().typed()
    }

    /// Reads a Run's report: at once for a Run that ended, and at its end for a
    /// live one, which the read never ends. A Run another engine works is read
    /// through that engine, and one no engine works from its committed state,
    /// never taking its writer lock. A Run that has no live writer and has not
    /// ended fails with `run_interrupted`, whose `fix` names a start with its
    /// Run ID, and a Run ID the Run index does not list with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// As [`Run::report`], with the report typed as a crawl's.
    ///
    /// # Errors
    ///
    /// Once sent, as [`Run::report`].
    pub fn report(&self) -> Call<Answer<CrawlRunReport>, RunParams> {
        self.run.report().typed()
    }

    /// Reads a Run's progress: what it does and why it waits. Without `after`,
    /// it answers at once; given `after`, the position of the value the reader
    /// last read, it waits for a change and answers the latest value, so values
    /// in between are coalesced. The value whose `last` is true ends the
    /// sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
    /// or no engine works it (`writer: none`) until a start with its Run ID
    /// resumes it. A Run another engine works is read through that engine, the
    /// owner the Run index names, and one no engine works from its committed
    /// state; a read never takes the Run's writer lock. A Run ID the Run index
    /// does not list fails with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Follows it as [`Run::progress`] does, each value typed as a crawl's:
    /// its request counts, and whether a keep-alive crawl waits idle.
    ///
    /// # Errors
    ///
    /// An item fails as [`Run::progress`]'s do, and with `internal` when a
    /// value does not read as a crawl's progress.
    pub fn progress(&self) -> EventStream<Answer<CrawlRunProgress>> {
        self.run.progress().typed()
    }

    /// The Run's Crawl requests, for a program that answers them itself
    /// (ADRs 0093 and 0112): the stream beneath [`Crawl::serve`](super::Crawl::serve), for a
    /// crawl that attaches no handler, such as one whose state cannot be
    /// `Send`. Each item is a [`CrawlCall`], answered with
    /// [`CrawlCall::finish`]; the stream ends once the Run ends, as the
    /// serve returns then, or once the connection has ended, and the Run's
    /// `report` then reads how it ended. A request whose params do not
    /// read is answered with that error, which is the stream's item in its
    /// place.
    ///
    /// ```no_run
    /// use goodfetch_foxhound::{ConnectOptions, Crawl, Foxhound, Seed};
    ///
    /// # async fn crawl() -> Result<(), goodfetch_foxhound::Error> {
    /// let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    /// let mut crawl = Crawl::new();
    /// let page = crawl.label::<()>("page")?;
    /// let run = foxhound
    ///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &page, ())])
    ///     .await?;
    /// let mut calls = run.crawl_requests();
    /// while let Some(call) = calls.next().await {
    ///     let call = call?;
    ///     let answered = if call.context().url().ends_with(".pdf") {
    ///         Err(call.context().skip("no PDF"))
    ///     } else {
    ///         Ok(())
    ///     };
    ///     call.finish(answered)?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream gives back each request of the Run the
    /// connection kept aside for it, as dropping a serve does; a
    /// [`CrawlCall`] it handed out is given back when it is dropped
    /// unanswered.
    pub fn crawl_requests(&self) -> EventStream<CrawlCall> {
        EventStream::from_items(Reading {
            next: None,
            ended: Some(Mutex::new(self.ended())),
            forget: Forget {
                requests: Arc::clone(&self.requests),
                run_id: self.id().clone(),
            },
            connection: self.connection.clone(),
            run: self.run.clone(),
            crawl: self.crawl,
            declared: Arc::clone(&self.declared),
        })
    }

    /// Reads the committed Records of one Dataset of a crawl's Run in
    /// number order, from the first, or after the number `after`, which
    /// resumes a reader from the last Record it handled. Each Record goes
    /// in a `$/record` notification with its number, its value, its
    /// request's ID and URL and when it was committed, one per credit
    /// granted with `$/credit`. While the Run is live it follows the Run,
    /// sending each Record once it commits; once the Run has ended, or has
    /// no live writer, and every Record is sent, it answers how many it
    /// sent and the last one's number. With `through`, such as the number
    /// `dataset.page` answers as the last committed, it reads no Record
    /// numbered past it and answers once it sent every Record through it, a
    /// live Run's included. Records are read from the Run's committed state
    /// on a connection of their own, so a reader that grants no credit
    /// never slows the crawl. With `wall` set to `true` it reads only the
    /// Records whose page was read past a Wall, which their Provenance
    /// marks, and with `false` only the others (ADR 0052). A Dataset the
    /// crawl does not declare fails with `dataset_unknown`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Each Record is a [`DatasetRecord`] whose value is the Dataset's own
    /// type `I`, which `I`'s `Deserialize` parses from the Record's JSON
    /// (ADR 0103). A field `I` does not declare is kept as `I` keeps it: a
    /// `#[serde(flatten)]` map holds each one, and a Dataset of
    /// [`serde_json::Value`] of the same name, from any [`Crawl`](super::Crawl),
    /// reads each Record's JSON whole. A Record `I` refuses ends the stream
    /// with `record_invalid`, whose details name the Dataset and the
    /// Record's number, with one issue at the root that carries serde's
    /// message; [`ReadRecords::after`] resumes past it.
    ///
    /// ```no_run
    /// use goodfetch_foxhound::{ConnectOptions, Crawl, Foxhound, Seed};
    ///
    /// # async fn crawl() -> Result<(), goodfetch_foxhound::Error> {
    /// let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    /// let mut crawl = Crawl::new();
    /// let page = crawl.label::<()>("page")?;
    /// let titles = crawl.dataset::<String>("titles")?;
    /// crawl.handle(&page, |_ctx| async { Ok(()) })?;
    /// let run = foxhound
    ///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &page, ())])
    ///     .await?;
    /// crawl.serve(&run).await?;
    /// let mut records = run.records(&titles);
    /// while let Some(record) = records.next().await {
    ///     let record = record?;
    ///     println!("{}: {}", record.number, record.value);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// An item is `dataset_unknown` for a Dataset the crawl does not
    /// declare, `record_invalid` for a Record `I` refuses, `closed` once
    /// the connection has ended, and the engine's error when the read
    /// fails, after which the stream ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels the read, and leaves the Run as it is.
    pub fn records<I>(&self, dataset: &Dataset<I>) -> ReadRecords<I> {
        ReadRecords::new(
            self.connection.clone(),
            self.id().clone(),
            &dataset.name,
            Reach::Follows,
        )
    }

    /// Reads one Dataset's rejection table from a crawl's committed state:
    /// each Record rejected rather than committed, numbered from 1 in
    /// commit order, with the Record as JSON, the reason, each issue's
    /// path, the Hook when one rejected it, and the request's ID and URL.
    /// It starts at the first entry, or after `after`, the number a reader
    /// last handled. While the Run is live and nothing follows, it waits
    /// for the next commit; a page says `ended` once the Run has ended, or
    /// has no live writer, and no entry follows. With `through`, such as
    /// the `rejected` count `dataset.list` answers, it reads no entry
    /// numbered past it and never waits, a live Run's included: a page says
    /// `ended` once it reaches that number or has read every committed
    /// entry. A reader never slows the Run. A
    /// Dataset the crawl does not declare fails with `dataset_unknown`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// Each entry is a [`RejectedRecord`]; the stream reads the table page
    /// after page and ends with the page that says it ended (ADR 0104).
    /// [`ReadRejectedRecords::after`] sets `after`; a handler reads through
    /// [`AttemptRun::rejected_records`](crate::AttemptRun::rejected_records),
    /// which sets `through`.
    ///
    /// # Errors
    ///
    /// An item is `dataset_unknown` for a Dataset the crawl does not
    /// declare, `closed` once the connection has ended, and the engine's
    /// error when a page's read fails, after which the stream ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels the page being read, and leaves the Run
    /// as it is.
    pub fn rejected_records<I>(&self, dataset: &Dataset<I>) -> ReadRejectedRecords {
        ReadRejectedRecords::new(
            self.connection.clone(),
            self.id().clone(),
            &dataset.name,
            Reach::Follows,
        )
    }

    /// The Run's report, which the engine answers once the Run ends.
    pub(super) fn ended(&self) -> Reply<CrawlRunReport> {
        self.connection
            .call::<CrawlRunReport>(
                RUN_REPORT.name,
                &RunParams {
                    run_id: self.id().clone(),
                },
            )
            .no_timeout()
            .into_future()
    }
}

/// The next Crawl request of a Run, or `None` once the connection ended.
type NextRequest = Pin<Box<dyn Future<Output = Option<HandlerCall>> + Send + Sync>>;

/// The stream of a Run's Crawl requests, which ends with the Run.
struct Reading {
    /// The read of the Run's next request, once one was asked for: dropped
    /// before `forget`, so no request it read stays kept for the Run.
    next: Option<NextRequest>,
    /// The Run's report, whose answer ends the stream, or `None` once the
    /// stream ended. The lock only makes the stream `Sync`: the report is
    /// polled through `get_mut`, never locked.
    ended: Option<Mutex<Reply<CrawlRunReport>>>,
    forget: Forget,
    connection: ClientConnection,
    run: Run,
    crawl: CrawlId,
    declared: Arc<[String]>,
}

impl Stream for Reading {
    type Item = Result<CrawlCall, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let reading = self.get_mut();
        let Some(ended) = reading.ended.as_mut() else {
            return Poll::Ready(None);
        };
        let report = ended.get_mut().unwrap_or_else(PoisonError::into_inner);
        if report.as_mut().poll(context).is_ready() {
            reading.next = None;
            reading.ended = None;
            return Poll::Ready(None);
        }
        let forget = &reading.forget;
        let next = reading.next.get_or_insert_with(|| {
            let requests = Arc::clone(&forget.requests);
            let run_id = forget.run_id.clone();
            Box::pin(async move { requests.next(&run_id).await })
        });
        let call = ready!(next.as_mut().poll(context));
        reading.next = None;
        let Some(call) = call else {
            reading.ended = None;
            return Poll::Ready(None);
        };
        Poll::Ready(Some(reading.call_of(call)))
    }
}

impl Reading {
    /// The [`CrawlCall`] of `call`, or the refusal of params that do not
    /// read, which answers it.
    fn call_of(&self, call: HandlerCall) -> Result<CrawlCall, Error> {
        match delivered(
            &call,
            &self.connection,
            &self.run,
            self.crawl,
            &self.declared,
        ) {
            Ok(Delivered { attempt, payload }) => Ok(CrawlCall {
                call,
                context: CrawlContext::new(attempt, payload),
            }),
            Err(error) => {
                sent(answer(call, None, Err(Box::new(params_unread(&error)))));
                Err(params_unread(&error))
            }
        }
    }
}

/// One Crawl request of a Run that the program answers itself, read from
/// [`CrawlRun::crawl_requests`] (ADRs 0093 and 0109): its attempt's
/// context, whose Payload is the request's JSON, and the answer
/// [`CrawlCall::finish`] gives. Dropped unanswered, it is given back to
/// the engine: the request returns to pending without counting an
/// attempt, and the Run delivers it again.
#[derive(Debug)]
pub struct CrawlCall {
    call: HandlerCall,
    context: CrawlContext<Value>,
}

impl CrawlCall {
    /// The attempt's context, as a handler is given it, with the request's
    /// Payload as JSON.
    pub fn context(&self) -> &CrawlContext<Value> {
        &self.context
    }

    /// Answers the attempt with `result`, as a handler's return does: `Ok`
    /// commits what the attempt pushed; an `Err` fails it with
    /// `handler_failed`, a Goodfetch [`Error`] keeping its own code; a
    /// [`CrawlContext::skip`] or [`CrawlContext::retry`] called on the
    /// context ends it as it asked, whatever `result` is (ADR 0109).
    ///
    /// # Errors
    ///
    /// `closed` once the connection has ended.
    pub fn finish(self, result: Result<(), BoxError>) -> Result<(), Error> {
        answer(self.call, self.context.attempt.ending(), result)
    }
}
