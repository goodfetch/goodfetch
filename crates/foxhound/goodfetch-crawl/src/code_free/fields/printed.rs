//! Texts printed for people, read under a locale's CLDR forms (spec #582;
//! ADR 0123), as pure functions: a `number` written with the locale's
//! digits, decimal separator and grouping, a currency sign or unit beside
//! it; a `date` written in one of the locale's short, medium or long date
//! forms with a four-digit year; and a `date_time` written in one of those
//! forms with the locale's hours and minutes, seconds or not. Each form is
//! learned from ICU4X's CLDR data by formatting known values, so a text
//! reads as the locale prints it, and anything else, a two-digit year, a
//! decimal separator the locale does not use, two numbers, is no value:
//! nothing is guessed.

use fixed_decimal::Decimal;
use icu_calendar::{Date, Gregorian};
use icu_datetime::FixedCalendarDateTimeFormatter;
use icu_datetime::fieldsets::YMD;
use icu_datetime::input::{DateTime, Time};
use icu_decimal::DecimalFormatter;
use icu_decimal::options::DecimalFormatterOptions;
use icu_locale_core::Locale;
use jiff::civil;
use writeable::Writeable as _;

/// The characters that only steer the direction of text, which a printed
/// form may hold and a reader ignores.
const DIRECTION_MARKS: [char; 11] = [
    '\u{200e}', '\u{200f}', '\u{061c}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
    '\u{2066}', '\u{2067}', '\u{2069}',
];

/// The minus signs a printed number may start with besides its locale's.
const MINUS_SIGNS: [&str; 2] = ["-", "\u{2212}"];

/// The year, day, hours, minutes and seconds the known values are printed
/// with, none of them a month's number nor another's.
const YEAR: i32 = 2033;
const DAY: u8 = 22;
const MINUTE: u8 = 45;
const SECOND: u8 = 56;

/// A locale's digits and separators, as it prints a number.
struct Symbols {
    /// Its digits, zero first.
    digits: [char; 10],
    /// Its decimal separator.
    decimal: String,
    /// Its grouping separator.
    group: String,
    /// The minus sign it starts a negative number with.
    minus: String,
}

impl Symbols {
    /// `locale`'s, as ICU4X prints numbers there.
    fn of(locale: &Locale) -> Option<Self> {
        let formatter =
            DecimalFormatter::try_new(locale.into(), DecimalFormatterOptions::default()).ok()?;
        let printed = |number: &str| -> Option<String> {
            let decimal: Decimal = number.parse().ok()?;
            Some(unmarked(&formatter.format(&decimal).write_to_string()))
        };
        let mut digits = ['0'; 10];
        for (digit, slot) in digits.iter_mut().enumerate() {
            let printed = printed(&digit.to_string())?;
            let mut characters = printed.chars();
            *slot = characters.next()?;
            if characters.next().is_some() {
                return None;
            }
        }
        let is_digit = |character: &char| digits.contains(character);
        let separator = |printed: String| -> String {
            printed
                .chars()
                .skip_while(is_digit)
                .take_while(|character| !is_digit(character))
                .collect()
        };
        let decimal = separator(printed("0.5")?);
        let group = separator(printed("1000000")?);
        let minus: String = printed("-1")?
            .chars()
            .take_while(|character| !is_digit(character))
            .collect();
        (!decimal.is_empty()).then_some(Self {
            digits,
            decimal,
            group,
            minus,
        })
    }

    /// `text` with the locale's digits written as ASCII digits.
    fn ascii(&self, text: &str) -> String {
        text.chars()
            .map(|character| {
                self.digits
                    .iter()
                    .position(|digit| *digit == character)
                    .and_then(|digit| char::from_digit(u32::try_from(digit).ok()?, 10))
                    .unwrap_or(character)
            })
            .collect()
    }
}

