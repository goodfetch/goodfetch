//! Texts printed for people, read under a locale's CLDR forms (ADR 0123):
//! each case's expected value is the value the text prints, written by
//! hand, and each text no value is a form the locale does not print.

use rstest::rstest;

use super::{date, date_time, number};

/// A number printed with its locale's digits, separators and grouping,
/// with a currency sign or a unit beside it, reads as the decimal it
/// prints; a minus sign makes it negative, a dash a letter runs into does
/// not.
#[rstest]
#[case::de_de_grouped_with_a_currency_sign("1.099,98\u{a0}€", "de-DE", "1099.98")]
#[case::en_us_grouped("$1,234,567.89", "en-US", "1234567.89")]
#[case::en_us_not_grouped("1234.5 kg", "en-US", "1234.5")]
#[case::fr_fr_grouped_with_a_narrow_space("1\u{202f}234,56 €", "fr-FR", "1234.56")]
#[case::fr_fr_grouped_with_a_space("1 234,56 €", "fr-FR", "1234.56")]
#[case::de_ch_grouped_with_an_apostrophe("CHF 1’234.50", "de-CH", "1234.50")]
#[case::hi_in_in_the_indian_grouping("₹12,34,567.00", "hi-IN", "1234567.00")]
#[case::ar_eg_in_arabic_indic_digits("١٢٬٣٤٥٫٦٧ ج.م.\u{200f}", "ar-EG", "12345.67")]
#[case::negative("-12,5 %", "de-DE", "-12.5")]
#[case::an_integer("Preis: 42 €", "de-DE", "42")]
#[case::a_dash_after_a_letter_is_no_minus_sign("SKU-5", "en-US", "5")]
fn a_printed_number_reads_as_its_locale_prints_it(
    #[case] text: &str,
    #[case] locale: &str,
    #[case] read: &str,
) {
    assert_eq!(number(text, locale).as_deref(), Some(read));
}

/// A text that is not one number as the locale prints it is no number: a
/// decimal separator the locale does not use, groups it does not make, two
/// numbers, no digit, groups the locale does not make.
#[rstest]
#[case::another_locales_separators("1,099.98 €", "de-DE")]
#[case::a_group_of_two("1,5", "en-US")]
#[case::two_numbers("2 for 10 €", "en-US")]
#[case::no_digit("free", "en-US")]
#[case::groups_of_one("1.2.3", "de-DE")]
fn a_text_the_locale_does_not_print_as_a_number_is_none(#[case] text: &str, #[case] locale: &str) {
    assert_eq!(number(text, locale), None);
}

/// A date printed in one of its locale's short, medium or long forms
/// with a four-digit year reads as the date it prints.
#[rstest]
#[case::de_de_long("1. Februar 2026", "de-DE", "2026-02-01")]
#[case::de_de_numeric("01.02.2026", "de-DE", "2026-02-01")]
#[case::en_us_long("February 1, 2026", "en-US", "2026-02-01")]
#[case::en_us_medium("Feb 1, 2026", "en-US", "2026-02-01")]
#[case::en_gb_long("1 February 2026", "en-GB", "2026-02-01")]
#[case::fr_fr_long_any_case("1 Février 2026", "fr-FR", "2026-02-01")]
#[case::ru_long_in_the_genitive("1 февраля 2026 г.", "ru", "2026-02-01")]
#[case::ja_long("2026年2月1日", "ja", "2026-02-01")]
#[case::ar_eg_in_arabic_indic_digits("١\u{200f}/٢\u{200f}/٢٠٢٦", "ar-EG", "2026-02-01")]
fn a_printed_date_reads_as_its_locale_prints_it(
    #[case] text: &str,
    #[case] locale: &str,
    #[case] read: &str,
) {
    assert_eq!(date(text, locale).as_deref(), Some(read));
}

/// A date the locale does not print so is no date: a two-digit year,
/// another locale's order, an impossible date, text around it.
#[rstest]
#[case::a_two_digit_year("01.02.26", "de-DE")]
#[case::another_locales_order("02/13/2026", "en-GB")]
#[case::an_impossible_date("30. Februar 2026", "de-DE")]
#[case::text_around_it("Published 1 February 2026", "en-GB")]
fn a_text_the_locale_does_not_print_as_a_date_is_none(#[case] text: &str, #[case] locale: &str) {
    assert_eq!(date(text, locale), None);
}

/// A date and time printed in one of its locale's forms reads as the civil
/// date and time it prints, a 12-hour time by its day period.
#[rstest]
#[case::de_de_long("1. Februar 2026 um 10:00", "de-DE", "2026-02-01T10:00:00")]
#[case::ar_eg_afternoon_in_arabic_indic_digits(
    "١ فبراير ٢٠٢٦ في ٣:٠٥ م",
    "ar-EG",
    "2026-02-01T15:05:00"
)]
#[case::de_de_with_seconds("01.02.2026, 10:00:30", "de-DE", "2026-02-01T10:00:30")]
#[case::en_us_afternoon("Feb 1, 2026, 3:05 PM", "en-US", "2026-02-01T15:05:00")]
#[case::en_us_morning("February 1, 2026 at 9:05 AM", "en-US", "2026-02-01T09:05:00")]
#[case::en_us_noon("Feb 1, 2026, 12:00 PM", "en-US", "2026-02-01T12:00:00")]
#[case::en_us_midnight("Feb 1, 2026, 12:00 AM", "en-US", "2026-02-01T00:00:00")]
fn a_printed_date_and_time_reads_as_its_locale_prints_it(
    #[case] text: &str,
    #[case] locale: &str,
    #[case] read: &str,
) {
    assert_eq!(
        date_time(text, locale)
            .map(|read| read.to_string())
            .as_deref(),
        Some(read)
    );
}

/// A date and time the locale does not print so is none: a time past the
/// day's hours, a 12-hour time without its day period.
#[rstest]
#[case::an_impossible_time("1. Februar 2026, 25:00", "de-DE")]
#[case::no_day_period("Feb 1, 2026, 3:05", "en-US")]
fn a_text_the_locale_does_not_print_as_a_date_and_time_is_none(
    #[case] text: &str,
    #[case] locale: &str,
) {
    assert_eq!(date_time(text, locale), None);
}
