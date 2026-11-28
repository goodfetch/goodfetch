//! Handlers for SDK-only cookies and Web Storage (`session.cookies`,
//! `session.add_cookies`, `session.clear_cookies`, `page.local_storage`,
//! `page.session_storage`; ADR 0092; ticket #746).

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::storage::{
    AddCookiesParams, ClearCookiesParams, Cookie, NewCookie, PAGE_LOCAL_STORAGE,
    PAGE_SESSION_STORAGE, PageStorageParams, PageStorageResult, SESSION_ADD_COOKIES,
    SESSION_CLEAR_COOKIES, SESSION_COOKIES, SessionCookiesChanged, SessionCookiesParams,
    SessionCookiesResult, WebStorageAction,
};
use url::Url;

use super::{Pages, in_call};
use crate::approvals::SourceDomains;
use crate::sessions::Sessions;

/// Adds the SDK-only cookie and Web Storage handlers to `contribution`.
pub(super) fn with_storage_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let cookies_sessions = pages.sessions.clone();
    let add_sessions = pages.sessions.clone();
    let clear_sessions = pages.sessions.clone();
    let local_pages = pages.clone();
    let session_pages = pages.clone();
    contribution
        .with_handler(Handler::new(
            &SESSION_COOKIES,
            move |params: SessionCookiesParams, context: CallContext| {
                let sessions = cookies_sessions.clone();
                in_call((SESSION_COOKIES.name, context.form()), async move {
                    session_cookies(&sessions, params).await
                })
            },
        ))
        .with_handler(Handler::new(
            &SESSION_ADD_COOKIES,
            move |params: AddCookiesParams, context: CallContext| {
                let sessions = add_sessions.clone();
                in_call((SESSION_ADD_COOKIES.name, context.form()), async move {
                    session_add_cookies(&sessions, params).await
                })
            },
        ))
        .with_handler(Handler::new(
            &SESSION_CLEAR_COOKIES,
            move |params: ClearCookiesParams, context: CallContext| {
                let sessions = clear_sessions.clone();
                in_call((SESSION_CLEAR_COOKIES.name, context.form()), async move {
                    session_clear_cookies(&sessions, params).await
                })
            },
        ))
        .with_handler(Handler::new(
            &PAGE_LOCAL_STORAGE,
            move |params: PageStorageParams, context: CallContext| {
                let pages = local_pages.clone();
                in_call((PAGE_LOCAL_STORAGE.name, context.form()), async move {
                    page_web_storage(&pages, params, true).await
                })
            },
        ))
        .with_handler(Handler::new(
            &PAGE_SESSION_STORAGE,
            move |params: PageStorageParams, context: CallContext| {
                let pages = session_pages.clone();
                in_call((PAGE_SESSION_STORAGE.name, context.form()), async move {
                    page_web_storage(&pages, params, false).await
                })
            },
        ))
}

async fn session_cookies(
    sessions: &Sessions,
    params: SessionCookiesParams,
) -> Result<SessionCookiesResult, CallError> {
    let source_domains = sessions.source_domains_of(&params.session_id)?;
    let parsed_urls = match params.urls {
        Some(urls) => {
            let mut parsed = Vec::with_capacity(urls.len());
            for raw in &urls {
                let url = Url::parse(raw).map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InvalidParams,
                        format!("invalid cookie filter URL `{raw}`: {error}"),
                    )
                })?;
                if let Some(bounds) = &source_domains {
                    bounds
                        .check_url(raw)
                        .map_err(|domain| bounds.refused_error(&domain, Some(raw)))?;
                }
                parsed.push(url);
            }
            Some(parsed)
        }
        None => None,
    };
    let all = sessions.get_cookies(&params.session_id).await?;
    let cookies = all
        .into_iter()
        .filter(|cookie| {
            source_domains
                .as_ref()
                .is_none_or(|bounds| bounds.check_domain(&cookie.domain).is_ok())
        })
        .filter(|cookie| {
            parsed_urls
                .as_ref()
                .is_none_or(|urls| urls.iter().any(|url| cookie_matches_url(cookie, url)))
        })
        .collect();
    Ok(SessionCookiesResult { cookies })
}

