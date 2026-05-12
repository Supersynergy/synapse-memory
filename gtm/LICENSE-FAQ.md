# Synapse License FAQ

Synapse Pro and Enterprise binaries ship under the **Functional Source License (FSL-1.1-Apache-2.0)** — the same license used by Sentry, Convex, Keygen.sh, and others.

This page mirrors the canonical FAQ at <https://fsl.software> and adds Synapse-specific notes. If anything here conflicts with the upstream FSL text, the upstream text wins.

---

## TL;DR

- Source is **public** on GitHub from day one. You can read it, audit it, fork it for your own internal use.
- For **2 years** after each release, you may not use that release **to compete with Synapse**.
- After 2 years, that release **automatically converts to Apache-2.0** — fully open source, no strings.
- "Compete" means: offer Synapse-as-a-service, or embed Synapse in a product that substitutes for it.
- Internal use, modification, and self-hosting are **always allowed**, including in commercial products that are not Synapse-substitutes.

---

## Q: Is this open source?

No, not on day one. FSL is **source-available**, not OSI-approved open source. After the 2-year change date, each release becomes Apache-2.0 and is then OSI-compliant open source.

We chose FSL over MIT/Apache because we sell support, hosting, and hardened binaries. Pure permissive licensing rewards AWS-style repackagers, not the people doing the work. We chose FSL over BSL/SSPL because FSL has a **fixed, automatic** open-source conversion date — no "we'll relicense if we feel like it" theatre.

## Q: Can I use Synapse in my commercial product?

**Yes**, as long as your product is not itself a memory database / retrieval layer competing with Synapse. Embedding Synapse inside an agent platform, SaaS app, internal tool, consulting deliverable, or research paper is fine.

Examples that are fine:
- Agent platform that uses Synapse as one component
- Internal RAG system at your company
- Proprietary product where Synapse stores customer memory
- Research benchmarks and academic papers

Examples that are not fine (during the 2-year window):
- Hosted "Synapse-as-a-service" offered to third parties
- A new product called "MemoryDB" that is mostly Synapse rebranded
- Reselling unmodified Synapse binaries

## Q: What about the encrypted binary and license server?

Pro and Enterprise binaries are **separately distributed**. They include obfuscation, license-server checks, and per-customer watermarking that the source-available code does not. The source repo lets you build an unencumbered binary for your own internal use; the paid binary saves you that work and adds support, signing, and update channels.

This matters: **you can audit every line of the source.** The hardening is operational, not a black box.

## Q: Can I fork it?

Yes. Run your fork internally, ship it inside your products (subject to the non-compete clause above), or wait 2 years and ship it under Apache-2.0 with no restrictions.

## Q: What is the change date?

Each release has its own change date set to 2 years after that release's tag date, embedded in the `LICENSE` file at the repo root. The change date for `v1.0.0` (tagged 2026-04-25) is **2028-04-25**, after which `v1.0.0` is Apache-2.0.

## Q: Is FSL legally tested?

It's newer than MIT/Apache but older than SSPL. Sentry adopted it 2023. Convex, Keygen.sh, and Oxide followed. No litigation precedent yet because the non-compete window is narrow and the auto-OSS conversion removes most of the long-tail risk that triggered SSPL/BSL fights.

If your legal team needs the full text and rationale, point them at <https://fsl.software> — it was drafted to be readable without a lawyer.

## Q: Why not AGPL?

AGPL kills enterprise sales. Most Fortune 500 procurement explicitly bans AGPL in code-modified-and-served contexts. FSL-with-2yr-Apache-conversion is the strictly better deal: you get future-permissive guarantee without the present-tense ban.

## Q: Why not BSL?

BSL has the same 2-year auto-conversion idea but lets the licensor pick *any* open-source license at change date — including non-OSI ones. FSL pins the change to Apache-2.0 (or MIT, depending on suffix) from day one. Less rope for licensor mischief.

## Q: I want to contribute. How does the CLA work?

Contributions are dual-licensed under FSL-1.1 and Apache-2.0 via a one-line CLA in the PR template. Your PR-day code becomes Apache-2.0 immediately for everyone — only the project's release tarballs go through the 2-year window.

## Q: Where is the actual license text?

`LICENSE` at the repo root. The text is short — 200 lines. Read it, not summaries.

---

## Synapse-specific notes

- **Free tier (cloud, rate-limited):** governed by separate ToS, not FSL.
- **Pro binary (€49/mo):** FSL-1.1-Apache-2.0 + commercial EULA covering watermarking and resale clauses. Source for the matching release is public.
- **Enterprise (€4,900/yr+):** FSL-1.1-Apache-2.0 + negotiated MSA. Includes SLAs and dedicated support that the license itself does not cover.
- **Trademark "Synapse":** common-law mark in DACH; not registered yet. Forks must rename.

Questions: <true@supersynergy.de>.
