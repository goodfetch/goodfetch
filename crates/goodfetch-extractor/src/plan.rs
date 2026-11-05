//! The demand plan of a page's analysis (ADR 0138): a pure function of
//! what the request reads and of the Run's settings that gives the readers
//! to run on the page, and no other. Each reader declares what it produces
//! and which readers it needs, so a request that reads nothing from the
//! Clean-up rules runs no clean-up, and a Run with `frontier.learning` off
//! runs no Page signature.
//!
//! The demand holds the formats and `frontier.learning` today; the fields,
//! Datasets, `follow` and the tier join it as their readers move onto the
//! Page analysis (spec #1577's tickets).

use goodfetch_protocol::page::{FormatsRequest, HtmlKind, MarkdownContent};

/// What a page's analysis is asked for.
#[derive(Clone, Copy, Debug)]
pub struct Demand<'a> {
    /// The formats the request names.
    pub formats: &'a FormatsRequest,
    /// Whether the Run learns from its pages (`frontier.learning`): its
    /// Page signature, which a learned rule compares.
    pub learning: bool,
}

/// One reader of a page's analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reader {
    /// One streaming pass over the page as it stands: its links and head.
    Untouched,
    /// The Clean-up rules' cosmetic removal (ADR 0047), the page's shadow
    /// roots composed after: what every reader of the cleaned page reads.
    CleanUp,
    /// The cleaned page's images and media: a streaming pass over it, the
    /// untouched page's when the rules removed nothing.
    CleanedAssets,
    /// One tree of the cleaned page: its tables, structured data and
    /// contacts.
    CleanedTree,
    /// The main content of the cleaned page: one run of the Run's chosen
    /// Extractor.
    MainContent,
    /// The Page signature (ADR 0005): the title and the main text of the
    /// page as it stands, one run of the main-content step.
    PageSignature,
}

/// What a reader produces: a format of the request, or the Page signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Product {
    /// `links`.
    Links,
    /// `metadata`, its head's part, or its structured data's.
    Metadata,
    /// `html.cleaned`.
    CleanedHtml,
    /// `markdown` of the whole body.
    FullMarkdown,
    /// `images`.
    Images,
    /// `media`.
    Media,
    /// `tables`.
    Tables,
    /// `contacts`.
    Contacts,
    /// `main_content`, `markdown` of the main content or its `fit`,
    /// `html.main` and `text`.
    MainContent,
    /// The Page signature.
    PageSignature,
}

impl Reader {
    /// Every reader, in the order a page's analysis runs them.
    pub const ALL: [Self; 6] = [
        Self::Untouched,
        Self::PageSignature,
        Self::CleanUp,
        Self::CleanedAssets,
        Self::CleanedTree,
        Self::MainContent,
    ];

    /// What it produces.
    pub fn produces(self) -> &'static [Product] {
        match self {
            Self::Untouched => &[Product::Links, Product::Metadata],
            Self::CleanUp => &[Product::CleanedHtml, Product::FullMarkdown],
            Self::CleanedAssets => &[Product::Images, Product::Media],
            Self::CleanedTree => &[Product::Tables, Product::Metadata, Product::Contacts],
            Self::MainContent => &[Product::MainContent],
            Self::PageSignature => &[Product::PageSignature],
        }
    }

    /// The readers whose result it reads.
    pub fn needs(self) -> &'static [Self] {
        match self {
            Self::Untouched | Self::CleanUp | Self::PageSignature => &[],
            Self::CleanedAssets | Self::CleanedTree | Self::MainContent => &[Self::CleanUp],
        }
    }
}

impl Demand<'_> {
    /// Whether it asks for `product`.
    fn asks(&self, product: Product) -> bool {
        let formats = self.formats;
        match product {
            Product::Links => formats.links.is_some(),
            Product::Metadata => formats.metadata.is_some(),
            Product::CleanedHtml => formats
                .html
                .is_some_and(|format| format.kind() == HtmlKind::Cleaned),
            Product::FullMarkdown => formats
                .markdown
                .is_some_and(|format| format.content() == MarkdownContent::Full),
            Product::Images => formats.images.is_some(),
            Product::Media => formats.media.is_some(),
            Product::Tables => formats.tables.is_some(),
            Product::Contacts => formats.contacts.is_some(),
            Product::MainContent => formats.reads_main_content(),
            Product::PageSignature => self.learning,
        }
    }
}

/// The readers a page's analysis runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// One bit per reader, at its place in [`Reader::ALL`].
    readers: u8,
}

impl Plan {
    /// The readers `demand` needs: each that produces what it asks for,
    /// and each those need.
    pub fn of(demand: &Demand<'_>) -> Self {
        let mut plan = Self::default();
        for reader in Reader::ALL {
            if reader
                .produces()
                .iter()
                .any(|product| demand.asks(*product))
            {
                plan.add(reader);
            }
        }
        plan
    }

    fn add(&mut self, reader: Reader) {
        self.readers |= Self::bit(reader);
        for needed in reader.needs() {
            self.add(*needed);
        }
    }

    fn bit(reader: Reader) -> u8 {
        let place = Reader::ALL
            .iter()
            .position(|listed| *listed == reader)
            .expect("invariant: `Reader::ALL` lists every reader");
        1 << place
    }

    /// Whether it runs `reader`.
    pub fn runs(self, reader: Reader) -> bool {
        self.readers & Self::bit(reader) != 0
    }

    /// The readers it runs, in their order.
    pub fn readers(self) -> impl Iterator<Item = Reader> {
        Reader::ALL
            .into_iter()
            .filter(move |reader| self.runs(*reader))
    }
}
