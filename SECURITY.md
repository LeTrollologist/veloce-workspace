VeloceNetwork Security Policy (`SECURITY.md`)

#### 1. Our Commitment

VeloceNetwork is built on a "Security-First" philosophy. We believe that while our management core is proprietary, the protocols and networking layers must be transparent and verifiable. We are committed to a three-stage audit cycle (v0.7–v0.9) to ensure a hardened v1.0 release.

#### 2. Scope

This policy covers the following VeloceNetwork components:

* **Open Modules:** `veloce-net`, `veloce-mesh`, `veloce-ipc`, and the `veloce-sdk`.
* **Proprietary Core:** `veloce-core` (Binary analysis and behavioral auditing welcome).
* **Protocols:** The Veloce IPC framing and Noise_IK implementation.

#### 3. Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

If you discover a security flaw, please report it via the following channel:

* **Email:** trollologistog@gmail.com

**Please include the following in your report:**

1. A description of the vulnerability and its potential impact.
2. Steps to reproduce the issue (or a proof-of-concept script).
3. The version of VeloceNetwork affected.

#### 4. The "Veloce Disclosure" Timeline

We aim to acknowledge all reports within **48 hours**. We follow a coordinated disclosure timeline:

* **Remediation:** We prioritize fixes based on severity (Critical/High/Medium/Low).
* **Disclosure:** Once a fix is verified and shipped, we will publish a security advisory (similar to our v0.7.0 release notes) to inform the community.
* **Credit:** With your permission, we will credit you in our release notes and our "Security Hall of Fame."

#### 5. Safe Harbor

We encourage the community to research and audit VeloceNetwork. We will not take legal action against researchers who:

* Perform research without harming VeloceNetwork users or their data.
* Avoid DoS attacks against our infrastructure (e.g., STUN servers).
* Provide us a reasonable amount of time to fix the issue before public disclosure.

---
