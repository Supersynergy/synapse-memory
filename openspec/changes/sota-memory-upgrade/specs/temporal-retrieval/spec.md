# Spec — temporal-retrieval

## ADDED Requirements

### Requirement: Natural-language temporal phrase parser
A library MUST translate phrases like `yesterday`, `last week`, `Q3 2025`,
`vor 3 Tagen`, `gestern` into a `(start_ts, end_ts)` Unix-second range.

#### Scenario: English phrase
- **WHEN** `parse_temporal("yesterday", Locale::English)` is called
- **THEN** it returns a 24-hour range covering the previous calendar day in UTC.

#### Scenario: German phrase
- **WHEN** `parse_temporal("gestern", Locale::German)` is called
- **THEN** it normalises to English ("yesterday") and returns a valid day window.

#### Scenario: Quarter shorthand
- **WHEN** `parse_temporal("Q3 2025", _)` is called
- **THEN** it returns the inclusive range `2025-07-01..2025-09-30`.

#### Scenario: Unrecognised phrase
- **WHEN** the input is not a temporal expression
- **THEN** the parser returns `None` and never panics.

### Requirement: Recall period filter
`Store::recall(RecallParams)` MAY accept a `period: Option<TimeRange>` and
SHALL filter candidate memories whose `created_ts` is outside that range
when present. (Wiring follow-up — parser already implemented.)

## Implementation notes
Mined from `stevedonovan/chrono-english` (production usage in
`facebook/sapling/eden/mononoke/.../datetime.rs`). 90 % of parser logic is
reused; the wrapper adapts:
* a small German→English phrase normaliser,
* day-window widening for bare-day phrases,
* `Q[1-4] YYYY` shorthand fallback.
