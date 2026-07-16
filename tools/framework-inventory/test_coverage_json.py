#!/usr/bin/env python3
"""Tests for coverage.py's `--emit-json` mode and generate_website_json.py.

No test framework dependency (offline builds; stdlib `unittest` only).

Run:
  python3 tools/framework-inventory/test_coverage_json.py
  python3 -m unittest tools/framework-inventory/test_coverage_json.py
"""
from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOL_DIR = Path(__file__).resolve().parent

sys.path.insert(0, str(TOOL_DIR))

import coverage as coverage_mod  # noqa: E402
import generate_website_json  # noqa: E402

VALID_STATUSES = {"implemented", "partial", "missing", "out_of_scope"}


def assert_schema(payload: dict) -> list[str]:
    """Return a list of schema violations (empty = valid)."""
    errors = []
    if not isinstance(payload.get("framework"), str) or not payload["framework"]:
        errors.append("framework must be a non-empty string")
    sections = payload.get("sections")
    if not isinstance(sections, list):
        errors.append("sections must be a list")
        sections = []
    for section in sections:
        name = section.get("name")
        if not isinstance(name, str) or not name:
            errors.append(f"section missing a name: {section!r}")
        members = section.get("members")
        if not isinstance(members, list):
            errors.append(f"section {name!r} members must be a list")
            members = []
        for member in members:
            if not isinstance(member.get("name"), str) or not member["name"]:
                errors.append(f"member missing a name in section {name!r}: {member!r}")
            if not isinstance(member.get("kind"), str) or not member["kind"]:
                errors.append(f"member {member!r} in {name!r} missing a kind")
            status = member.get("status")
            if status not in VALID_STATUSES:
                errors.append(f"member {member!r} in {name!r} has invalid status {status!r}")
            if "notes" in member and not isinstance(member["notes"], str):
                errors.append(f"member {member!r} in {name!r} has non-string notes")
        counts = section.get("counts")
        if not isinstance(counts, dict):
            errors.append(f"section {name!r} missing counts")
            continue
        for key in ("implemented", "partial", "missing", "out_of_scope", "total"):
            if key not in counts:
                errors.append(f"section {name!r} counts missing {key!r}")
        if counts.get("total") != sum(
            counts.get(k, 0) for k in ("implemented", "partial", "missing", "out_of_scope")
        ):
            errors.append(f"section {name!r} counts.total does not match its parts")
        actual_by_status: dict[str, int] = {}
        for member in members:
            actual_by_status[member["status"]] = actual_by_status.get(member["status"], 0) + 1
        for status in VALID_STATUSES:
            if counts.get(status, 0) != actual_by_status.get(status, 0):
                errors.append(
                    f"section {name!r} counts[{status!r}]={counts.get(status)} "
                    f"but {actual_by_status.get(status, 0)} members have that status"
                )
    totals = payload.get("totals")
    if not isinstance(totals, dict):
        errors.append("totals missing")
    else:
        summed = {"implemented": 0, "partial": 0, "missing": 0, "out_of_scope": 0}
        for section in sections:
            for key in summed:
                summed[key] += section.get("counts", {}).get(key, 0)
        for key, expected in summed.items():
            if totals.get(key) != expected:
                errors.append(f"totals[{key!r}]={totals.get(key)} but sections sum to {expected}")
        if totals.get("total") != sum(summed.values()):
            errors.append("totals.total does not match the sum of its parts")
    return errors


