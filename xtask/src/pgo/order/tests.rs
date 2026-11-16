use std::collections::BTreeMap;

use super::*;

/// `llvm-profdata show --all-functions --counts`, as it prints a profile:
/// a local function's name carries its file before a `;`.
const SHOWN: &str = "\
Counters:
  _RNvCs1_4main4warm:
    Hash: 0x0000000000000001
    Counters: 2
    Block counts: [40, 7]
  main.abc-cgu.0;_RNvCs1_4main3hot:
    Hash: 0x0000000000000002
    Counters: 3
    Block counts: [1000, 2, 0]
  _RNvCs1_4main4cold:
    Hash: 0x0000000000000003
    Counters: 1
    Block counts: [0]
  other.def-cgu.0;_RNvCs1_4main4warm:
    Hash: 0x0000000000000004
    Counters: 1
    Block counts: [3]
  _RNvCs1_4main5tepid:
    Hash: 0x0000000000000005
    Counters: 1
    Block counts: [40]
Instrumentation level: Front-end
Functions shown: 5
";

#[test]
fn the_hot_functions_come_hottest_first_each_once_without_the_unrun() {
    assert_eq!(
        hot_functions(SHOWN),
        [
            "_RNvCs1_4main3hot",
            "_RNvCs1_4main4warm",
            "_RNvCs1_4main5tepid"
        ]
    );
}

/// An LLD map (`-Map`, `--no-demangle`): the output sections at no
/// indentation, their input sections at 8 spaces, the symbols at 16.
const MAP: &str = "\
             VMA              LMA     Size Align Out     In      Symbol
            1000             1000      100    16 .rodata
            1000             1000       20     8         /b/main.rcgu.o:(.rodata..Lanon.0a1b.3)
            1020             1020       30     8         <internal>:(.rodata.str1.1)
            1050             1050       10     4         /b/main.rcgu.o:(.rodata._RNvCs1_4main3hot)
            1060             1060       10     8         /b/main.rcgu.o:(.rodata._RNvCs1_4main5TABLE)
            1060             1060       10     1                 _RNvCs1_4main5TABLE
            1070             1070       10     8         /zig/libc.a(malloc.o):(.rodata.sizes)
            2000             2000      100     8 .data.rel.ro
            2000             2000       10     8         /b/main.rcgu.o:(.data.rel.ro..Lanon.0a1b.4)
            3000             3000     3000    16 .text
            3000             3000       40    16         /b/main.rcgu.o:(.text.hot._RNvCs1_4main3hot)
            3000             3000       40     1                 _RNvCs1_4main3hot
            3040             3040       20    16         /zig/libc.a(malloc.o):(.text.malloc)
            3040             3040       20     1                 malloc
            4000             4000       40    16         /b/main.rcgu.o:(.text._RNvCs1_4main4cold)
            4000             4000       40     1                 _RNvCs1_4main4cold
            5000             5000       40    16         /b/main.rcgu.o:(.text._RNvCs1_4main4warm)
            5000             5000       40     1                 _RNvCs1_4main4warm
";

#[test]
fn the_text_functions_are_the_symbols_the_map_places_in_text() {
    let placed = placed(MAP);

    assert_eq!(
        text_functions(&placed),
        [
            (0x3000, "_RNvCs1_4main3hot".to_owned()),
            (0x3040, "malloc".to_owned()),
            (0x4000, "_RNvCs1_4main4cold".to_owned()),
            (0x5000, "_RNvCs1_4main4warm".to_owned()),
        ]
    );
}

#[test]
fn the_order_file_lists_the_touched_code_hottest_first_then_the_other_hot_functions_then_touched_data()
 {
    let placed = placed(MAP);
    // Page 3 (code), touched first; then page 5; then pages 1 and 2 (data).
    // Page 4 is never touched.
    let touched = BTreeMap::from([(3, 0), (5, 1), (1, 2), (2, 2)]);

    let order = order_file(
        &order(&[
            "_RNvCs1_4main4cold",
            "_RNvCs1_4main4warm",
            "_RNvCs1_4main3hot",
        ]),
        &touched_sections(&placed, &touched, 0x1000),
    );

    assert_eq!(
        order,
        [
            // The touched code the profile ran, hottest first.
            "_RNvCs1_4main4warm",
            "_RNvCs1_4main3hot",
            // The touched code it did not count, in touch order.
            "malloc",
            // A hot function no touching crawl ran: another workload's.
            "_RNvCs1_4main4cold",
            // Data: a named symbol, and the sections link-cc names in a Rust
            // object; libc's `.rodata.sizes`, with no symbol, cannot be
            // ordered, and merged strings are LLD's own.
            ".rodata..Lanon.0a1b.3",
            ".rodata._RNvCs1_4main3hot",
            "_RNvCs1_4main5TABLE",
            ".data.rel.ro..Lanon.0a1b.4",
        ]
    );
}

fn order(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn functions(placed: &[(u64, &str)]) -> Vec<(u64, String)> {
    placed
        .iter()
        .map(|&(address, name)| (address, name.to_owned()))
        .collect()
}

#[test]
fn text_that_starts_with_the_order_files_functions_in_its_order_follows_it() {
    let text = functions(&[(0x10, "hot"), (0x20, "warm"), (0x30, "cold")]);

    assert_eq!(departure(&order(&["hot", "inlined", "warm"]), &text), None);
}

#[test]
fn an_alias_at_an_ordered_functions_address_departs_from_nothing() {
    let text = functions(&[(0x10, "hot"), (0x10, "alias"), (0x20, "warm")]);

    assert_eq!(departure(&order(&["hot", "warm"]), &text), None);
}

#[test]
fn text_that_starts_with_an_unordered_function_departs() {
    let text = functions(&[(0x10, "cold"), (0x20, "hot"), (0x30, "warm")]);

    assert_eq!(
        departure(&order(&["hot", "warm"]), &text),
        Some(Departure::NotFirst {
            found: "cold".to_owned(),
            expected: "hot".to_owned()
        })
    );
}

#[test]
fn ordered_functions_out_of_the_files_order_depart() {
    let text = functions(&[(0x10, "hot"), (0x20, "tepid"), (0x30, "warm")]);

    assert_eq!(
        departure(&order(&["hot", "warm", "tepid"]), &text),
        Some(Departure::OutOfOrder {
            index: 2,
            function: "tepid".to_owned(),
            before: "warm".to_owned()
        })
    );
}

#[test]
fn a_function_linked_twice_is_not_followed() {
    // Two files' local functions of one name: no single place to check.
    let text = functions(&[
        (0x10, "hot"),
        (0x20, "warm"),
        (0x30, "local"),
        (0x40, "local"),
    ]);

    assert_eq!(departure(&order(&["hot", "local", "warm"]), &text), None);
}

#[test]
fn text_with_none_of_the_order_files_functions_departs() {
    let text = functions(&[(0x10, "cold")]);

    assert_eq!(
        departure(&order(&["hot"]), &text),
        Some(Departure::NoneLinked)
    );
}

#[test]
fn only_the_first_followed_functions_are_checked() {
    let mut names: Vec<String> = (0..FOLLOWED).map(|index| format!("f{index}")).collect();
    let mut text: Vec<(u64, String)> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (index as u64, name.clone()))
        .collect();
    // Two past the followed ones, the wrong way round.
    names.extend(["late".to_owned(), "later".to_owned()]);
    text.extend([(1000, "later".to_owned()), (1001, "late".to_owned())]);

    assert_eq!(departure(&names, &text), None);
}
