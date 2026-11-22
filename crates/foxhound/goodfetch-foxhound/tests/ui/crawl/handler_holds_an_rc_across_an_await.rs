// A handler's future runs inside `serve`, whose future is `Send` (ADRs 0081
// and 0093): a handler holding an `Rc` across an await does not compile.
use std::rc::Rc;

use goodfetch_foxhound::{Crawl, CrawlContext};

#[derive(serde::Serialize, serde::Deserialize)]
struct Visit {
    depth: u32,
}

fn main() {
    let mut crawl = Crawl::new();
    let page = crawl.label::<Visit>("page").unwrap();
    let _ = crawl.handle(&page, |ctx: CrawlContext<Visit>| async move {
        let depth = Rc::new(ctx.payload().depth);
        std::future::ready(()).await;
        drop(depth);
        Ok(())
    });
}
