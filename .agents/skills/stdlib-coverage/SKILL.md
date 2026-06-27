---
name: stdlib-coverage
description: Understand how much of the Swift standard library the runtime implements, by section and by member. Use when the user asks about stdlib coverage, what's implemented/missing/verified, which APIs to build next, or when planning stdlib work.
---

# Stdlib coverage

Measures the runtime's Swift stdlib implementation against the full reference
surface (`docs/swift-runtime/stdlib-inventory.md`). Each member is classified:

- **missing** — not in the `tswift-std` registry.
- **implemented** — in the registry (declared coverage).
- **verified** — in the registry *and* exercised by a passing CLI golden fixture.

## Workflow — top-down, two steps

Always go overall → section. Don't read the inventory by hand.

1. **Refresh the registry snapshot** (cannot drift — reads the live registry):
   ```sh
   cargo test -p tswift-std dump_registered_keys
   ```

2. **Check overall coverage first** — list every targeted section with
   impl/verified/total counts and the overall roll-up:
   ```sh
   python3 tools/stdlib-inventory/coverage.py
   ```
   Use this to spot the lowest-coverage sections. Add `--all` to include
   sections with no coverage yet.

3. **Then drill into one section** — member-by-member, grouped into
   verified / implemented / missing:
   ```sh
   python3 tools/stdlib-inventory/coverage.py Array
   python3 tools/stdlib-inventory/coverage.py "free functions"
   ```
   The `missing` group is the concrete to-do list for that section.

## Notes

- Section names match inventory headings, case-insensitive; an unknown name
  prints close matches.
- After registering a new stdlib intrinsic, re-run step 1 so the report is current.
- Tooling details: `tools/stdlib-inventory/README.md`. Plan context:
  `docs/plan/stdlib-support.md` §2/§4.2.
