---
project_name: [PROJECT_NAME]
project_type: [PROJECT_TYPE]
domain: [DOMAIN]
complexity: [COMPLEXITY]
created: [DATE]
last_updated: [DATE]
version: 1.0
---

<!--
=============================================================================
SECTION ID -> TEMPLATE HEADER MAPPING
=============================================================================
This mapping connects CSV section IDs (from project-types.csv and
domain-complexity.csv) to the actual headers in this template.

  CSV Section ID            -> Template Header
  ─────────────────────────────────────────────────────────────
  executive_summary         -> ## Executive Summary
  success_criteria          -> ## Success Criteria
  user_journeys             -> ## User Journeys
  user_journeys_visual      -> ## User Journeys (visual/diagram variant)
                               NOTE: user_journeys_visual is a skip-only signal
                               (used in skip_sections to suppress visual journey
                               diagrams). It shares the User Journeys header.
  functional_reqs           -> ## Functional Requirements
  nfr_performance           -> ### Performance (under Non-Functional Requirements)
  nfr_security              -> ### Security (under Non-Functional Requirements)
  nfr_scalability           -> ### Scalability (under Non-Functional Requirements)
  ui_design                 -> ## UI Design
  accessibility             -> ### Accessibility (under Non-Functional Requirements)
  accessibility_web         -> ### Accessibility (web-specific variant)
  seo                       -> ## SEO Requirements
  api_design                -> ## API Design
  multi_tenancy             -> ## Multi-Tenancy Architecture
  integration               -> ## Integration
  mobile_specific           -> ## Mobile-Specific Requirements
  iot_specific              -> ## IoT-Specific Requirements
  data_architecture         -> ## Data Architecture
  infrastructure_design     -> ## Infrastructure Design
  regulatory_compliance     -> ## Regulatory Compliance
  financial_security        -> ## Financial Security
  audit_requirements        -> ## Audit Requirements
  clinical_requirements     -> ## Clinical Requirements
  data_protection           -> ## Data Protection
  accessibility_requirements -> ## Accessibility Requirements
  content_safety            -> ## Content Safety
  data_privacy              -> ## Data Privacy
  payment_security          -> ## Payment Security
  inventory_design          -> ## Inventory Design
  tax_compliance            -> ## Tax Compliance
  security_controls         -> ## Security Controls
=============================================================================
-->

# Product Requirements Document: [PROJECT_NAME]

## Executive Summary

<!-- A concise description of what this product does, who it serves, and why it matters. -->

### What Makes This Special

<!-- What differentiates this from existing solutions? Why build this now? What is the unique insight or advantage? -->

---

## Success Criteria

### User Success
<!-- How will users know this product is working for them? What does their "before and after" look like? -->

- [ ] 

### Business Success
<!-- What business outcomes does this product need to achieve? Revenue, adoption, efficiency? -->

- [ ] 

### Measurable Outcomes
<!-- Specific, time-bound metrics that define success. Each should be testable. -->

| Metric | Target | Timeframe | How Measured |
|--------|--------|-----------|--------------|
|        |        |           |              |

---

## User Journeys

<!-- 
Write user journeys as narratives, not bullet points. Each journey follows this arc:

  Opening Scene -> Rising Action -> Climax -> Resolution

Include the emotional arc: what the user feels at each stage. 
Focus on the "why" behind each action, not just the "what."

Example format:
  Sarah, a project manager at a mid-size agency, opens her laptop Monday morning 
  feeling overwhelmed by the week ahead. She navigates to the dashboard and 
  immediately sees her prioritized task list... [continue the narrative]

Each journey should cover:
  - Who is the user and what is their context?
  - What triggers them to use the product?
  - What do they experience step by step?
  - How do they feel at each stage?
  - What is the successful outcome?
-->

### Journey 1: [Journey Name]

### Journey 2: [Journey Name]

---

## Functional Requirements

<!-- 
Define capabilities as a contract. Use this format:

  FR#: [Actor] can [capability]
  
  Example:
    FR1: Authenticated user can create a new project with name, description, and deadline
    FR2: Admin can invite team members via email with role assignment
    FR3: System can send email notifications when task status changes

IMPORTANT: If it's not here, it won't exist in the final product.
Every feature, behavior, and interaction must be captured as a functional requirement.
Be specific. "User can manage projects" is too vague. 
"User can create, rename, archive, and delete projects" is correct.
-->

### Core Capabilities

### Supporting Capabilities

### Administrative Capabilities

---

## Non-Functional Requirements

<!-- 
Only include categories that matter for THIS product.
Delete any section that is not relevant. Do not pad with generic requirements.

Subsection IDs for CSV mapping:
  nfr_performance  -> ### Performance
  nfr_security     -> ### Security
  nfr_scalability  -> ### Scalability
  accessibility    -> ### Accessibility
-->