/// `text` without direction marks, each space-like character a space and
/// each apostrophe-like one an apostrophe, so that a page's no-break or
/// narrow spaces read as the locale's.
fn unmarked(text: &str) -> String {
    text.chars()
        .filter(|character| !DIRECTION_MARKS.contains(character))
        .map(|character| match character {
            '\u{a0}' | '\u{202f}' | '\u{2009}' | '\u{2007}' => ' ',
            '\u{2019}' | '\u{2bc}' => '\'',
            other => other,
        })
        .collect()
}

/// The decimal `text` prints under `locale`, as plain ASCII with a dot
/// (`-1099.98`), for the caller to write as an exact JSON number: one
/// number in the locale's digits, its whole part grouped as the locale
/// groups it or not grouped, its fraction after the locale's decimal
/// separator, with a minus sign before it, and any text without digits,
/// such as a currency sign or a unit, before or after it. `None` for any
/// other text.
pub(super) fn number(text: &str, locale: &str) -> Option<String> {
    let locale: Locale = locale.parse().ok()?;
    let symbols = Symbols::of(&locale)?;
    let text = symbols.ascii(&unmarked(text));
    let first = text.find(|character: char| character.is_ascii_digit())?;
    let last = text.rfind(|character: char| character.is_ascii_digit())?;
    let (before, core) = (&text[..first], &text[first..=last]);
    let before = before.trim_end();
    let minus = unmarked(&symbols.minus);
    // A minus sign counts only where no letter or digit runs into it, so a
    // reference such as `SKU-5` is no negative number.
    let negative = std::iter::once(minus.as_str())
        .filter(|sign| !sign.is_empty())
        .chain(MINUS_SIGNS)
        .any(|sign| {
            before
                .strip_suffix(sign)
                .is_some_and(|rest| !rest.chars().next_back().is_some_and(char::is_alphanumeric))
        });
    let (whole, fraction) = match core.split_once(symbols.decimal.as_str()) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (core, None),
    };
    let whole = ungrouped(whole, &unmarked(&symbols.group))?;
    let fraction = match fraction {
        Some(fraction)
            if !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            format!(".{fraction}")
        }
        Some(_) => return None,
        None => String::new(),
    };
    let sign = if negative { "-" } else { "" };
    Some(format!("{sign}{whole}{fraction}"))
}

/// The digits of `whole`, a number's whole part written with no grouping
/// or grouped by `group`: groups of three after the first, or of two
/// before the last three, as the Indian system groups them.
fn ungrouped(whole: &str, group: &str) -> Option<String> {
    if whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return (!whole.is_empty()).then(|| whole.to_owned());
    }
    if group.is_empty() {
        return None;
    }
    let groups: Vec<&str> = whole.split(group).collect();
    let all_digits = groups
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    let (first, rest) = groups.split_first()?;
    let (last, middle) = rest.split_last()?;
    let shaped = (1..=3).contains(&first.len())
        && last.len() == 3
        && (middle.iter().all(|part| part.len() == 3) || middle.iter().all(|part| part.len() == 2));
    (all_digits && shaped).then(|| groups.concat())
}

/// One piece of a printed date or date and time's form.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    /// Text the form prints as it is, compared without case.
    Text(String),
    /// The year's four digits.
    Year,
    /// The month's number, of one or two digits.
    Month,
    /// The day's number.
    Day,
    /// The hour, 0 to 23.
    Hour,
    /// The hour, 1 to 12, of a form that prints its day period.
    HourOfPeriod,
    /// The minutes.
    Minute,
    /// The seconds.
    Second,
    /// The day period, morning or afternoon, as the form prints it.
    Period,
}

/// A printed form of a date or date and time, and the month its text names
/// when it prints the month's name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Form {
    pieces: Vec<Piece>,
    month: Option<u8>,
    /// The morning and afternoon names of a form with a day period.
    periods: Option<(String, String)>,
}

/// `printed` cut into runs of ASCII digits and runs of anything else, each
/// space run one space, the text lower-cased and trimmed.
fn runs(printed: &str) -> Vec<(bool, String)> {
    let mut runs: Vec<(bool, String)> = Vec::new();
    let collapsed = printed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    for character in collapsed.chars() {
        let digit = character.is_ascii_digit();
        match runs.last_mut() {
            Some((was_digit, run)) if *was_digit == digit => run.push(character),
            _ => runs.push((digit, character.to_string())),
        }
    }
    runs
}

