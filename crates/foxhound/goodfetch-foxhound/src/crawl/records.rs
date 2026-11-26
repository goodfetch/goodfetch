//! A crawl's Records read back (ADRs 0103 and 0104): each committed
//! Record as its Dataset's own type, a [`DatasetRecord`], and each entry of
//! a Dataset's rejection table, page after page.

use std::collections::VecDeque;
use std::future::IntoFuture as _;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};
use std::task::{Context, Poll, ready};
use std::time::SystemTime;

use futures_util::{Stream, StreamExt};
use goodfetch_collie_client::{ClientConnection, Error, EventStream, Reply};
use goodfetch_protocol::foxhound::dataset::{
    RECORD, RECORD_INVALID, RUN_RECORDS, RecordRead, RunRecordsParams,
};
use goodfetch_protocol::foxhound::dataset_view::{
    DATASET_LIST, DATASET_PAGE, DatasetList, DatasetListParams, DatasetPage, DatasetPageParams,
};
use goodfetch_protocol::foxhound::provenance::RecordProvenance;
use goodfetch_protocol::foxhound::rejections::{
    RUN_REJECTED_RECORDS, RejectedRecord, RejectedRecords, RunRejectedRecordsParams,
};
use goodfetch_protocol::run::RunId;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::handles::one_invalid_record;
use crate::sync::Mutex;

/// One committed Record of a Dataset, as a crawl's Run reads it back: its
/// value of the Dataset's own type `I`, which `I`'s `Deserialize` parsed
/// from the Record's JSON, and where and when it was committed (ADR 0103).
///
/// A field of the Record that `I` does not declare is kept as `I` keeps
/// it: a `#[serde(flatten)]` map holds each one, a `Value` holds them all,
/// and a struct without either drops them (G3).
#[derive(Clone, Debug, PartialEq)]
pub struct DatasetRecord<I = Value> {
    /// Its number in its Dataset, from 1, in commit order, with no gaps,
    /// which a later read's `after` takes.
    pub number: u64,
    /// The Record, of its Dataset's type.
    pub value: I,
    /// The ID of the crawl request whose attempt committed it.
    pub request_id: String,
    /// That crawl request's URL.
    pub url: String,
    /// When it was committed.
    pub committed_at: SystemTime,
    /// How it was obtained (ADR 0123's Provenance of a Record).
    pub provenance: RecordProvenance,
}

/// Where a read of a crawl's Records stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::crawl) enum Reach {
    /// It follows a live Run, reading each Record once it commits, and
    /// ends once the Run has ended: the program's own read.
    Follows,
    /// It reads what had committed when it was sent, then ends, a live
    /// Run's included: a handler's read through its context, which the
    /// Run its attempt holds would never end under (ADR 0103).
    Committed,
}

/// The read of one Dataset's committed Records that
/// [`CrawlRun::records`](crate::CrawlRun::records) and
/// [`AttemptRun::records`](crate::AttemptRun::records) return: a stream of
/// each Record as the Dataset's type `I`, sent once it is first polled.
/// Dropping it cancels the read, and leaves the Run as it is.
#[derive(Debug)]
#[must_use = "a read sends nothing until it is polled"]
pub struct ReadRecords<I> {
    connection: ClientConnection,
    params: RunRecordsParams,
    reach: Reach,
    /// The read, once polled.
    read: Option<EventStream<DatasetRecord<I>>>,
}

impl<I> ReadRecords<I> {
    /// The read of `dataset`'s Records in the Run `run_id`, from the first.
    pub(in crate::crawl) fn new(
        connection: ClientConnection,
        run_id: RunId,
        dataset: &str,
        reach: Reach,
    ) -> Self {
        Self {
            connection,
            params: RunRecordsParams {
                run_id,
                dataset: dataset.to_owned(),
                after: None,
                through: None,
                wall: None,
            },
            reach,
            read: None,
        }
    }