### Performance
<!-- SECTION ID: nfr_performance -->
<!-- 
Define measurable performance targets:
  - Response time (p50, p95, p99) for key operations
  - Throughput requirements (requests/sec, concurrent users)
  - Resource constraints (memory, CPU, bandwidth)
  - Cold start / warm start expectations
  - Batch processing time limits
-->

### Security
<!-- SECTION ID: nfr_security -->
<!-- 
Define security requirements:
  - Authentication and authorization model
  - Data encryption requirements (at rest, in transit)
  - Input validation and sanitization rules
  - Secret management approach
  - Vulnerability scanning and patching cadence
  - Security logging and alerting
-->

### Scalability
<!-- SECTION ID: nfr_scalability -->
<!-- 
Define scaling expectations:
  - Expected growth trajectory (users, data volume, request volume)
  - Horizontal vs vertical scaling strategy
  - Database scaling approach (read replicas, sharding, partitioning)
  - Caching strategy
  - Queue and async processing requirements
-->

### Accessibility
<!-- SECTION ID: accessibility / accessibility_web -->
<!-- 
Define accessibility standards:
  - Target WCAG conformance level (A, AA, AAA)
  - Screen reader compatibility requirements
  - Keyboard navigation requirements
  - Color contrast and text sizing
  - Focus management and ARIA usage
-->

### Reliability
<!-- 
Define reliability targets:
  - Uptime SLA (e.g. 99.9%)
  - Recovery Time Objective (RTO)
  - Recovery Point Objective (RPO)
  - Failure mode handling
-->

### Observability
<!-- 
Define observability requirements:
  - Logging standards and retention
  - Metrics and dashboards
  - Distributed tracing
  - Alerting thresholds
-->

---

## UI Design
<!-- CONDITIONAL: Only include if project type requires it (web_app, mobile_app) -->
<!-- SECTION ID: ui_design -->
<!-- 
Define UI/UX requirements:
  - Design system or component library to use
  - Responsive breakpoints and layout approach
  - Theming and branding guidelines
  - Key screen wireframes or mockup references
  - Interaction patterns (drag-and-drop, infinite scroll, etc.)
  - Loading states, empty states, and error states
-->

---

## SEO Requirements
<!-- CONDITIONAL: Only include if project type requires it (web_app) -->
<!-- SECTION ID: seo -->
<!-- 
Define SEO requirements:
  - Server-side rendering or static generation needs
  - Meta tag strategy (title, description, Open Graph, Twitter Cards)
  - Structured data (JSON-LD schemas)
  - Sitemap and robots.txt requirements
  - URL structure and canonical URLs
  - Core Web Vitals targets (LCP, FID, CLS)
-->

---

## API Design
<!-- CONDITIONAL: Only include if project type requires it (api_backend, saas_b2b) -->
<!-- SECTION ID: api_design -->
<!-- 
Define API design requirements:
  - API style (REST, GraphQL, gRPC, WebSocket)
  - Versioning strategy (URL path, header, query param)
  - Authentication mechanism (OAuth 2.0, API keys, JWT)
  - Rate limiting and throttling rules
  - Request/response format and schema conventions
  - Pagination approach (cursor-based, offset-based)
  - Error response format and status code conventions
  - API documentation approach (OpenAPI/Swagger, GraphQL introspection)
  - CORS policy
  - Idempotency requirements
-->

---

## Multi-Tenancy Architecture
<!-- CONDITIONAL: Only include if project type requires it (saas_b2b) -->
<!-- SECTION ID: multi_tenancy -->
<!-- 
Define multi-tenancy requirements:
  - Tenant isolation model (shared database, schema-per-tenant, database-per-tenant)
  - Tenant identification strategy (subdomain, header, URL path)
  - Data segregation and cross-tenant access prevention
  - Per-tenant configuration and feature flags
  - Tenant provisioning and onboarding flow
  - Tenant-specific resource limits and quotas
  - Billing and metering per tenant
  - Tenant data export and deletion (offboarding)
-->

---

## Integration
<!-- CONDITIONAL: Only include if project type requires it (saas_b2b) -->
<!-- SECTION ID: integration -->
<!-- 
Define integration requirements:
  - Third-party services and APIs to integrate with
  - Webhook support (inbound and outbound)
  - SSO and identity provider integration (SAML, OIDC)
  - Data import/export formats and mechanisms
  - Integration authentication and credential management
  - Retry and error handling for external service calls
  - Integration monitoring and health checks
-->

---

## Mobile-Specific Requirements
<!-- CONDITIONAL: Only include if project type requires it (mobile_app) -->
<!-- SECTION ID: mobile_specific -->
<!-- 
Define mobile-specific requirements:
  - Target platforms (iOS, Android) and minimum OS versions
  - App store submission requirements and guidelines
  - Offline capability and data sync strategy
  - Push notification requirements
  - Device feature usage (camera, GPS, biometrics, contacts)
  - Deep linking and universal links
  - App size and performance budgets
  - Background processing needs
  - Crash reporting and analytics SDK
