//! The order file and its check, from the texts the toolchain prints: the
//! merged profile as `llvm-profdata show` lists it, and the executable's
//! layout as LLD's link map places it (`-Map`, with `--no-demangle`, which
//! ci/musl/link-cc passes whenever it links with an order file).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The functions the training ran, hottest first: from `llvm-profdata show
/// --all-functions --counts`, each function's largest block count, those
/// above 0, by symbol (a local function's profile name is
/// `<file>;<symbol>`), each once.
pub(crate) fn hot_functions(shown: &str) -> Vec<String> {
    let mut entries: Vec<(u64, &str)> = Vec::new();
    let mut name: Option<&str> = None;
    for line in shown.lines() {
        if let Some(function) = line
            .strip_prefix("  ")
            .filter(|rest| !rest.starts_with(' '))
            .and_then(|rest| rest.strip_suffix(':'))
        {
            name = Some(function);
        } else if let (Some(function), Some(counts)) = (
            name,
            line.strip_prefix("    Block counts: [")
                .and_then(|rest| rest.strip_suffix(']')),
        ) {
            let hottest = counts
                .split(',')
                .filter_map(|count| count.trim().parse::<u64>().ok())
                .max()
                .unwrap_or_default();
            let symbol = function.rsplit(';').next().unwrap_or(function);
            entries.push((hottest, symbol));
            name = None;
        }
    }
    // Stable: equal counts keep the profile's order.
    entries.sort_by_key(|(count, _)| std::cmp::Reverse(*count));
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter(|(count, symbol)| *count > 0 && seen.insert(*symbol))
        .map(|(_, symbol)| symbol.to_owned())
        .collect()
}

/// An input section as the link map places it.
#[derive(Debug)]
pub(crate) struct Placed {
    /// The output section it is in: `.text`, `.rodata`, `.data.rel.ro`.
    pub(crate) output: String,
    /// The file it comes from: an object, or an archive's member as
    /// `<archive>(<member>)`; `<internal>` for a section LLD made.
    pub(crate) file: String,
    /// Its name in that file.
    pub(crate) section: String,
    /// Its address in the linked image, and its size.
    pub(crate) address: u64,
    pub(crate) size: u64,
    /// The symbols the map lists in it, with their addresses.
    pub(crate) symbols: Vec<(u64, String)>,
}

impl Placed {
    /// Whether ci/musl/link-cc can name the section, which has no symbol
    /// of its own, for the order file: a constant's section in a Rust
    /// object.
    fn named_by_link_cc(&self) -> bool {
        self.file.ends_with(".rcgu.o")
            && (self.section.starts_with(".rodata.") || self.section.starts_with(".data.rel.ro."))
    }
}

/// The input sections of the link map `map`, in its order, which is the
/// image's.
pub(crate) fn placed(map: &str) -> Vec<Placed> {
    let mut sections: Vec<Placed> = Vec::new();
    let mut output = String::new();
    for line in map.lines() {
        // LLD prints `%16llx %16llx %8llx %5llu ` (64-bit), then the name
        // indented 0, 8 or 16 spaces; a wider size widens its column. The
        // header line's columns are no numbers.
        let Some(([address, _, size, _], rest)) = columns(line) else {
            continue;
        };
        let (Ok(address), Ok(size)) = (
            u64::from_str_radix(address, 16),
            u64::from_str_radix(size, 16),
        ) else {
            continue;
        };
        let indent = rest.len() - rest.trim_start().len();
        let text = rest.trim();
        match indent {
            0 => text.clone_into(&mut output),
            8 => {
                if let Some((file, section)) = text
                    .strip_suffix(')')
                    .and_then(|text| text.rsplit_once(":("))
                {
                    sections.push(Placed {
                        output: output.clone(),
                        file: file.to_owned(),
                        section: section.to_owned(),
                        address,
                        size,
                        symbols: Vec::new(),
                    });
                }
            }
            16 => {
                if let Some(section) = sections.last_mut() {
                    section.symbols.push((address, text.to_owned()));
                }
            }
            _ => {}
        }
    }
    sections
}