    /// Reads the Records numbered past `after`, the number of the last
    /// Record a reader handled: it resumes a reader, past a Record its
    /// type refused included. Set before the read is first polled.
    pub fn after(mut self, after: u64) -> Self {
        self.params.after = Some(after);
        self
    }
}

impl<I: DeserializeOwned + Send + Sync + 'static> ReadRecords<I> {
    /// The next Record, or `None` once the read has ended; the first call
    /// sends the read.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: dropping the future before it completes loses no
    /// Record, and the read it sent stays sent.
    pub async fn next(&mut self) -> Option<Result<DatasetRecord<I>, Error>> {
        StreamExt::next(self).await
    }

    /// Sends the read. A read of what had committed first asks
    /// `dataset.page` for the last Record committed, the bound the catalog
    /// declares for `run.records`, and reads through it.
    pub(crate) fn sent(self) -> EventStream<DatasetRecord<I>> {
        let Self {
            connection,
            mut params,
            reach,
            read,
        } = self;
        if let Some(read) = read {
            return read;
        }
        let dataset = params.dataset.clone();
        let reads = match reach {
            Reach::Follows => connection.stream::<RecordRead>(RUN_RECORDS.name, &params, RECORD),
            Reach::Committed => {
                let last = connection
                    .call::<DatasetPage>(
                        DATASET_PAGE.name,
                        &DatasetPageParams {
                            run_id: params.run_id.clone(),
                            dataset: params.dataset.clone(),
                            after: None,
                            through: None,
                            filter: None,
                            sort: None,
                            columns: Some(Vec::new()),
                            limit: Some(1),
                        },
                    )
                    .into_future();
                bounded(last, move |page: DatasetPage| {
                    params.through = Some(page.through);
                    connection.stream::<RecordRead>(RUN_RECORDS.name, &params, RECORD)
                })
            }
        };
        typed(&dataset, reads)
    }
}

impl<I: DeserializeOwned + Send + Sync + 'static> Stream for ReadRecords<I> {
    type Item = Result<DatasetRecord<I>, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let records = self.get_mut();
        let read = records.read.get_or_insert_with(|| {
            let unsent = Self {
                connection: records.connection.clone(),
                params: records.params.clone(),
                reach: records.reach,
                read: None,
            };
            unsent.sent()
        });
        read.poll_next_unpin(context)
    }
}

/// The read of one Dataset's rejection table that
/// [`CrawlRun::rejected_records`](crate::CrawlRun::rejected_records) and
/// [`AttemptRun::rejected_records`](crate::AttemptRun::rejected_records)
/// return: a stream of its entries, read page after page from
/// `run.rejected_records`, its first page asked once it is first polled.
/// Dropping it cancels the page being read, and leaves the Run as it is.
#[derive(Debug)]
#[must_use = "a read sends nothing until it is polled"]
pub struct ReadRejectedRecords {
    connection: ClientConnection,
    params: RunRejectedRecordsParams,
    reach: Reach,
    /// The read, once polled.
    read: Option<EventStream<RejectedRecord>>,
}

impl ReadRejectedRecords {
    /// The read of `dataset`'s rejection table in the Run `run_id`, from
    /// the first entry.
    pub(in crate::crawl) fn new(
        connection: ClientConnection,
        run_id: RunId,
        dataset: &str,
        reach: Reach,
    ) -> Self {
        Self {
            connection,
            params: RunRejectedRecordsParams {
                run_id,
                dataset: dataset.to_owned(),
                after: None,
                through: None,
            },
            reach,
            read: None,
        }
    }

    /// Reads the entries numbered past `after`, the number of the last
    /// entry a reader handled. Set before the read is first polled.
    pub fn after(mut self, after: u64) -> Self {
        self.params.after = Some(after);
        self
    }

    /// The next entry, or `None` once the read has ended; the first call
    /// asks the first page.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: dropping the future before it completes loses no
    /// entry, and the page being read stays asked.
    pub async fn next(&mut self) -> Option<Result<RejectedRecord, Error>> {
        StreamExt::next(self).await
    }

