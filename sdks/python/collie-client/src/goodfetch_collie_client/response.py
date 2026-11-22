"""The response a page's request got, such as a navigation's document (ADR 0085).

What `page.goto`, `reload`, `go_back`, `go_forward` and `request` answer.
Its head is in hand; its body is read on demand, through `response.body`,
from the file the engine keeps it in, which a Client on the same machine
reads (ADR 0118). `AsyncResponse` is the async API's, `Response` its sync
twin's. Python keeps Playwright's spellings, its head's properties
included; Rust's `Response` and TypeScript's are its twins.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

import anyio.to_thread

from goodfetch_collie_client import _decoding
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated import models
from goodfetch_collie_client.generated.errors import (
    BodyUnavailableError,
    ResponseNotJSONError,
)
from goodfetch_collie_client.markers import warn_experimental

if TYPE_CHECKING:
    from goodfetch_collie_client.handle import AsyncHandleHost, HandleHost
    from goodfetch_collie_client.json_values import JSONValue

__all__ = ["AsyncResponse", "Response"]

_BODY = "response.body"
_TIMEOUT = "timeout.wait"


class _ResponseHead:
    """A response's head, which both APIs' responses read alike."""

    def __init__(self, response: models.PageResponse) -> None:
        """Takes the response as the engine answered it."""
        self._response = response

    @property
    def url(self) -> str:
        """Its final URL."""
        return self._response.url

    @property
    def status(self) -> int:
        """Its HTTP status."""
        return self._response.status

    @property
    def status_text(self) -> str:
        """Its status text."""
        return self._response.status_text

    @property
    def ok(self) -> bool:
        """Whether its status is a success, from 200 to 299.

        It says nothing of the page: a login page answered with a 200 is `ok`.
        """
        return 200 <= self._response.status <= 299

    @property
    def headers(self) -> dict[str, str]:
        """Its headers, their names lower-cased, as the browser received them."""
        return self._response.headers

    @property
    def encoding(self) -> str | None:
        """For a navigation's document, the encoding the browser decoded it with.

        As `document.characterSet` names it, such as `Shift_JIS` (ADR 0119);
        none for another response, and for a navigation that answered at
        `commit`, before its document was decoded.
        """
        return self._response.encoding

    @property
    def request_id(self) -> str:
        """The engine's ID for its request's hop."""
        return self._response.request_id

    @property
    def from_service_worker(self) -> bool:
        """Whether a site's service worker supplied it (ADR 0121).

        From the worker's own cache or through its own fetch.
        """
        return self._response.from_service_worker

    def __repr__(self) -> str:
        """Its status and URL, as Playwright's `<Response url=… request=…>` shows."""
        return f"<{type(self).__name__} url={self.url!r} status={self.status}>"

    def _params(self, page_id: models.PageId) -> models.ResponseBodyParams:
        return models.ResponseBodyParams(
            page_id=page_id, request_id=self._response.request_id
        )

    def _text(self, body: bytes) -> str:
        return _decoding.text(
            body, self._response.encoding, self._response.headers.get("content-type")
        )

    def _json(self, body: bytes) -> JSONValue:
        try:
            return _decoding.json_of(body)
        except ValueError as error:
            url = self._response.url
            message = f"the body of {url} is not the JSON asked for: {error}"
            raise ResponseNotJSONError(message, url=url) from error


def _read(handle: models.BodyHandle) -> bytes:
    """The bytes of the body `handle` names, from the file the engine keeps it in.

    Raises:
        BodyUnavailableError: The file cannot be read, or holds other than
            the body's size.
    """
    try:
        body = Path(handle.path).read_bytes()
        if len(body) != handle.size:
            message = (
                f"the file holds {len(body)} bytes where the body has {handle.size}"
            )
            raise ValueError(message)
    except (OSError, ValueError) as error:
        message = f"could not read the response's body from `{handle.path}`: {error}"
        raise BodyUnavailableError(message) from error
    return body


