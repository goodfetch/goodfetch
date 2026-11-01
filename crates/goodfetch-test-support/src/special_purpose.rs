//! IANA's special-purpose address registries (ADR 0067), committed with
//! their date in `goodfetch-protocol`: the blocks they list, whether
//! each is globally reachable, and the addresses a block holds.

use std::net::IpAddr;

/// The blocks of one or more registry files, in their files' order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecialPurposeRegistries {
    /// The blocks; a cell that lists two blocks gives two.
    pub blocks: Vec<SpecialPurposeBlock>,
}

/// One block of a registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecialPurposeBlock {
    /// The block in CIDR notation, as the registry writes it without its
    /// footnote mark.
    pub block: String,
    /// The registry's name for it.
    pub name: String,
    /// Its "Globally Reachable" column.
    pub globally_reachable: Reachability,
}

/// A registry's "Globally Reachable" column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reachability {
    /// `True`.
    Global,
    /// `False`.
    NotGlobal,
    /// `N/A`, or empty for a terminated block.
    Unstated,
}

/// Why a registry file cannot be read.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// The first line is not IANA's header.
    #[error("the registry's first line is not IANA's header")]
    Header,
    /// A row lacks the registry's columns.
    #[error("line {line} of the registry lacks its columns")]
    Row {
        /// The row's line, from 1.
        line: usize,
    },
}

/// The committed registries' text, IPv4's then IPv6's, as IANA publishes
/// them; `crates/goodfetch-protocol/iana/` holds them with their date,
/// beside the classification table `xtask codegen` writes from them.
const COMMITTED: [&str; 2] = [
    include_str!("../../goodfetch-protocol/iana/iana-ipv4-special-registry-1.csv"),
    include_str!("../../goodfetch-protocol/iana/iana-ipv6-special-registry-1.csv"),
];

impl SpecialPurposeRegistries {
    /// The committed IPv4 and IPv6 registries, in that order.
    ///
    /// # Errors
    ///
    /// A [`RegistryError`] when a committed file is not of IANA's form.
    pub fn committed() -> Result<Self, RegistryError> {
        let mut blocks = Vec::new();
        for text in COMMITTED {
            blocks.extend(Self::parse_csv(text)?.blocks);
        }
        Ok(Self { blocks })
    }

    /// The blocks of the registry file `text`, IANA's CSV form.
    ///
    /// # Errors
    ///
    /// A [`RegistryError`] when the text is not of IANA's form.
    pub fn parse_csv(text: &str) -> Result<Self, RegistryError> {
        let mut lines = text.lines();
        if lines.next().map(fields).as_deref() != Some(&HEADER.map(str::to_owned)[..]) {
            return Err(RegistryError::Header);
        }
        let mut blocks = Vec::new();
        for (index, line) in lines.enumerate() {
            let row = || RegistryError::Row { line: index + 2 };
            let cells = fields(line);
            if cells.len() != HEADER.len() {
                return Err(row());
            }
            let globally_reachable = match without_footnote(&cells[GLOBALLY_REACHABLE]) {
                "True" => Reachability::Global,
                "False" => Reachability::NotGlobal,
                "N/A" | "" => Reachability::Unstated,
                _ => return Err(row()),
            };
            for block in without_footnote(&cells[0]).split(", ") {
                blocks.push(SpecialPurposeBlock {
                    block: block.to_owned(),
                    name: cells[1].clone(),
                    globally_reachable,
                });
            }
        }
        Ok(Self { blocks })
    }
}

/// IANA's columns, in its order.
const HEADER: [&str; 10] = [
    "Address Block",
    "Name",
    "RFC",
    "Allocation Date",
    "Termination Date",
    "Source",
    "Destination",
    "Forwardable",
    "Globally Reachable",
    "Reserved-by-Protocol",
];

/// The "Globally Reachable" column's index.
const GLOBALLY_REACHABLE: usize = 8;

/// A CSV line's fields, unquoted.
fn fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match (character, quoted) {
            ('"', true) if characters.peek() == Some(&'"') => {
                characters.next();
                field.push('"');
            }
            ('"', _) => quoted = !quoted,
            (',', false) => fields.push(std::mem::take(&mut field)),
            (character, _) => field.push(character),
        }
    }
    fields.push(field);
    fields
}

/// A cell without IANA's trailing footnote mark, such as ` [1]`.
fn without_footnote(cell: &str) -> &str {
    cell.split(" [").next().unwrap_or(cell)
}

/// A block of addresses: an address and a prefix length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    address: IpAddr,
    prefix: u8,
}

impl Cidr {
    /// The block `text` writes in CIDR notation, if it is one.
    pub fn parse(text: &str) -> Option<Self> {
        let (address, prefix) = text.split_once('/')?;
        let address: IpAddr = address.parse().ok()?;
        let prefix: u8 = prefix.parse().ok()?;
        let bits = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        (prefix <= bits).then_some(Self { address, prefix })
    }

    /// Whether the block holds `address`, of the same family.
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.address, address) {
            (IpAddr::V4(block), IpAddr::V4(address)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(block) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(block), IpAddr::V6(address)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(block) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
