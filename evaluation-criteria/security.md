# Security Rubric

Higher pass threshold than code quality. Security failures in production are costly.

## Input Validation (weight: 3x)
- 1: No validation, direct user input in queries/commands/templates
- 2: Basic type checking only, missing boundary validation
- 3: Validation present but incomplete (some paths unvalidated)
- 4: Comprehensive validation at all system entry points
- 5: Defense in depth — validated at boundaries AND internal layers, allowlists over denylists

## Authentication & Authorization (weight: 3x)
- 1: Missing or trivially bypassable
- 2: Present but has known weakness patterns (e.g., JWT without expiry)
- 3: Correctly implemented for main paths
- 4: Comprehensive including edge cases (token expiry, session management, RBAC)
- 5: Follows OWASP best practices, no privilege escalation paths, principle of least privilege

## Secrets Management (weight: 2x)
- 1: Hardcoded secrets in source code
- 2: Secrets in config files that could be committed to git
- 3: Environment variables but not all paths covered consistently
- 4: Proper secrets management (vault, KMS, or equivalent) with rotation capability
- 5: Rotation-ready, least-privilege access, audit-logged, encrypted at rest

## Data Protection (weight: 2x)
- 1: PII/sensitive data exposed in logs, errors, or API responses
- 2: Some protection but inconsistent across the codebase
- 3: Protected in main paths but gaps in error/debug paths
- 4: Comprehensive protection including logs, error messages, and stack traces
- 5: Encryption at rest and in transit, proper data classification, GDPR/CCPA ready

## Pass Threshold
- Weighted average >= 4.0 (higher bar for security)
- No individual criterion below 3
- If Input Validation or Auth scores 1-2, the review is an automatic FAIL