/// The form `printed` is, the known values printed: each run of digits
/// named by the value it prints (`numbers`, each value with its piece);
/// `None` when a run prints none of them, such as a two-digit year.
fn form_of(printed: &str, numbers: &[(u32, Piece)], month: Option<u8>) -> Option<Form> {
    let pieces = runs(printed)
        .into_iter()
        .map(|(digit, run)| {
            if !digit {
                return Some(Piece::Text(run));
            }
            let value: u32 = run.parse().ok()?;
            let width_fits = run.len() <= 2 || value == u32::try_from(YEAR).ok()?;
            numbers
                .iter()
                .find(|(number, _)| *number == value && width_fits)
                .map(|(_, piece)| piece.clone())
        })
        .collect::<Option<Vec<_>>>()?;
    let month = if pieces.contains(&Piece::Month) {
        None
    } else {
        month
    };
    Some(Form {
        pieces,
        month,
        periods: None,
    })
}

/// The date `text` names as `form` prints it, with the time it names:
/// year, month, day, hour, minute and second.
fn read_in(form: &Form, text: &str) -> Option<[u32; 6]> {
    let runs = runs(text);
    if runs.len() != form.pieces.len() {
        return None;
    }
    let mut read = [0, u32::from(form.month.unwrap_or(0)), 0, 0, 0, 0];
    let mut afternoon = false;
    for (piece, (digit, run)) in form.pieces.iter().zip(&runs) {
        let number = || -> Option<u32> {
            if *digit && run.len() <= 2 {
                run.parse().ok()
            } else {
                None
            }
        };
        match piece {
            Piece::Text(text) if run == text => {}
            Piece::Period => match &form.periods {
                Some((morning, _)) if run == morning => {}
                Some((_, evening)) if run == evening => afternoon = true,
                _ => return None,
            },
            Piece::Year if *digit && run.len() == 4 => read[0] = run.parse().ok()?,
            Piece::Text(_) | Piece::Year => return None,
            Piece::Month => read[1] = number()?,
            Piece::Day => read[2] = number()?,
            Piece::Hour => read[3] = number()?,
            Piece::HourOfPeriod => {
                read[3] = number().filter(|hour| (1..=12).contains(hour))? % 12;
            }
            Piece::Minute => read[4] = number()?,
            Piece::Second => read[5] = number()?,
        }
    }
    if afternoon {
        read[3] += 12;
    }
    Some(read)
}

/// The forms `locale` prints a date in: short, medium and long, each with
/// the month's number, or once per month with its name.
fn date_forms(locale: &Locale, symbols: &Symbols) -> Vec<Form> {
    let mut forms: Vec<Form> = Vec::new();
    for length in [YMD::short(), YMD::medium(), YMD::long()] {
        let Ok(formatter) =
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(locale.into(), length)
        else {
            continue;
        };
        for month in 1..=12 {
            let Ok(date) = Date::try_new_gregorian(YEAR, month, DAY) else {
                continue;
            };
            let printed = symbols.ascii(&unmarked(&formatter.format(&date).write_to_string()));
            let numbers = [
                (u32::try_from(YEAR).unwrap_or_default(), Piece::Year),
                (u32::from(DAY), Piece::Day),
                (u32::from(month), Piece::Month),
            ];
            if let Some(form) = form_of(&printed, &numbers, Some(month))
                && !forms.contains(&form)
            {
                forms.push(form);
            }
        }
    }
    forms
}

