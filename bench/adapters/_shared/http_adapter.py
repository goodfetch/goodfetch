"""Shared JSON-line bench adapter runner for Python competitors (ADR 0025)."""

from __future__ import annotations

import argparse
import http.client
import json
import re
import sys
from pathlib import Path
from typing import cast
from urllib.parse import urljoin, urlparse

_PRODUCT_ID_RE = re.compile(r'<article class="product" data-id="([^"]+)"')
_PRODUCT_NAME_RE = re.compile(r'<h1 class="name">([^<]+)</h1>')
_PRODUCT_PRICE_RE = re.compile(r'data-price="([^"]+)"')
_LISTING_ITEM_RE = re.compile(
    r'<li class="product" data-id="([^"]+)"[^>]*>([\s\S]*?)</li>'
)
_LINK_TEXT_RE = re.compile(r"<a[^>]*>([^<]+)</a>")
_PRICE_SPAN_RE = re.compile(r'<span class="price"[^>]*>[^0-9]*([0-9]+\.[0-9]{2})')
_NEXT_REL_FIRST_RE = re.compile(r'<a[^>]*rel="next"[^>]*href="([^"]+)"')
_NEXT_HREF_FIRST_RE = re.compile(r'<a[^>]*href="([^"]+)"[^>]*rel="next"')


def _as_str_dict(value: object) -> dict[str, object] | None:
    if not isinstance(value, dict):
        return None
    mapping = cast("dict[object, object]", value)
    return {str(k): v for k, v in mapping.items()}


def _as_str_list(value: object, fallback: list[str]) -> list[str]:
    if not isinstance(value, list):
        return fallback
    items = cast("list[object]", value)
    return [str(item) for item in items]


def _reply(msg_id: object, result: object) -> None:
    sys.stdout.write(json.dumps({"version": 1, "id": msg_id, "result": result}) + "\n")
    sys.stdout.flush()


def _reply_error(msg_id: object, error: str) -> None:
    sys.stdout.write(json.dumps({"version": 1, "id": msg_id, "error": error}) + "\n")
    sys.stdout.flush()


def _fetch_page(address: str, url: str) -> str:
    parsed = urlparse(url)
    host, _, port_str = address.rpartition(":")
    conn = http.client.HTTPConnection(host, int(port_str), timeout=15)
    try:
        path = parsed.path or "/"
        if parsed.query:
            path = f"{path}?{parsed.query}"
        conn.request(
            "GET", path, headers={"Host": parsed.netloc, "Connection": "close"}
        )
        resp = conn.getresponse()
        return resp.read().decode("utf-8", errors="replace")
    finally:
        conn.close()


def _extract_product(html: str) -> dict[str, str]:
    id_m = _PRODUCT_ID_RE.search(html)
    name_m = _PRODUCT_NAME_RE.search(html)
    price_m = _PRODUCT_PRICE_RE.search(html)
    return {
        "id": id_m.group(1) if id_m else "",
        "name": name_m.group(1) if name_m else "",
        "price": price_m.group(1) if price_m else "",
    }


def _crawl_listing(address: str, start_url: str) -> list[dict[str, str]]:
    seen_urls: set[str] = set()
    products: dict[str, dict[str, str]] = {}
    current: str | None = start_url
    while current is not None and current not in seen_urls:
        seen_urls.add(current)
        html = _fetch_page(address, current)
        for match in _LISTING_ITEM_RE.finditer(html):
            prod_id = match.group(1)
            inner = match.group(2)
            name_m = _LINK_TEXT_RE.search(inner)
            price_m = _PRICE_SPAN_RE.search(inner)
            if prod_id and prod_id not in products:
                products[prod_id] = {
                    "id": prod_id,
                    "name": name_m.group(1).strip() if name_m else "",
                    "price": price_m.group(1) if price_m else "",
                }
        next_m = _NEXT_REL_FIRST_RE.search(html) or _NEXT_HREF_FIRST_RE.search(html)
        current = urljoin(current, next_m.group(1)) if next_m else None
    return list(products.values())


def run_adapter(config_path: Path) -> None:
    """Serve the bench JSON-line adapter protocol for `config_path`."""
    raw_cfg = _as_str_dict(json.loads(config_path.read_text(encoding="utf-8")))
    if raw_cfg is None:
        msg = f"expected JSON object in {config_path}"
        raise TypeError(msg)
    capabilities = _as_str_list(raw_cfg.get("capabilities"), ["crawl", "extract"])
    raw_conc = raw_cfg.get("max_concurrency", 16)
    max_concurrency = raw_conc if isinstance(raw_conc, int) else 16
    tool = str(raw_cfg.get("tool", "python-competitor"))
    address = ""

    for raw_line in sys.stdin:
        line = raw_line.strip()
        if not line:
            continue
        parsed = _as_str_dict(json.loads(line))
        if parsed is None:
            continue
        msg_id = parsed.get("id")
        method = parsed.get("method")
        params = _as_str_dict(parsed.get("params")) or {}

        if method == "hello":
            _reply(
                msg_id,
                {"capabilities": capabilities, "max_concurrency": max_concurrency},
            )
        elif method == "prepare":
            ground = _as_str_dict(params.get("proving_ground"))
            if ground is not None and "address" in ground:
                address = str(ground["address"])
            _reply(msg_id, {})
        elif method == "run":
            task = _as_str_dict(params.get("task"))
            if task is None or "kind" not in task or "start_url" not in task:
                _reply_error(msg_id, "invalid task payload")
                continue
            kind = str(task["kind"])
            start_url = str(task["start_url"])
            if kind not in capabilities:
                _reply(
                    msg_id,
                    {
                        "status": "unsupported",
                        "reason": f"{tool} does not support {kind}",
                    },
                )
                continue
            try:
                if kind == "extract":
                    html = _fetch_page(address, start_url)
                    _reply(
                        msg_id,
                        {"status": "success", "records": [_extract_product(html)]},
                    )
                elif kind == "crawl":
                    records = _crawl_listing(address, start_url)
                    _reply(msg_id, {"status": "success", "records": records})
                else:
                    _reply(
                        msg_id,
                        {"status": "unsupported", "reason": f"unhandled kind {kind}"},
                    )
            except Exception as exc:
                _reply_error(msg_id, str(exc))
        elif method == "close":
            _reply(msg_id, {})
            return
        else:
            _reply_error(msg_id, f"unknown method {method}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    ns = parser.parse_args()
    run_adapter(Path(str(ns.config)))
