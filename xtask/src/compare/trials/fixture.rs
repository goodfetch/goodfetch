//! A made-up `Comparison` for the tests that read one: its trials, machine
//! and declarations, built without running a tool.

use super::{Comparison, CountedCell, Declared, GrowthCell, Machine, Measured, Trial};
use crate::compare::matrix::{self, Cell, Scenario};

/// Three trials of each measured cell over the site of `site_pages` pages,
/// or of its scenario's own length, each with the figures `measured` gives
/// its cell and number, on an unnamed machine whose memory was not read.
pub(crate) fn comparison(site_pages: u64, measured: impl Fn(&Cell, u32) -> Measured) -> Comparison {
    let trials = matrix::measured()
        .flat_map(|(cell, config)| {
            let measured = &measured;
            (1..=3).map(move |number| Trial {
                scenario: cell.scenario,
                cell: cell.name.to_owned(),
                tool: config.tool(),
                number,
                site_pages: cell.scenario.site_pages(site_pages),
                load: None,
                retried: None,
                measured: measured(cell, number),
            })
        })
        .collect();
    Comparison {
        declared: Declared {
            trials: 3,
            statistic: "median".to_owned(),
            concurrency: 16,
            site: format!("/sites/{site_pages}/1"),
            spider: "2.53.9".to_owned(),
            crawlee: "3.18.2".to_owned(),
            scrapy: "2.19.0".to_owned(),
            crawlee_rs: "2a3ed90b8391c86005ce6ed8dd3e3248d0d34579".to_owned(),
            playwright: "1.64.1".to_owned(),
            scrapy_playwright: "0.0.48".to_owned(),
            browser: "156.0.8078.4".to_owned(),
            scenarios: Scenario::ALL.to_vec(),
            command: "just compare release".to_owned(),
        },
        machine: Machine {
            class: "local-controlled".to_owned(),
            name: None,
            os: "linux 6.8.0".to_owned(),
            arch: "x86_64".to_owned(),
            cpu: "AMD EPYC-Milan Processor".to_owned(),
            threads: 16,
            memory_bytes: None,
            load: None,
        },
        commit: "0".repeat(40),
        order_seed: 7,
        trials,
        stopped: Vec::new(),
        syscalls: CountedCell::NotRun {
            reason: "a test".to_owned(),
        },
        growth: GrowthCell::NotRun {
            reason: "a test".to_owned(),
        },
        instructions: CountedCell::NotRun {
            reason: "a test".to_owned(),
        },
        rules: Vec::new(),
    }
}
