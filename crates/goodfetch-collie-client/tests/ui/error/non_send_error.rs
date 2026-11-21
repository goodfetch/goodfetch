// A program's own error crosses threads with the SDK's tasks, so `BoxError`
// is `Send + Sync + 'static` (ADR 0109): an error holding an `Rc` does not
// become one, and `?` refuses it.
use std::fmt;
use std::rc::Rc;

use goodfetch_collie_client::BoxError;

#[derive(Debug)]
struct Shared(Rc<str>);

impl fmt::Display for Shared {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Shared {}

fn read() -> Result<u32, Shared> {
    Err(Shared(Rc::from("no count")))
}

fn handler() -> Result<(), BoxError> {
    read()?;
    Ok(())
}

fn main() {
    let _ = handler();
}