-->

---

## IoT-Specific Requirements
<!-- CONDITIONAL: Only include if project type requires it -->
<!-- SECTION ID: iot_specific -->
<!-- 
Define IoT-specific requirements:
  - Device protocols (MQTT, CoAP, HTTP, BLE)
  - Device provisioning and registration flow
  - Firmware update mechanism (OTA)
  - Device telemetry and data collection
  - Edge computing requirements
  - Device fleet management
  - Power and connectivity constraints
  - Device security (secure boot, certificate rotation)
-->

---

## Data Architecture
<!-- CONDITIONAL: Only include if project type requires it (data_pipeline) -->
<!-- SECTION ID: data_architecture -->
<!-- 
Define data architecture requirements:
  - Data sources and ingestion methods (batch, streaming, CDC)
  - Data storage layers (raw, processed, curated)
  - Data modeling approach (star schema, data vault, document)
  - Transformation and processing framework
  - Data quality and validation rules
  - Data lineage and cataloging
  - Retention and archival policies
  - Data access patterns and query performance targets
  - Schema evolution strategy
-->

---

## Infrastructure Design
<!-- CONDITIONAL: Only include if project type requires it (infrastructure) -->
<!-- SECTION ID: infrastructure_design -->
<!-- 
Define infrastructure requirements:
  - Cloud provider(s) and target regions
  - Infrastructure as Code approach (Terraform, Pulumi, CloudFormation)
  - Networking architecture (VPC, subnets, security groups, load balancers)
  - Compute platform (containers, serverless, VMs)
  - High availability and disaster recovery design
  - CI/CD pipeline and deployment strategy
  - Environment topology (dev, staging, production)
  - Cost estimation and budget constraints
  - Monitoring and alerting infrastructure
  - Backup and restore procedures
-->

---

<!--
=============================================================================
DOMAIN-SPECIFIC SECTIONS
=============================================================================
The following sections are populated based on the project's domain as defined
in domain-complexity.csv. Only include sections that match the project domain.
Remove all domain-specific sections that do not apply.
=============================================================================
-->

## Regulatory Compliance
<!-- CONDITIONAL: Only include if domain requires it (fintech, healthcare, govtech) -->
<!-- SECTION ID: regulatory_compliance -->
<!-- 
Define regulatory compliance requirements:
  - Applicable regulations and standards (e.g. SOX, GDPR, HIPAA, FedRAMP)
  - Compliance certification targets and timelines
  - Required compliance documentation
  - Regulatory reporting obligations
  - Compliance monitoring and audit support
  - Legal review requirements
  - Data residency and sovereignty requirements
  - Incident reporting obligations and timelines
-->

---

## Financial Security
<!-- CONDITIONAL: Only include if domain requires it (fintech) -->
<!-- SECTION ID: financial_security -->
<!-- 
Define financial security requirements:
  - PCI-DSS compliance level and scope
  - KYC/AML verification requirements
  - Fraud detection and prevention mechanisms
  - Transaction monitoring and suspicious activity reporting
  - Secure payment processing flow
  - Financial data encryption standards
  - Segregation of funds and account isolation
  - Chargeback and dispute handling
-->

---

## Audit Requirements
<!-- CONDITIONAL: Only include if domain requires it (fintech, govtech) -->
<!-- SECTION ID: audit_requirements -->
<!-- 
Define audit trail and auditing requirements:
  - Audit log scope (what actions/events must be logged)
  - Audit log retention period
  - Tamper-proof log storage mechanism
  - Audit log access controls and review process
  - Regulatory audit support requirements
  - Internal audit cadence and scope
  - Audit report generation and export
  - Chain of custody for sensitive data changes
-->

---

## Clinical Requirements
<!-- CONDITIONAL: Only include if domain requires it (healthcare) -->
<!-- SECTION ID: clinical_requirements -->
<!-- 
Define clinical and healthcare-specific requirements:
  - FDA device classification (if applicable)
  - Clinical validation and testing requirements
  - Clinical workflow integration points
  - HL7/FHIR interoperability requirements
  - Clinical data standards (ICD, SNOMED, LOINC)
  - Clinical decision support requirements
  - Patient safety considerations
  - Clinical staff training requirements
-->

---

## Data Protection
<!-- CONDITIONAL: Only include if domain requires it (healthcare) -->
<!-- SECTION ID: data_protection -->
<!-- 
Define data protection requirements:
  - PHI/PII identification and classification
  - HIPAA safeguards (administrative, physical, technical)
  - Data minimization principles
  - Consent management and patient rights
  - De-identification and anonymization standards
  - Breach notification procedures and timelines
  - Business Associate Agreements (BAA) requirements
  - Data access logging and monitoring