    /// Sends the read. A read of what had committed first asks
    /// `dataset.list` how many entries the table holds, the last one's
    /// number since they have no gaps, and reads through it.
    pub(crate) fn sent(self) -> EventStream<RejectedRecord> {
        let Self {
            connection,
            mut params,
            reach,
            read,
        } = self;
        if let Some(read) = read {
            return read;
        }
        match reach {
            Reach::Follows => rejected(connection, params),
            Reach::Committed => {
                let listed = connection
                    .call::<DatasetList>(
                        DATASET_LIST.name,
                        &DatasetListParams {
                            run_id: params.run_id.clone(),
                        },
                    )
                    .into_future();
                bounded(listed, move |list: DatasetList| {
                    // A Dataset the crawl does not declare reads through
                    // nothing, and `run.rejected_records` refuses it.
                    let rejected_count = list
                        .datasets
                        .iter()
                        .find(|summary| summary.name == params.dataset)
                        .map_or(0, |summary| summary.rejected);
                    params.through = Some(rejected_count);
                    rejected(connection, params)
                })
            }
        }
    }
}

impl Stream for ReadRejectedRecords {
    type Item = Result<RejectedRecord, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let rejected = self.get_mut();
        let read = rejected.read.get_or_insert_with(|| {
            let unsent = Self {
                connection: rejected.connection.clone(),
                params: rejected.params.clone(),
                reach: rejected.reach,
                read: None,
            };
            unsent.sent()
        });
        read.poll_next_unpin(context)
    }
}

/// The stream `open` opens with what `asked` answers: the bound a read
/// reads through. A failed answer is the stream's one item, and ends it.
fn bounded<B: Send + 'static, T: Send + 'static>(
    asked: Reply<B>,
    open: impl FnOnce(B) -> EventStream<T> + Send + Sync + 'static,
) -> EventStream<T> {
    EventStream::from_items(Bounded {
        asked: Some(Mutex::new(asked)),
        open: Some(Box::new(open)),
        read: None,
    })
}

/// The stream [`bounded`] returns.
struct Bounded<B, T> {
    /// The bound, until answered. The lock only makes the stream `Sync`:
    /// the answer is polled through `get_mut`, never locked.
    asked: Option<Mutex<Reply<B>>>,
    open: Option<Box<dyn FnOnce(B) -> EventStream<T> + Send + Sync>>,
    /// The read, once the bound is answered.
    read: Option<EventStream<T>>,
}

impl<B, T> Stream for Bounded<B, T> {
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let bounded = self.get_mut();
        if let Some(asked) = bounded.asked.as_mut() {
            let asked = asked.get_mut().unwrap_or_else(PoisonError::into_inner);
            let answer = ready!(asked.as_mut().poll(context));
            bounded.asked = None;
            let open = bounded.open.take();
            match (answer, open) {
                (Ok(bound), Some(open)) => bounded.read = Some(open(bound)),
                (Err(error), _) => return Poll::Ready(Some(Err(error))),
                (Ok(_), None) => return Poll::Ready(None),
            }
        }
        match bounded.read.as_mut() {
            Some(read) => read.poll_next_unpin(context),
            None => Poll::Ready(None),
        }
    }
}

/// The stream of `dataset`'s Records that `reads` reads, each parsed as
/// `I`: the first that `I` refuses ends it with `record_invalid`, which
/// names that Record's number, and cancels the read.
pub(super) fn typed<I: DeserializeOwned + Send + Sync + 'static>(
    dataset: &str,
    reads: EventStream<RecordRead>,
) -> EventStream<DatasetRecord<I>> {
    EventStream::from_items(Typed {
        reads: Some(reads),
        dataset: Arc::from(dataset),
        record: PhantomData,
    })
}

