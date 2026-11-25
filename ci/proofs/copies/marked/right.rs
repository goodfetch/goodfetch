//! A fixture of the copy check's proof (ci/proofs/run.sh): its function is
//! the same in both files of this folder.

// The proof's deliberate copy (issue 3776): the check must allow a copy
// between jscpd's markers, so this side cannot share the other's code.
// jscpd:ignore-start
pub fn sum_right(text: &str) -> Result<(u64, usize), String> {
    let mut total = 0_u64;
    let mut longest = 0_usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            return Err(format!("line without a value: {trimmed}"));
        };
        let value: u64 = value
            .trim()
            .parse()
            .map_err(|_| format!("{name} is not a number"))?;
        total = total.checked_add(value).ok_or("the total overflows")?;
        longest = longest.max(name.trim().len());
    }
    if longest == 0 {
        return Err("no value at all".to_owned());
    }
    Ok((total, longest))
}
// jscpd:ignore-end
