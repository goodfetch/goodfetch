//! The `launch` pages on `launch.test` (issue 2330): minimal pages that
//! exercise Chromium launch switches reconciled against Playwright's list on
//! the pinned Chromium build. No task: tests read the DOM and the Proving
//! ground's request observations.
//!
//! - `GET /ipc-flood`: runs 500 `history.replaceState` calls in a tight loop
//!   and writes the final `location.hash` into `#result`, exercising
//!   `--disable-ipc-flooding-protection`.
//! - `GET /popup`: calls `window.open("/popup-target")` on load without a user
//!   gesture and writes `opened` or `blocked` into `#result`, exercising
//!   `--disable-popup-blocking`.
//! - `GET /popup-target`: the page `/popup` opens.
//! - `GET /repost`: renders a `POST` form targeting `/repost`; `POST /repost`
//!   answers `200 OK` (no redirect) with `#result` set to `posted`, so
//!   reloading it resubmits the form, exercising `--disable-prompt-on-repost`.

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};

/// The host name the pages are served on.
pub(crate) const HOST: &str = "launch.test";

/// Answers a request to `launch.test`; `None` for any other host.
pub(crate) fn respond(name: &str, request: &Request<'_>) -> Option<Reply> {
    if name != HOST {
        return None;
    }
    let reply = match (request.method, request.path) {
        (&Method::GET, "/ipc-flood") => html(
            "IPC flood",
            "<main><p id=\"result\">pending</p></main>\n\
             <script>\n\
             for (let i = 0; i < 500; i += 1) {\n\
               history.replaceState(null, \"\", \"#i=\" + i);\n\
             }\n\
             document.getElementById(\"result\").textContent = location.hash;\n\
             </script>\n",
        ),
        (&Method::GET, "/popup") => html(
            "Popup",
            "<main><p id=\"result\">pending</p></main>\n\
             <script>\n\
             const opened = window.open(\"/popup-target\");\n\
             document.getElementById(\"result\").textContent = opened ? \"opened\" : \"blocked\";\n\
             </script>\n",
        ),
        (&Method::GET, "/popup-target") => {
            html("Popup target", "<main><p id=\"popup\">reached</p></main>\n")
        }
        (&Method::GET, "/repost") => html(
            "Repost form",
            "<main>\n\
             <form method=\"post\" action=\"/repost\">\n\
               <input name=\"item\" value=\"book\">\n\
               <button id=\"send\" type=\"submit\">Send</button>\n\
             </form>\n\
             </main>\n",
        ),
        (&Method::POST, "/repost") => {
            html("Reposted", "<main><p id=\"result\">posted</p></main>\n")
        }
        _ => Reply::not_found(),
    };
    Some(reply)
}

fn html(title: &str, body: &str) -> Reply {
    Reply {
        status: StatusCode::OK,
        content_type: "text/html; charset=utf-8",
        headers: Vec::new(),
        body: format!(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><link rel=\"icon\" href=\"data:,\"><title>{title}</title></head>\n<body>\n{body}</body>\n</html>\n"
        )
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(host: &str, method: &Method, path: &str) -> Option<(StatusCode, String)> {
        static EMPTY: hyper::body::Bytes = hyper::body::Bytes::new();
        let request = Request {
            method,
            path,
            query: "",
            cookies: "",
            authorization: "",
            headers: &hyper::HeaderMap::new(),
            body: &EMPTY,
            peer: std::net::IpAddr::from([127, 0, 0, 1]),
            tls: None,
        };
        respond(host, &request).map(|reply| {
            (
                reply.status,
                String::from_utf8(reply.body.to_vec()).expect("utf-8 html"),
            )
        })
    }

    #[test]
    fn launch_test_serves_ipc_flood_popup_and_repost_pages() {
        let (status, flood) = send(HOST, &Method::GET, "/ipc-flood").unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(flood.contains("history.replaceState"));

        let (status, popup) = send(HOST, &Method::GET, "/popup").unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(popup.contains("window.open(\"/popup-target\")"));

        let (status, target) = send(HOST, &Method::GET, "/popup-target").unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(target.contains("reached"));

        let (status, form) = send(HOST, &Method::GET, "/repost").unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(form.contains("method=\"post\""));

        let (status, posted) = send(HOST, &Method::POST, "/repost").unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(posted.contains("posted"));

        assert_eq!(
            send(HOST, &Method::GET, "/missing").unwrap().0,
            StatusCode::NOT_FOUND
        );
        assert!(send("other.test", &Method::GET, "/ipc-flood").is_none());
    }
}