/// The stream [`typed`] returns.
struct Typed<I> {
    /// The read, dropped, which cancels it, once it ended or failed.
    reads: Option<EventStream<RecordRead>>,
    dataset: Arc<str>,
    record: PhantomData<fn() -> I>,
}

impl<I: DeserializeOwned> Stream for Typed<I> {
    type Item = Result<DatasetRecord<I>, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let typed = self.get_mut();
        let Some(reads) = typed.reads.as_mut() else {
            return Poll::Ready(None);
        };
        let read = ready!(reads.poll_next_unpin(context));
        let record = read.map(|read| read.and_then(|read| parsed(&typed.dataset, read)));
        if !matches!(record, Some(Ok(_))) {
            typed.reads = None;
        }
        Poll::Ready(record)
    }
}

/// `read`'s Record with its value parsed as `I`, or the `record_invalid`
/// naming its number when `I` refuses it.
fn parsed<I: DeserializeOwned>(dataset: &str, read: RecordRead) -> Result<DatasetRecord<I>, Error> {
    let record = read.record;
    let value = serde_json::from_value(record.value).map_err(|error| {
        Error::of_sdk(
            &RECORD_INVALID,
            &format!(
                "the Record {} of the Dataset `{dataset}` does not read as the Dataset's type: {error}",
                record.number
            ),
            "Read the Dataset with a type whose `Deserialize` accepts the Record, or resume past it with `ReadRecords::after`.",
            // Serde names no JSON Pointer, so the issue is at the root.
            &one_invalid_record(dataset, String::new(), error.to_string(), Some(record.number)),
        )
    })?;
    Ok(DatasetRecord {
        number: record.number,
        value,
        request_id: record.request_id,
        url: record.url,
        committed_at: record.committed_at,
        provenance: record.provenance,
    })
}

/// The entries of one Dataset's rejection table, read page after page
/// from `run.rejected_records` until a page says the table ended.
pub(super) fn rejected(
    connection: ClientConnection,
    params: RunRejectedRecordsParams,
) -> EventStream<RejectedRecord> {
    EventStream::from_items(Rejected {
        connection,
        params,
        page: VecDeque::new(),
        asked: None,
        ended: false,
    })
}

/// The stream [`rejected`] returns.
struct Rejected {
    connection: ClientConnection,
    /// The next page's read, its `after` the last page's `next`.
    params: RunRejectedRecordsParams,
    /// The entries of the last page not handed out yet.
    page: VecDeque<RejectedRecord>,
    /// The next page, once asked for. The lock only makes the stream
    /// `Sync`: the page is polled through `get_mut`, never locked.
    asked: Option<Mutex<Reply<RejectedRecords>>>,
    /// Whether no page follows: the last said so, or failed.
    ended: bool,
}

impl Stream for Rejected {
    type Item = Result<RejectedRecord, Error>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let rejected = self.get_mut();
        loop {
            if let Some(entry) = rejected.page.pop_front() {
                return Poll::Ready(Some(Ok(entry)));
            }
            if rejected.ended {
                return Poll::Ready(None);
            }
            let (connection, params) = (&rejected.connection, &rejected.params);
            let asked = rejected.asked.get_or_insert_with(|| {
                // While the Run is live, the engine waits for the next
                // commit: the read has no deadline, and dropping it
                // cancels it.
                Mutex::new(
                    connection
                        .call::<RejectedRecords>(RUN_REJECTED_RECORDS.name, params)
                        .no_timeout()
                        .into_future(),
                )
            });
            let page = asked.get_mut().unwrap_or_else(PoisonError::into_inner);
            let page = ready!(page.as_mut().poll(context));
            rejected.asked = None;
            match page {
                Ok(page) => {
                    rejected.params.after = Some(page.next);
                    rejected.ended = page.ended;
                    rejected.page.extend(page.records);
                }
                Err(error) => {
                    rejected.ended = true;
                    return Poll::Ready(Some(Err(error)));
                }
            }
        }
    }
}