/// A map line's four number columns and what follows the single space
/// after the fourth, its indentation kept.
fn columns(line: &str) -> Option<([&str; 4], &str)> {
    let mut found = [""; 4];
    let mut rest = line;
    for slot in &mut found {
        let field = rest.trim_start();
        let end = field.find(' ')?;
        *slot = &field[..end];
        rest = &field[end..];
    }
    Some((found, rest.strip_prefix(' ')?))
}

/// The function symbols `.text` holds, `(address, name)`, in the map's
/// order.
pub(crate) fn text_functions(placed: &[Placed]) -> Vec<(u64, String)> {
    placed
        .iter()
        .filter(|section| section.output == ".text")
        .flat_map(|section| section.symbols.iter().cloned())
        .collect()
}

/// The input sections on the touched pages, in the order the training first
/// touched them: `touched` maps each touched page (the image's address over
/// `page`) to the sample it was first seen in; sections first seen in one
/// sample keep the image's order.
pub(crate) fn touched_sections<'a>(
    placed: &'a [Placed],
    touched: &BTreeMap<u64, u32>,
    page: u64,
) -> Vec<&'a Placed> {
    let mut sections: Vec<(u32, &Placed)> = placed
        .iter()
        .filter(|section| section.size > 0)
        .filter_map(|section| {
            let pages = section.address / page..=(section.address + section.size - 1) / page;
            touched
                .range(pages)
                .map(|(_, sample)| *sample)
                .min()
                .map(|sample| (sample, section))
        })
        .collect();
    sections.sort_by_key(|(sample, _)| *sample);
    sections.into_iter().map(|(_, section)| section).collect()
}

/// The order file: the functions on the `touched` sections first, those
/// the profile counted in `hot`'s order (hottest first) and then the others
/// in touch order; then the rest of `hot`, the other workloads' code, which
/// no touching crawl ran; then the touched sections' data. A data section
/// lists its symbols, or, with none, its own name when ci/musl/link-cc can
/// name it (a Rust object's constant); one LLD made (merged strings) or
/// another file's cannot be ordered. Each entry once.
pub(crate) fn order_file(hot: &[String], touched: &[&Placed]) -> Vec<String> {
    let mut functions = Vec::new();
    let mut data = Vec::new();
    for section in touched {
        let names = section.symbols.iter().map(|(_, name)| name.clone());
        match section.output.as_str() {
            ".text" => functions.extend(names),
            ".rodata" | ".data.rel.ro" | ".data" | ".bss" => {
                if section.symbols.is_empty() {
                    if section.named_by_link_cc() {
                        data.push(section.section.clone());
                    }
                } else {
                    data.extend(names);
                }
            }
            _ => {}
        }
    }
    let ran: HashSet<&str> = functions.iter().map(String::as_str).collect();
    let (hot_ran, hot_not_ran): (Vec<&String>, Vec<&String>) = hot
        .iter()
        .partition(|function| ran.contains(function.as_str()));
    let mut seen = HashSet::new();
    hot_ran
        .into_iter()
        .cloned()
        .chain(functions)
        .chain(hot_not_ran.into_iter().cloned())
        .chain(data)
        .filter(|entry| seen.insert(entry.clone()))
        .collect()
}

/// How many of the order file's functions linked in the executable the
/// check follows: the hottest, which start `.text`.
pub(crate) const FOLLOWED: usize = 64;

/// How `.text` departs from the order file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Departure {
    /// No function of the order file is in `.text`: the order was dropped,
    /// or the file is another build's.
    NoneLinked,
    /// `.text` starts with `found`, where the order file's first linked
    /// function is `expected`.
    NotFirst { found: String, expected: String },
    /// The order file's `index`th linked function, `function`, lies before
    /// the one the file lists ahead of it, `before`.
    OutOfOrder {
        index: usize,
        function: String,
        before: String,
    },
}

