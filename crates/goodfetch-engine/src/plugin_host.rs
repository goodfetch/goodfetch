//! Running process Plugins (ADR 0033): each instance belongs to one run, or
//! to one task outside a run, starts confined with its approved manifest,
//! answers its calls and is gone once its run or task ends. An instance's
//! state changes only through its transition table, [`lifecycle`] (ADR
//! 0128). Its process starts through its launcher, confined ([`process`]).

pub mod lifecycle;
pub mod process;
