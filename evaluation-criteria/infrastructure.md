# Infrastructure Rubric

For Terraform, Kubernetes, Helm, CI/CD, and deployment code. Higher bar.

## Security Posture (weight: 3x)
- 1: Wildcard permissions, public access, no encryption
- 2: Basic security but overly permissive (e.g., broad IAM roles)
- 3: Reasonable security, some tightening needed
- 4: Least privilege, proper network segmentation, encrypted
- 5: Zero trust approach, defense in depth, audit-logged, compliance-ready

## Blast Radius (weight: 3x)
- 1: Single change can take down the entire system
- 2: Large blast radius, multiple unrelated services affected
- 3: Moderate blast radius, contained to related services
- 4: Small blast radius, proper isolation between components
- 5: Changes are isolated, rollback-safe, blue-green ready, canary-deployable

## Idempotency (weight: 2x)
- 1: Running twice causes failures, data corruption, or duplicate resources
- 2: Mostly idempotent but some operations create duplicates
- 3: Idempotent for main operations
- 4: Fully idempotent with proper state management (terraform state, k8s reconciliation)
- 5: Idempotent + convergent (self-healing), drift detection

## Observability (weight: 1x)
- 1: No logging, metrics, or alerting configured
- 2: Basic logging only
- 3: Logging + some metrics
- 4: Comprehensive logging, metrics, and alerting on error conditions
- 5: Full observability stack with distributed tracing, dashboards, SLO-based alerting

## Pass Threshold
- Weighted average >= 4.0 (higher bar for infrastructure)
- No individual criterion below 3
- Security Posture below 3 is an automatic FAIL
- Blast Radius below 3 is an automatic FAIL