class EmitJsonSchemaTests(unittest.TestCase):
    """Schema + cross-check tests against the real checked-in manifests."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.frameworks = sorted(coverage_mod.load_manifest())

    def test_every_framework_emits_schema_valid_json(self) -> None:
        for name in self.frameworks:
            with self.subTest(framework=name):
                cov = coverage_mod.Coverage(name)
                payload = cov.to_dict(include_all=False)
                errors = assert_schema(payload)
                self.assertEqual(errors, [], f"{name}: {errors}")

    def test_totals_match_text_report_counts(self) -> None:
        """Text-report `total` is the in-scope denominator only (implemented +
        partial + missing after JSON rename). JSON `totals.total` is wider:
        it also includes per-member `out_of_scope` from scope.toml
        `[out_of_scope_members]` (and type-level OOS when those types are
        still listed). Compare the in-scope slice, not JSON grand total,
        against the text total. See JSON_STATUS for verified→implemented /
        implemented→partial renaming."""
        for name in self.frameworks:
            with self.subTest(framework=name):
                cov = coverage_mod.Coverage(name)
                payload = cov.to_dict(include_all=False)
                totals = payload["totals"]

                text_impl = text_total = 0
                include_free = cov.include_free_functions()
                for section in cov.targeted_sections(False):
                    impl, _verif, total, _out = cov.counts(section)
                    text_impl += impl
                    text_total += total
                if include_free:
                    impl, _verif, total, _out = cov.counts(coverage_mod.FREE_SECTION)
                    text_impl += impl
                    text_total += total

                # Text total = in-scope impl+missing; JSON total also includes
                # per-member out_of_scope on in-scope types.
                self.assertEqual(totals["implemented"] + totals["partial"], text_impl)
                self.assertEqual(
                    totals["implemented"] + totals["partial"] + totals["missing"],
                    text_total,
                )
                # JSON grand total may exceed text total by the OOS count.
                self.assertGreaterEqual(
                    totals["total"],
                    text_total,
                )
                self.assertEqual(
                    totals["total"],
                    totals["implemented"]
                    + totals["partial"]
                    + totals["missing"]
                    + totals["out_of_scope"],
                )

    def test_out_of_scope_members_only_named_keys(self) -> None:
        """`[out_of_scope_members]` marks only explicitly listed Type.member
        keys. Adjacent members of the same in-scope type stay counted
        (implemented/partial/missing). Frameworks without the table are
        unchanged."""
        # (a)+(b) Charts has `[out_of_scope_members]` with View.chart3D*/chartZ*.
        charts = coverage_mod.Coverage("charts")
        self.assertTrue(
            charts.scope.get("out_of_scope_members"),
            "charts must declare [out_of_scope_members] for this test",
        )
        oos_keys: set[str] = set()
        for keys in charts.scope["out_of_scope_members"].values():
            oos_keys.update(keys)

        # Named 3D members are out_of_scope with the chart3d bucket note.
        self.assertIn("View.chart3DPose", oos_keys)
        self.assertEqual(charts.member_state("View", "chart3DPose"), "out_of_scope")
        view_section = charts.section_json("View")
        pose = next(m for m in view_section["members"] if m["name"] == "chart3DPose")
        self.assertEqual(pose["status"], "out_of_scope")
        self.assertEqual(pose.get("notes"), "out of scope: chart3d")

        # (b) Adjacent in-scope View members remain implemented/partial/missing.
        for name in ("chartXAxis", "chartLegend", "chartYScale"):
            state = charts.member_state("View", name)
            self.assertIn(
                state,
                {"verified", "implemented", "missing"},
                f"View.{name} must stay in-scope, got {state}",
            )
            entry = next(m for m in view_section["members"] if m["name"] == name)
            self.assertNotEqual(entry["status"], "out_of_scope")
            self.assertNotIn("notes", entry)

        # ChartContent has no OOS members now — a11y keys are in-scope.
        self.assertNotIn("ChartContent", charts._excluded_members)
        self.assertNotEqual(
            charts.member_state("ChartContent", "accessibilityHidden"),
            "out_of_scope",
        )
        self.assertNotEqual(
            charts.member_state("ChartContent", "opacity"),
            "out_of_scope",
        )

        # Only members listed in the table are OOS for View (no blanket type drop).
        listed_view = {
            key.split(".", 1)[1]
            for key in oos_keys
            if key.startswith("View.")
        }
        for member in view_section["members"]:
            if member["status"] == "out_of_scope":
                self.assertIn(
                    member["name"],
                    listed_view,
                    f"unexpected OOS View.{member['name']} not in scope table",
                )

        # (c) Foundation has no [out_of_scope_members] — entirely unaffected.
        foundation = coverage_mod.Coverage("foundation")
        self.assertFalse(
            foundation.scope.get("out_of_scope_members"),
            "foundation must not declare [out_of_scope_members]",
        )
        self.assertEqual(foundation._excluded_members, {})
        foundation_payload = foundation.to_dict(include_all=False)
        # No per-member OOS from the members table (type-level OOS may still
        # exist via [out_of_scope] types that appear in targeted sections only
        # when scoped — foundation excludes whole types from tiers).
        for section in foundation_payload["sections"]:
            for member in section["members"]:
                if member["status"] == "out_of_scope":
                    # Type-level exclusion only — notes come from type buckets.
                    self.assertTrue(
                        member.get("notes", "").startswith("out of scope:"),
                        member,
                    )
        # Spot-check a known in-scope type is classified without member OOS.
        if "Data" in foundation.types_inv and "Data" in foundation.targeted_sections(False):
            data = foundation.section_json("Data")
            self.assertTrue(data["members"])
            self.assertEqual(data["counts"]["out_of_scope"], 0)
            for member in data["members"]:
                self.assertIn(member["status"], ("implemented", "partial", "missing"))

    def test_cli_emit_json_matches_in_process_payload(self) -> None:
        name = "foundation"
        cov = coverage_mod.Coverage(name)
        expected = cov.to_dict(include_all=False)
        result = subprocess.run(
            [sys.executable, str(TOOL_DIR / "coverage.py"), "--framework", name, "--emit-json"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(json.loads(result.stdout), expected)

    def test_emit_json_all_flag_widens_stdlib_sections(self) -> None:
        cov = coverage_mod.Coverage("stdlib")
        narrow = cov.to_dict(include_all=False)
        wide = cov.to_dict(include_all=True)
        self.assertLessEqual(len(narrow["sections"]), len(wide["sections"]))


class GenerateWebsiteJsonTests(unittest.TestCase):
    def test_checked_in_files_are_not_stale(self) -> None:
        """Fails loudly if a manifest changed but nobody re-ran
        scripts/generate-coverage-json.sh. Mirrors the `validate web` gate."""
        rendered = generate_website_json.build_all()
        for name, text in rendered.items():
            path = generate_website_json.OUT_DIR / f"{name}.json"
            self.assertTrue(path.exists(), f"missing checked-in {path}")
            self.assertEqual(
                path.read_text(),
                text,
                f"{path} is stale; run scripts/generate-coverage-json.sh",
            )

    def test_index_totals_match_per_framework_files(self) -> None:
        rendered = generate_website_json.build_all()
        index = json.loads(rendered["index"])
        for entry in index["frameworks"]:
            per_framework = json.loads(rendered[entry["framework"]])
            self.assertEqual(entry["totals"], per_framework["totals"])
            self.assertEqual(entry["display_name"], per_framework["framework"])


if __name__ == "__main__":
    unittest.main()