/// How `functions`, `.text`'s `(address, name)`, depart from `order`, the
/// order file, over its first [`FOLLOWED`] functions linked once (a name
/// linked twice, two files' local functions, has no single place): none
/// when `.text` starts with them in the file's order.
pub(crate) fn departure(order: &[String], functions: &[(u64, String)]) -> Option<Departure> {
    let mut addresses = HashMap::<&str, Option<u64>>::new();
    for (address, name) in functions {
        addresses
            .entry(name)
            .and_modify(|once| *once = None)
            .or_insert(Some(*address));
    }
    let followed: Vec<(&str, u64)> = order
        .iter()
        .filter_map(|name| Some((name.as_str(), addresses.get(name.as_str()).copied()??)))
        .take(FOLLOWED)
        .collect();
    let Some(&(expected, start)) = followed.first() else {
        return Some(Departure::NoneLinked);
    };
    if let Some((lowest, found)) = functions.iter().min_by_key(|(address, _)| *address)
        && *lowest < start
    {
        return Some(Departure::NotFirst {
            found: found.clone(),
            expected: expected.to_owned(),
        });
    }
    followed
        .windows(2)
        .position(|pair| pair[1].1 < pair[0].1)
        .map(|position| Departure::OutOfOrder {
            index: position + 1,
            function: followed[position + 1].0.to_owned(),
            before: followed[position].0.to_owned(),
        })
}

/// The crate a Rust symbol's path starts from, and its disambiguator,
/// which rustc derives from the crate's `-C metadata`: from the v0 mangling
/// `_R`, its namespaces (`N<ns>`), then `C[s<base-62>_]<length>[_]<name>`
/// (an `_` comes before a name that starts with a digit or an `_`). None
/// for a symbol whose path starts elsewhere (an impl's method, a generic
/// instance) or that is no Rust symbol: every crate has functions of the
/// first kind.
pub(crate) fn crate_root(symbol: &str) -> Option<(&str, &str)> {
    let mut rest = symbol.strip_prefix("_R")?;
    while let Some(namespace) = rest.strip_prefix('N') {
        rest = namespace.get(1..)?;
    }
    rest = rest.strip_prefix('C')?;
    let (disambiguator, rest) = match rest.strip_prefix('s') {
        Some(hashed) => hashed.split_once('_')?,
        None => ("", rest),
    };
    let digits = rest.find(|character: char| !character.is_ascii_digit())?;
    let length: usize = rest[..digits].parse().ok()?;
    let rest = &rest[digits..];
    let name = rest.strip_prefix('_').unwrap_or(rest);
    Some((name.get(..length)?, disambiguator))
}

/// The crates the order file names under an identity the build has not,
/// while the build links them under another: their crate identity changed
/// since the profile was trained (another release, or a build that does
/// not compile through ci/musl/stable-metadata/rustc), so the profile's
/// records and the order file miss all of their code.
pub(crate) fn stale_crates<'a>(order: &'a [String], built: &HashSet<&str>) -> BTreeSet<&'a str> {
    let mut linked: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (name, disambiguator) in built.iter().filter_map(|symbol| crate_root(symbol)) {
        linked.entry(name).or_default().insert(disambiguator);
    }
    order
        .iter()
        .filter_map(|symbol| crate_root(symbol))
        .filter(|(name, disambiguator)| {
            linked
                .get(name)
                .is_some_and(|identities| !identities.contains(disambiguator))
        })
        .map(|(name, _)| name)
        .collect()
}

/// How many of the order file's symbols the build holds, of how many.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Share {
    pub(crate) matched: usize,
    pub(crate) total: usize,
}

/// The order file's symbols that `built` holds: its section names (a
/// constant's `.rodata..Lanon.<hash>.<n>`) are left out, since their
/// counters shift with any change to the code before them.
pub(crate) fn match_share(order: &[String], built: &HashSet<&str>) -> Share {
    let symbols = order.iter().filter(|entry| !entry.starts_with('.'));
    let (matched, total) = symbols.fold((0, 0), |(matched, total), symbol| {
        (
            matched + usize::from(built.contains(symbol.as_str())),
            total + 1,
        )
    });
    Share { matched, total }
}

#[cfg(test)]
mod tests;