async fn session_add_cookies(
    sessions: &Sessions,
    params: AddCookiesParams,
) -> Result<SessionCookiesChanged, CallError> {
    let source_domains = sessions.source_domains_of(&params.session_id)?;
    for cookie in &params.cookies {
        validate_new_cookie(cookie, source_domains.as_deref())?;
    }
    sessions
        .add_cookies(&params.session_id, params.cookies)
        .await?;
    Ok(SessionCookiesChanged {})
}

async fn session_clear_cookies(
    sessions: &Sessions,
    params: ClearCookiesParams,
) -> Result<SessionCookiesChanged, CallError> {
    let source_domains = sessions.source_domains_of(&params.session_id)?;
    if let (Some(bounds), Some(domain)) = (&source_domains, params.domain.as_deref()) {
        bounds
            .check_domain(domain)
            .map_err(|unlisted| bounds.refused_error(&unlisted, None))?;
    }
    let clear_all = source_domains.is_none()
        && params.name.is_none()
        && params.domain.is_none()
        && params.path.is_none();
    sessions
        .clear_cookies(&params.session_id, clear_all, |cookie| {
            if let Some(bounds) = &source_domains
                && bounds.check_domain(&cookie.domain).is_err()
            {
                return false;
            }
            if let Some(name) = params.name.as_deref()
                && cookie.name != name
            {
                return false;
            }
            if let Some(domain) = params.domain.as_deref()
                && !cookie_domain_filter_matches(&cookie.domain, domain)
            {
                return false;
            }
            if let Some(path) = params.path.as_deref()
                && cookie.path != path
            {
                return false;
            }
            true
        })
        .await?;
    Ok(SessionCookiesChanged {})
}

async fn page_web_storage(
    pages: &Pages,
    params: PageStorageParams,
    is_local: bool,
) -> Result<PageStorageResult, CallError> {
    let (page, _spent) = pages
        .open(&params.page_id, |page| {
            page.driver.navigation_deadline(None)
        })
        .await?;
    if let Some(bounds) = pages.sessions.source_domains_of(&page.session)? {
        let href = page.driver.current_location_href().await?;
        bounds
            .check_url(&href)
            .map_err(|domain| bounds.refused_error(&domain, Some(&href)))?;
    }
    let result = page.driver.web_storage(is_local, &params.action).await?;
    if is_local
        && matches!(
            params.action,
            WebStorageAction::SetItem { .. }
                | WebStorageAction::RemoveItem { .. }
                | WebStorageAction::Clear
        )
    {
        pages.sessions.persist_page_state(&page).await;
    }
    Ok(result)
}

fn validate_new_cookie(
    cookie: &NewCookie,
    source_domains: Option<&SourceDomains>,
) -> Result<(), CallError> {
    match (&cookie.url, &cookie.domain, &cookie.path) {
        (Some(raw_url), None, None) => {
            let parsed = Url::parse(raw_url).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!(
                        "cookie `{}` has invalid `url` `{raw_url}`: {error}",
                        cookie.name
                    ),
                )
            })?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!(
                        "cookie `{}` `url` must be an HTTP(S) URL with a host: `{raw_url}`",
                        cookie.name
                    ),
                ));
            }
            if let Some(bounds) = source_domains {
                bounds
                    .check_url(raw_url)
                    .map_err(|domain| bounds.refused_error(&domain, Some(raw_url)))?;
            }
            Ok(())
        }
        (None, Some(domain), Some(_path)) => {
            if domain.trim().is_empty() {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("cookie `{}` `domain` must not be empty", cookie.name),
                ));
            }
            if let Some(bounds) = source_domains {
                bounds
                    .check_domain(domain)
                    .map_err(|unlisted| bounds.refused_error(&unlisted, None))?;
            }
            Ok(())
        }
        _ => Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "cookie `{}` must specify either `url`, or both `domain` and `path` (not both)",
                cookie.name
            ),
        )),
    }
}

