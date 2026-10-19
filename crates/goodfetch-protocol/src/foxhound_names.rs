//! The names of every entry only Foxhound carries, generated from the
//! catalog's Product marks by `xtask codegen` (ADRs 0087 and 0095), so that a
//! Collie engine refuses them with `not_in_product` without naming the
//! `foxhound` module.

include!("../generated/foxhound_names.rs");
