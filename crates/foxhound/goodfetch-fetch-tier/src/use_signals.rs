//! The AI-use signals a crawl request's response carries, judged against
//! the Run's Purpose (ADRs 0016, 0045, 0086 and 0093): W3C `TDMRep`'s
//! `tdm-reservation: 1` header, which reserves the text and data mining
//! rights of the page and so refuses every Purpose but `search`, and IETF
//! aipref's `Content-Usage` header, a dictionary of the vocabulary's
//! categories each `y` or `n`, where a narrower category's preference wins
//! over the broader one it falls under (`train-ai` and `ai-use` under
//! `bulk`). A Purpose of `search` is no declaration to judge them against
//! (ADR 0086). Signals read before the request (Content Signals in
//! robots.txt, `TDMRep`'s well-known file), `TDMRep`'s `<meta>` and RSL's
//! `Link` are not judged here yet.

/// What a run declares its results are for (`site_preferences.purpose`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The default, which no signal is judged against.
    Search,
    /// Input to an AI model.
    AiInput,
    /// Training an AI model.
    AiTrain,
    /// Plain collection: text and data mining.
    Collect,
}

impl Purpose {
    /// The Purpose `site_preferences.purpose` names.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "search" => Some(Self::Search),
            "ai-input" => Some(Self::AiInput),
            "ai-train" => Some(Self::AiTrain),
            "collect" => Some(Self::Collect),
            _ => None,
        }
    }

    /// The aipref categories that govern the Purpose, the narrowest first.
    fn categories(self) -> &'static [&'static str] {
        match self {
            Self::Search => &[],
            Self::AiInput => &["ai-use", "bulk"],
            Self::AiTrain => &["train-ai", "bulk"],
            Self::Collect => &["bulk"],
        }
    }
}

/// The AI-use signals of one response's headers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UseSignals {
    /// Whether `TDMRep`'s `tdm-reservation` header reserves the rights.
    reserved: bool,
    /// aipref's `Content-Usage` preferences, by category: `true` for `y`.
    usage: Vec<(String, bool)>,
}

impl UseSignals {
    /// The signals `headers` carry; a value it cannot read says nothing.
    pub fn of(headers: &[(String, String)]) -> Self {
        let mut signals = Self::default();
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("tdm-reservation") {
                signals.reserved |= value.trim() == "1";
            } else if name.eq_ignore_ascii_case("content-usage") {
                signals.usage.extend(value.split(',').filter_map(|member| {
                    let (category, preference) = member.split_once('=')?;
                    let allowed = match preference.trim() {
                        "y" => true,
                        "n" => false,
                        _ => return None,
                    };
                    Some((category.trim().to_ascii_lowercase(), allowed))
                }));
            }
        }
        signals
    }

    /// The signal that refuses `purpose`, named, if any.
    pub fn refusing(&self, purpose: Purpose) -> Option<&'static str> {
        if purpose == Purpose::Search {
            return None;
        }
        if self.reserved {
            return Some("TDMRep's `tdm-reservation: 1` header");
        }
        let stated = purpose.categories().iter().find_map(|category| {
            self.usage
                .iter()
                .rev()
                .find(|(stated, _)| stated == category)
                .map(|(_, allowed)| *allowed)
        });
        (stated == Some(false)).then_some("aipref's `Content-Usage` header")
    }
}

#[cfg(test)]
mod tests;