/// The forms `locale` prints a date and time in: each date form with the
/// hours and minutes, seconds or not, a 12-hour form with its day period.
fn date_time_forms(locale: &Locale, symbols: &Symbols) -> Vec<Form> {
    let mut forms: Vec<Form> = Vec::new();
    let lengths = [
        YMD::short().with_time_hm(),
        YMD::medium().with_time_hm(),
        YMD::long().with_time_hm(),
        YMD::short().with_time_hms(),
        YMD::medium().with_time_hms(),
        YMD::long().with_time_hms(),
    ];
    for length in lengths {
        let Ok(formatter) =
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(locale.into(), length)
        else {
            continue;
        };
        for month in 1..=12_u8 {
            // An evening hour whose 12-hour number is not the month's.
            let hour_of_period: u8 = if month == 7 { 9 } else { 7 };
            let printed = |hour: u8| -> Option<String> {
                let date_time = DateTime {
                    date: Date::try_new_gregorian(YEAR, month, DAY).ok()?,
                    time: Time::try_new(hour, MINUTE, SECOND, 0).ok()?,
                };
                Some(symbols.ascii(&unmarked(&formatter.format(&date_time).write_to_string())))
            };
            let (Some(evening), Some(morning)) =
                (printed(hour_of_period + 12), printed(hour_of_period))
            else {
                continue;
            };
            let numbers = [
                (u32::try_from(YEAR).unwrap_or_default(), Piece::Year),
                (u32::from(DAY), Piece::Day),
                (u32::from(month), Piece::Month),
                (u32::from(hour_of_period + 12), Piece::Hour),
                (u32::from(hour_of_period), Piece::HourOfPeriod),
                (u32::from(MINUTE), Piece::Minute),
                (u32::from(SECOND), Piece::Second),
            ];
            let (Some(mut form), Some(before_noon)) = (
                form_of(&evening, &numbers, Some(month)),
                form_of(&morning, &numbers, Some(month)),
            ) else {
                continue;
            };
            if form.pieces.contains(&Piece::HourOfPeriod) {
                // The day period is the one text the two times print apart.
                let differing: Vec<(String, String)> = form
                    .pieces
                    .iter()
                    .zip(&before_noon.pieces)
                    .filter_map(|pair| match pair {
                        (Piece::Text(after), Piece::Text(morning)) if after != morning => {
                            Some((morning.clone(), after.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                let [(morning, after)] = differing.as_slice() else {
                    continue;
                };
                for piece in &mut form.pieces {
                    if *piece == Piece::Text(after.clone()) {
                        *piece = Piece::Period;
                    }
                }
                form.periods = Some((morning.clone(), after.clone()));
            }
            if !forms.contains(&form) {
                forms.push(form);
            }
        }
    }
    forms
}

/// The ISO 8601 date, `YYYY-MM-DD`, that `text` prints under `locale` in
/// one of its date forms; `None` for any other text or an impossible date.
pub(super) fn date(text: &str, locale: &str) -> Option<String> {
    let locale: Locale = locale.parse().ok()?;
    let symbols = Symbols::of(&locale)?;
    let text = symbols.ascii(&unmarked(text));
    date_forms(&locale, &symbols).iter().find_map(|form| {
        let [year, month, day, ..] = read_in(form, &text)?;
        let date = civil::Date::new(
            i16::try_from(year).ok()?,
            i8::try_from(month).ok()?,
            i8::try_from(day).ok()?,
        )
        .ok()?;
        Some(date.to_string())
    })
}

/// The civil date and time `text` prints under `locale` in one of its
/// date and time forms, which state no offset; `None` for any other text
/// or an impossible date or time.
pub(super) fn date_time(text: &str, locale: &str) -> Option<civil::DateTime> {
    let locale: Locale = locale.parse().ok()?;
    let symbols = Symbols::of(&locale)?;
    let text = symbols.ascii(&unmarked(text));
    date_time_forms(&locale, &symbols).iter().find_map(|form| {
        let [year, month, day, hour, minute, second] = read_in(form, &text)?;
        civil::DateTime::new(
            i16::try_from(year).ok()?,
            i8::try_from(month).ok()?,
            i8::try_from(day).ok()?,
            i8::try_from(hour).ok()?,
            i8::try_from(minute).ok()?,
            i8::try_from(second).ok()?,
            0,
        )
        .ok()
    })
}

#[cfg(test)]
mod tests;
