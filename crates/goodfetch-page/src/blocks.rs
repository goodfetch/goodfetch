//! The seam through which a page's document is classified (ADR 0035):
//! Block classification is rule data its Product holds, so the page crate
//! asks the classifier its composition gave for a document's Block cause
//! and never names the rules. The pages read it to tell a Challenge that
//! the Filter lists touched, and a page whose Page check failed for a Block
//! cause, from a page that blocking may have broken (ADR 0047).

use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::page::PageResponse;

/// Who answered a request through the Session's proxy, as the
/// proxy-or-site rule (`crate::proxy`) tells it: a classifier reads only the
/// site's answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Party {
    /// The proxy itself.
    Proxy,
    /// The site, through the proxy.
    Site,
}

/// A Block cause, and the name of the rule that gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The cause.
    pub cause: BlockCause,
    /// The rule's name, which Provenance records.
    pub rule: &'static str,
}

impl Block {
    /// Whether the site checks the browser before it lets it through: a
    /// Challenge or a captcha, which ADR 0047 retries with blocking off
    /// when the Filter lists touched it.
    pub fn checks_the_browser(&self) -> bool {
        matches!(self.cause, BlockCause::Challenge | BlockCause::Captcha)
    }
}

/// Classifies a page's document: the Block cause of the document
/// `response` answered, whose markup the page now holds, if a rule decides
/// one.
pub trait BlockClassifier: Send + Sync {
    /// The Block cause of `response`'s document, whose markup is `markup`,
    /// given by `party` as the proxy-or-site rule told where the engine saw
    /// the answer (ADR 0127); none for a document no rule classifies, and
    /// none for the proxy's own answer, a proxy error and never a Block
    /// cause (ADR 0053).
    fn classify(&self, response: &PageResponse, party: Party, markup: &str) -> Option<Block>;

    /// The markers, lowercase, its rules look for in the document of a
    /// response with `status`, each once; none when no rule reads such a
    /// document, which then needs no reading. Given as `markup` the markers
    /// a document holds, each on a line of its own, it decides as on the
    /// whole document, so a page's document is read for them alone (#711).
    fn markers(&self, status: u16) -> Vec<&'static str> {
        let _ = status;
        Vec::new()
    }
}

/// The classifier of a composition that holds no Block classification
/// rules: no document has a Block cause.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoBlocks;

impl BlockClassifier for NoBlocks {
    fn classify(&self, _response: &PageResponse, _party: Party, _markup: &str) -> Option<Block> {
        None
    }
}