fn cookie_domain_filter_matches(cookie_domain: &str, filter: &str) -> bool {
    if cookie_domain.eq_ignore_ascii_case(filter) {
        return true;
    }
    !filter.starts_with('.')
        && cookie_domain
            .trim_start_matches('.')
            .eq_ignore_ascii_case(filter)
}

/// RFC 6265 domain, path, and secure-scheme matching for `session.cookies(urls)`.
fn cookie_matches_url(cookie: &Cookie, url: &Url) -> bool {
    if cookie.secure && url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host_lower = host.to_ascii_lowercase();
    let domain_lower = cookie.domain.to_ascii_lowercase();
    let domain_matches = if let Some(bare) = domain_lower.strip_prefix('.') {
        host_lower == bare
            || (host_lower.len() > bare.len()
                && host_lower.ends_with(bare)
                && host_lower.as_bytes()[host_lower.len() - bare.len() - 1] == b'.')
    } else {
        host_lower == domain_lower
    };
    if !domain_matches {
        return false;
    }
    let req_path = url.path();
    let cookie_path = if cookie.path.is_empty() {
        "/"
    } else {
        cookie.path.as_str()
    };
    if req_path == cookie_path {
        return true;
    }
    if req_path.starts_with(cookie_path) {
        return cookie_path.ends_with('/')
            || req_path.as_bytes().get(cookie_path.len()) == Some(&b'/');
    }
    false
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::page::storage::CookieSameSite;

    use super::*;

    fn sample_cookie(domain: &str, path: &str, secure: bool) -> Cookie {
        Cookie {
            name: "sid".to_owned(),
            value: "v".to_owned(),
            domain: domain.to_owned(),
            path: path.to_owned(),
            expires: None,
            http_only: false,
            secure,
            same_site: CookieSameSite::Lax,
        }
    }

    #[test]
    fn cookie_matches_url_respects_domain_path_and_secure() {
        let host_only = sample_cookie("shop.example", "/account", false);
        assert!(cookie_matches_url(
            &host_only,
            &Url::parse("http://shop.example/account").unwrap()
        ));
        assert!(cookie_matches_url(
            &host_only,
            &Url::parse("http://shop.example/account/settings").unwrap()
        ));
        assert!(!cookie_matches_url(
            &host_only,
            &Url::parse("http://shop.example/accounts").unwrap()
        ));
        assert!(!cookie_matches_url(
            &host_only,
            &Url::parse("http://sub.shop.example/account").unwrap()
        ));

        let domain_cookie = sample_cookie(".shop.example", "/", true);
        assert!(cookie_matches_url(
            &domain_cookie,
            &Url::parse("https://sub.shop.example/any").unwrap()
        ));
        assert!(!cookie_matches_url(
            &domain_cookie,
            &Url::parse("http://sub.shop.example/any").unwrap()
        ));
        assert!(!cookie_matches_url(
            &domain_cookie,
            &Url::parse("https://notshop.example/any").unwrap()
        ));
    }

    #[test]
    fn validate_new_cookie_requires_url_xor_domain_and_path() {
        let valid_url = NewCookie {
            name: "a".to_owned(),
            value: "1".to_owned(),
            url: Some("https://shop.example/".to_owned()),
            domain: None,
            path: None,
            expires: None,
            http_only: None,
            secure: None,
            same_site: None,
        };
        assert!(validate_new_cookie(&valid_url, None).is_ok());

        let both = NewCookie {
            domain: Some("shop.example".to_owned()),
            path: Some("/".to_owned()),
            ..valid_url.clone()
        };
        assert!(validate_new_cookie(&both, None).is_err());

        let neither = NewCookie {
            url: None,
            ..valid_url
        };
        assert!(validate_new_cookie(&neither, None).is_err());
    }
}
