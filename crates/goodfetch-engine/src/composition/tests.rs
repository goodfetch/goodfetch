//! A handler reaches a Run only through the Runs' registry: no accessor of
//! `CallContext` hands out a `Run` or one of its powers (ADR 0145). It has
//! `runs()`, the registry's handle, and `engine()`, the engine's context.

/// The source of `CallContext`, and of the Runs' module, whose `RunParts`
/// holds a live Run's identity and its powers.
const COMPOSITION: &str = include_str!("../composition.rs");
const RUNS: &str = include_str!("../runs.rs");

/// A live Run's parts and the registry's own record of it, and the type of
/// each of `RunParts`' fields.
fn run_types() -> Vec<String> {
    let mut types: Vec<String> = ["RunParts", "LiveRun", "RunSecrets"]
        .map(str::to_owned)
        .to_vec();
    let start = RUNS
        .find("\npub struct RunParts {\n")
        .expect("invariant: runs.rs declares RunParts");
    let body = &RUNS[start..];
    let end = body
        .find("\n}\n")
        .expect("invariant: RunParts' declaration ends");
    for line in body[..end].lines() {
        let Some(field) = line.trim().strip_prefix("pub ") else {
            continue;
        };
        let Some((_, spelled)) = field.split_once(": ") else {
            continue;
        };
        types.extend(identifiers(spelled).last().map(str::to_owned));
    }
    types
}

/// Each accessor of `CallContext`, by its name, with the type it returns.
fn accessors() -> Vec<(String, String)> {
    let start = COMPOSITION
        .find("\nimpl CallContext {\n")
        .expect("invariant: composition.rs implements CallContext");
    let body = &COMPOSITION[start..];
    let end = body
        .find("\n}\n")
        .expect("invariant: CallContext's impl block ends");
    let mut found = Vec::new();
    let mut rest = &body[..end];
    while let Some(at) = rest.find("fn ") {
        let signature = &rest[at + 3..];
        let open = signature
            .find('{')
            .expect("invariant: every accessor has a body");
        let (head, tail) = signature.split_at(open);
        let name = head.split(['(', '<']).next().unwrap_or_default();
        let returned = head.split_once("->").map_or("", |(_, returned)| returned);
        found.push((name.trim().to_owned(), returned.trim().to_owned()));
        rest = tail;
    }
    found
}

/// The identifiers a type is spelled with.
fn identifiers(spelled: &str) -> impl Iterator<Item = &str> {
    spelled
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|identifier| !identifier.is_empty())
}

#[test]
fn no_accessor_of_a_call_context_hands_out_a_run_or_one_of_its_powers() {
    let run_types = run_types();
    let accessors = accessors();
    for power in [
        "RunIdentity",
        "RunCharging",
        "RunEnd",
        "OwnCopies",
        "RunFiles",
    ] {
        assert!(
            run_types.iter().any(|named| named == power),
            "{power} is read from runs.rs"
        );
    }
    for handle in ["runs", "engine"] {
        assert!(
            accessors.iter().any(|(name, _)| name == handle),
            "{handle} is read"
        );
    }
    let handing_out: Vec<&(String, String)> = accessors
        .iter()
        .filter(|(_, returned)| {
            identifiers(returned)
                .any(|identifier| run_types.iter().any(|named| named == identifier))
        })
        .collect();
    assert_eq!(handing_out, Vec::<&(String, String)>::new());
}
