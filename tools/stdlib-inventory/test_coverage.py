#!/usr/bin/env python3
"""Unit tests for the stdlib coverage classifier.

Run: python3 tools/stdlib-inventory/test_coverage.py
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "coverage_tool", Path(__file__).resolve().parent / "coverage.py"
)
coverage = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(coverage)


class CoreMemberTests(unittest.TestCase):
    def test_operators_are_core(self):
        for op in ("+", "-", "*", "/", "==", "!=", "<", "<=", "&<<", "|"):
            self.assertTrue(coverage.is_core_member(op), op)

    def test_subscript_and_init_are_core(self):
        self.assertTrue(coverage.is_core_member("subscript"))
        self.assertTrue(coverage.is_core_member("init"))

    def test_named_methods_are_not_core(self):
        for name in ("append", "map", "count", "isEmpty", "first"):
            self.assertFalse(coverage.is_core_member(name), name)

    def test_range_and_coalescing_operators_are_core(self):
        # These need `?`/`.` in the operator class to classify correctly.
        for op in ("??", "...", "..<"):
            self.assertTrue(coverage.is_core_member(op), op)


class OutOfScopeMemberTests(unittest.TestCase):
    def test_reflection_hooks_are_out_of_scope(self):
        for name in ("customMirror", "customPlaygroundQuickLook"):
            self.assertTrue(coverage.is_out_of_scope(name), name)

    def test_unsafe_and_pointer_members_are_out_of_scope(self):
        for name in (
            "withUnsafeBufferPointer", "withUnsafeBytes", "withUnsafePointer",
            "withContiguousStorageIfAvailable", "span", "mutableSpan", "utf8Span",
            "bitPattern", "unsafeBitCast", "unsafeDowncast", "withCString",
            "withUTF8", "withMutableCharacters", "getVaList", "withVaList",
            "withExtendedLifetime", "extendLifetime",
        ):
            self.assertTrue(coverage.is_out_of_scope(name), name)

    def test_in_scope_members_are_not_out_of_scope(self):
        # Index model, views, bit-pattern accessors, and ordinary methods stay
        # in scope (visible as `missing`), never out-of-scope.
        for name in (
            "map", "append", "startIndex", "endIndex", "index", "distance",
            "unicodeScalars", "utf8", "utf16", "exponentBitPattern",
            "significandBitPattern", "pi", "description",
        ):
            self.assertFalse(coverage.is_out_of_scope(name), name)

    def test_out_of_scope_members_are_not_core(self):
        # core and out-of-scope are disjoint classifications.
        for name in ("customMirror", "withUnsafeBytes", "span"):
            self.assertFalse(coverage.is_core_member(name), name)


class MemberKeyTests(unittest.TestCase):
    def test_operator_free_functions_parse_as_their_token(self):
        # `func ??` must key as `??`, not bleed into the following `<T>` -> `<`.
        cases = {
            "- `public func ?? <T>(optional: T?, default: T) -> T`": "??",
            "- `public static func ... (minimum: Self) -> Range`": "...",
            "- `prefix public static func ..< (maximum: Self) -> Range`": "..<",
            "- `public func == (a: Self, b: Self) -> Bool`": "==",
            "- `public func map<T>(_ transform: (Element) -> T) -> [T]`": "map",
        }
        for line, expected in cases.items():
            self.assertEqual(coverage.member_key(line), expected, line)


class ClassifyTests(unittest.TestCase):
    def _report(self):
        # Array conforms to Sequence, so a `Sequence.map` entry covers Array.map.
        # `withUnsafeBytes` is out-of-scope; everything else is core/registry.
        types_inv = {
            "Array": {
                "+", "subscript", "init", "append", "map", "reduce",
                "withUnsafeBytes",
            }
        }
        reg = ({"print"}, {"Array": {"append"}}, {"map"})  # free, by_type, seq
        ex = (set(), {"Array": set()}, {"map"})
        return coverage.compute_report(
            free_inv=set(),
            types_inv=types_inv,
            reg=reg,
            ex=ex,
            report_types=["Array"],
        )

    def test_out_of_scope_member_gets_its_own_bucket(self):
        arr = self._report()["types"]["Array"]
        # withUnsafeBytes is neither core, missing, impl, nor verif.
        self.assertEqual(arr["oos"], 1)
        self.assertEqual(arr["missing"], 1)  # only reduce

    def test_buckets_partition_total_with_oos(self):
        arr = self._report()["types"]["Array"]
        self.assertEqual(
            arr["core"] + arr["oos"] + arr["missing"] + arr["impl"] + arr["verif"],
            arr["total"],
        )
        self.assertEqual(arr["total"], 7)

    def test_denominator_excludes_out_of_scope(self):
        overall = self._report()["overall"]
        # in-scope total = total(7) - oos(1) = 6.
        # covered = (core 3 + impl 1 + verif 1) / 6
        self.assertAlmostEqual(overall["pct_covered"], 100 * 5 / 6, places=3)
        self.assertAlmostEqual(overall["pct_verified"], 100 * 1 / 6, places=3)

    def test_core_members_not_counted_missing(self):
        arr = self._report()["types"]["Array"]
        # +, subscript, init are core-eval, not registry "missing".
        self.assertEqual(arr["core"], 3)
        # reduce is the only genuinely missing member.
        self.assertEqual(arr["missing"], 1)

    def test_registered_unexercised_is_implemented(self):
        arr = self._report()["types"]["Array"]
        # append is registered but not exercised -> implemented (not verified).
        self.assertEqual(arr["impl"], 1)

    def test_registered_and_exercised_is_verified(self):
        arr = self._report()["types"]["Array"]
        # map flows through the exercised Sequence entry -> verified.
        self.assertEqual(arr["verif"], 1)

    def test_free_function_operators_are_core_not_missing(self):
        # `==`/`??` are operator free functions: core-eval, never registry-missing.
        report = coverage.compute_report(
            free_inv={"==", "??", "print", "abs"},
            types_inv={},
            reg=({"print"}, {}, set()),
            ex=({"print"}, {}, set()),
            report_types=[],
        )
        free = report["free"]
        self.assertEqual(free["core"], 2)  # == and ??
        self.assertEqual(free["verif"], 1)  # print
        self.assertEqual(free["missing"], 1)  # abs
        self.assertEqual(free["core"] + free["missing"] + free["impl"] + free["verif"], free["total"])


if __name__ == "__main__":
    unittest.main()