-->

---

## Accessibility Requirements
<!-- CONDITIONAL: Only include if domain requires it (edtech, govtech) -->
<!-- SECTION ID: accessibility_requirements -->
<!-- 
Define domain-specific accessibility requirements:
  - WCAG 2.1 AA conformance requirements (or higher)
  - Section 508 compliance (for govtech)
  - Assistive technology compatibility testing plan
  - Accessibility testing tools and methodology
  - User testing with people with disabilities
  - Accessibility statement and feedback mechanism
  - Remediation process for accessibility issues
  - Multilingual and localization accessibility
-->

---

## Content Safety
<!-- CONDITIONAL: Only include if domain requires it (edtech) -->
<!-- SECTION ID: content_safety -->
<!-- 
Define content safety and moderation requirements:
  - Content moderation policy and approach (automated, manual, hybrid)
  - Age-appropriate content guidelines
  - User-generated content review workflow
  - Prohibited content categories and detection
  - Reporting mechanism for inappropriate content
  - Content appeals and review process
  - Moderator tools and dashboards
  - Content safety metrics and monitoring
-->

---

## Data Privacy
<!-- CONDITIONAL: Only include if domain requires it (edtech) -->
<!-- SECTION ID: data_privacy -->
<!-- 
Define data privacy requirements:
  - COPPA compliance (for users under 13)
  - FERPA compliance (for educational records)
  - Parental consent collection and management
  - Student data privacy protections
  - Data collection minimization
  - Privacy policy and terms of service requirements
  - Third-party data sharing restrictions
  - Data deletion and right to be forgotten procedures
-->

---

## Payment Security
<!-- CONDITIONAL: Only include if domain requires it (ecommerce) -->
<!-- SECTION ID: payment_security -->
<!-- 
Define payment security requirements:
  - PCI-DSS compliance scope and SAQ type
  - Payment gateway integration (Stripe, PayPal, Adyen, etc.)
  - Tokenization strategy for card data
  - 3D Secure / SCA implementation
  - Refund and chargeback processing
  - Payment method support (cards, wallets, BNPL, ACH)
  - Payment data storage and handling policies
  - Fraud scoring and transaction risk assessment
-->

---

## Inventory Design
<!-- CONDITIONAL: Only include if domain requires it (ecommerce) -->
<!-- SECTION ID: inventory_design -->
<!-- 
Define inventory management requirements:
  - Inventory tracking model (SKU, variant, lot, serial)
  - Stock level management (real-time, periodic)
  - Multi-warehouse and multi-location support
  - Inventory reservation and allocation rules
  - Low stock alerts and reorder triggers
  - Inventory sync with external systems (ERP, 3PL)
  - Returns and restocking workflow
  - Inventory reporting and analytics
-->

---

## Tax Compliance
<!-- CONDITIONAL: Only include if domain requires it (ecommerce) -->
<!-- SECTION ID: tax_compliance -->
<!-- 
Define tax compliance requirements:
  - Tax calculation engine (Avalara, TaxJar, custom)
  - Sales tax nexus and jurisdiction handling
  - Tax exemption certificate management
  - VAT/GST handling for international sales
  - Tax reporting and filing integration
  - Invoice and receipt tax line item display
  - Tax rate updates and maintenance
  - Marketplace facilitator tax obligations
-->

---

## Security Controls
<!-- CONDITIONAL: Only include if domain requires it (govtech) -->
<!-- SECTION ID: security_controls -->
<!-- 
Define government-grade security control requirements:
  - NIST 800-53 control family coverage
  - FedRAMP / StateRAMP authorization level (Low, Moderate, High)
  - FIPS 140-2/140-3 cryptographic module requirements
  - Continuous monitoring (ConMon) requirements
  - Security assessment and authorization (SA&A) process
  - Incident response plan requirements
  - Supply chain risk management (SCRM)
  - Personnel security and access control
  - Physical security requirements (if applicable)
  - Data sovereignty and authorized data centers
-->

---

## Scope

### MVP -- Phase 1 (Must Have)
<!-- The minimum set of capabilities needed for the product to be useful. Ruthlessly prioritize. -->

- [ ] 

### Growth -- Phase 2 (Should Have)
<!-- Capabilities that enhance the product but are not required for initial launch. -->

- [ ] 

### Vision -- Phase 3 (Could Have)
<!-- Aspirational capabilities for the long-term roadmap. -->

- [ ] 

---

## NEEDS CLARIFICATION

<!-- 
Items that remain unresolved and require stakeholder input before proceeding.
Each item should note:
  - What the question is
  - Why it matters (what is blocked)
  - Suggested options if any
  - Who needs to answer
-->

| # | Question | Impact | Options | Owner |
|---|----------|--------|---------|-------|
|   |          |        |         |       |
