use super::instructions_of;

/// The shape Callgrind 3.26 writes: a header, each function's costs, then the
/// total of the one event it counted.
const OUTPUT: &str = "# callgrind format\n\
    version: 1\n\
    creator: callgrind-3.26.0\n\
    pid: 7\n\
    cmd:  ./tool\n\
    positions: line\n\
    events: Ir\n\
    summary: 1234567\n\
    \n\
    fn=(1) main\n\
    0 100\n\
    \n\
    totals: 1234567\n";

#[test]
fn the_total_is_read_from_callgrinds_totals_line() {
    assert_eq!(instructions_of(OUTPUT).unwrap(), 1_234_567);
}

#[test]
fn an_output_with_no_total_is_an_error() {
    assert!(instructions_of("events: Ir\nfn=(1) main\n").is_err());
}
