# Architecture Rubric

Evaluates structural quality of the implementation.

## Separation of Concerns (weight: 2x)
- 1: Everything in one file/function, no boundaries
- 2: Some separation but responsibilities leak across boundaries
- 3: Clear boundaries for main components, minor leakage
- 4: Clean separation with well-defined interfaces between units
- 5: Each unit has one purpose, communicable interfaces, independently testable

## Consistency with Existing Patterns (weight: 3x)
- 1: Completely ignores existing codebase patterns and conventions
- 2: Partially follows but introduces contradictory patterns
- 3: Follows main patterns with minor deviations
- 4: Consistent with existing conventions throughout
- 5: Strengthens existing patterns, improves clarity where touched

## Dependency Management (weight: 1x)
- 1: Circular dependencies, tight coupling everywhere
- 2: Some unnecessary coupling between unrelated modules
- 3: Reasonable dependency structure, no circular deps
- 4: Clean dependency graph, minimal coupling, clear ownership
- 5: Dependency injection where appropriate, no implicit coupling, easily mockable

## Simplicity (weight: 2x)
- 1: Over-engineered with unnecessary abstractions and indirection
- 2: More complex than needed in several places
- 3: Appropriate complexity for most components
- 4: Simple and direct throughout, no premature abstractions
- 5: Minimal complexity, three similar lines > one premature abstraction, YAGNI applied

## Pass Threshold
- Weighted average >= 3.5
- No individual criterion below 3
- Consistency with Existing Patterns below 3 is an automatic FAIL (breaks the codebase)
