"""The workspace's root folder, for tests that read its files."""

from pathlib import Path

WORKSPACE = Path(__file__).resolve().parents[4]
"""The workspace's root folder.

A test names a workspace file from here, never by counting its own parent
folders: mutmut runs a copy of each package's tests one folder deeper, in
`<package>/mutants/` (ADR 0072).
"""