class AsyncResponse(_ResponseHead):
    """The response a page's request got, in the async API.

    A navigation's document, or what `page.request` answered. A 404 or a
    500 is a response, a value.
    """

    def __init__(
        self, host: AsyncHandleHost[models.PageId], response: models.PageResponse
    ) -> None:
        """Takes its page's host and the response; a program never constructs one."""
        super().__init__(response)
        self._host = host

    async def body(self) -> bytes:
        """Its body's bytes, once its request finished.

        Reads the body of the response a page's request got, once the request
        finished, as a page body's handle: kept already for a response
        `page.wait_for_request_finished` answered, and otherwise read from the
        browser, on the session of the frame that loaded it. A body the browser no
        longer holds, or a redirect's, which has none, raises `body_unavailable`,
        whose fix names `page.wait_for_request_finished`; one that broke off raises
        `response_truncated`, never returned as whole. Past its `timeout`,
        `timeout.wait` when the call gives none, a request that has not ended fails it
        with `deadline_exceeded`; a page that ends under it raises what ended it, as a
        call on the page does.

        Experimental: outside semver until it is promoted (ADR 0030).

        It answers its body's bytes, as the server sent them once their
        content coding is undone, read through that handle.

        Raises:
            ResponseTruncatedError: The body broke off, never returned as whole.
            BodyUnavailableError: The browser no longer holds the body, as
                after the page navigated away, or it is a redirect's, which
                has none: the engine keeps the body of a response obtained
                through `page.wait_for_request_finished` or `page.request`,
                which its fix names. Also when the file the engine keeps it
                in cannot be read.
        """
        warn_experimental(_BODY)
        answer = await self._host.call(
            _BODY, self._params(self._host.id), timeout_setting=_TIMEOUT
        )
        handle = result_of(models.BodyHandle, answer)
        return await anyio.to_thread.run_sync(_read, handle)

    async def text(self) -> str:
        """Its body as text, decoded with the response's own encoding (ADR 0119).

        A navigation's document decodes with the one the browser decoded it
        with, `encoding`; any other response with its BOM, then its
        `Content-Type` charset, then for HTML a `<meta>` as Chrome's prescan
        reads it (its first 1,024 bytes always, then on while only tags
        allowed in the head appear) or an XML declaration at its very
        start, for an XML type that declaration, and UTF-8 last. A legacy
        charset such as `Shift_JIS` is honoured; a BOM is left out, and a
        byte that does not decode becomes U+FFFD. Unlike Playwright's
        `text()`, which decodes every body as UTF-8, it honours the
        response's encoding, so a page that declares another one reads as
        `page.content()` does, not as mojibake.

        Raises:
            ResponseTruncatedError: The body broke off, never decoded as whole.
            BodyUnavailableError: The browser no longer holds the body, as
                `body()` raises it.
        """
        return self._text(await self.body())

    async def json(self) -> JSONValue:
        """Its body parsed as JSON.

        It decodes as UTF-8 whatever its charset, as JSON is, a leading BOM
        ignored.

        Raises:
            ResponseTruncatedError: The body broke off, as `body()` raises it.
            BodyUnavailableError: The browser no longer holds the body, as
                `body()` raises it.
            ResponseNotJSONError: The body is not JSON.
        """
        return self._json(await self.body())


class Response(_ResponseHead):
    """The response a page's request got, as `AsyncResponse` is, in the sync API."""

    def __init__(
        self, host: HandleHost[models.PageId], response: models.PageResponse
    ) -> None:
        """Takes its page's host and the response; a program never constructs one."""
        super().__init__(response)
        self._host = host

    def body(self) -> bytes:
        """Its body's bytes, once its request finished.

        Reads the body of the response a page's request got, once the request
        finished, as a page body's handle: kept already for a response
        `page.wait_for_request_finished` answered, and otherwise read from the
        browser, on the session of the frame that loaded it. A body the browser no
        longer holds, or a redirect's, which has none, raises `body_unavailable`,
        whose fix names `page.wait_for_request_finished`; one that broke off raises
        `response_truncated`, never returned as whole. Past its `timeout`,
        `timeout.wait` when the call gives none, a request that has not ended fails it
        with `deadline_exceeded`; a page that ends under it raises what ended it, as a
        call on the page does.

        Experimental: outside semver until it is promoted (ADR 0030).

        It answers its body's bytes, as `AsyncResponse.body` does.

        Raises:
            ResponseTruncatedError: The body broke off, never returned as whole.
            BodyUnavailableError: The browser no longer holds the body, or
                the file the engine keeps it in cannot be read, as
                `AsyncResponse.body` raises it.
        """
        warn_experimental(_BODY)
        answer = self._host.call(
            _BODY, self._params(self._host.id), timeout_setting=_TIMEOUT
        )
        return _read(result_of(models.BodyHandle, answer))

    def text(self) -> str:
        """Its body as text, decoded with the response's own encoding (ADR 0119).

        It decodes as `AsyncResponse.text` does. A legacy charset such as
        `Shift_JIS` is honoured; a BOM is left out, and a byte that does not
        decode becomes U+FFFD. Unlike
        Playwright's `text()`, which decodes every body as UTF-8, it
        honours the response's encoding.

        Raises:
            ResponseTruncatedError: The body broke off, never decoded as whole.
            BodyUnavailableError: The browser no longer holds the body, as
                `body()` raises it.
        """
        return self._text(self.body())

    def json(self) -> JSONValue:
        """Its body parsed as JSON.

        It decodes as UTF-8 whatever its charset, as JSON is, a leading BOM
        ignored.

        Raises:
            ResponseTruncatedError: The body broke off, as `body()` raises it.
            BodyUnavailableError: The browser no longer holds the body, as
                `body()` raises it.
            ResponseNotJSONError: The body is not JSON.
        """
        return self._json(self.body())
