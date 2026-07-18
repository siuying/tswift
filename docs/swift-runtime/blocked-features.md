# Blocked / Partial Features

Features that cannot be completed without a larger frontend change or a design
decision needing human input. Each entry records what works, what is missing,
and why it is blocked.

The shared builtin-enum registration now accepts explicit integer raw values.
EventKit registers all 16 in-scope NS_ENUM types with their SDK-defined values,
including 1-based `EKWeekday`, non-ordinal `EKReminderPriority`, and the
`EKAuthorizationStatus.authorized`/`fullAccess` raw-value alias. The
`eventkit_enum_raw_values` golden covers hits, misses, `.rawValue`, and alias
canonicalization; all EventKit enum `init` keys are fixture-verified.
