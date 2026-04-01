# Code Quality Rubric

The evaluator scores each criterion 1-5. Weighted average determines pass/fail.

## Correctness (weight: 3x)
- 1: Core functionality broken or missing
- 2: Works for happy path, fails on basic error cases
- 3: Works correctly for common cases, some edge cases missed
- 4: Handles all expected cases including errors
- 5: Handles edge cases, boundary conditions, and unexpected inputs gracefully

## Readability (weight: 1x)
- 1: Incomprehensible without significant effort
- 2: Requires significant effort to understand intent
- 3: Understandable with moderate effort, some unclear sections
- 4: Clear and well-organized, intent is obvious
- 5: Self-documenting, follows project conventions perfectly, a joy to read

## Test Coverage (weight: 2x)
- 1: No tests
- 2: Minimal happy-path tests only
- 3: Reasonable coverage of main paths and some error paths
- 4: Good coverage including error paths and important edge cases
- 5: Comprehensive including edge cases, integration tests, and property-based tests where appropriate

## Error Handling (weight: 2x)
- 1: No error handling, crashes on unexpected input
- 2: Basic try/catch but errors swallowed or generic messages
- 3: Errors caught and reported but not all paths covered
- 4: Robust error handling with meaningful messages and proper propagation
- 5: Graceful degradation, proper error propagation, recovery paths, no silent failures

## Pass Threshold
- Weighted average >= 3.5
- No individual criterion below 3
- If ANY criterion scores 1, the review is an automatic FAIL regardless of average
