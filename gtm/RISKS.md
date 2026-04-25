# Synapse Commercial Risks

Top-10, geranked nach Schadens-Erwartungswert (Wahrscheinlichkeit × Impact). Ehrliche Liste.

## 1. License-server downtime kills all customer installs (HIGH)
Wenn Server down + 30d offline-grace abgelaufen: customer-Stack steht.
**Mitigation:** 30d offline grace, JWT-cache, multi-region active-active (2 VPS verschiedene Provider, anycast DNS), public status-page, "panic-mode" CLI das offline-grace via signed-out-of-band-token verlängert.

## 2. FSL license confusion vs OSS expectations (HIGH)
HN/Reddit-Kommentare interpretieren "source-available" als "OSS-Betrug". Reputational damage.
**Mitigation:** FAQ direkt im README spiegeln (Sentry-Pattern), Link zu fsl.software, deutlich machen "auto-converts to Apache 2.0 after 2 years", Pro-Tier hat klare commercial-license, kein Verstecken.

## 3. Customer reverse-engineers despite hardening (MEDIUM-HIGH)
Watermark wird gestrippt, Binary cracked, in Telegram geteilt.
**Mitigation:** 3-fach watermarking (header + .rodata + runtime), BLAKE3 self-check, regelmäßige binary-rotation pro Customer, DMCA-takedown-playbook ready, EULA-Klausel mit liquidated damages, traitor-tracing aus geleaktem Binary.

## 4. Apple notarization rejection wegen Crypto/Hardening (MEDIUM-HIGH)
SQLCipher + watermarking flaggt Gatekeeper.
**Mitigation:** Ad-hoc signing als default, Linux als Tier-1-Plattform, notarization als nice-to-have nicht critical-path, Developer-ID rotation falls eine geblacklisted wird.

## 5. EU Cyber Resilience Act / Product-Liability (MEDIUM-HIGH)
Ab 2027 voll bindend; commercial software haftet für CVE-Latenz.
**Mitigation:** SBOM (syft) bei jedem Build, automated CVE-scan via osv-scanner+grype im CI, 90-day patch-SLA in EULA festgeschrieben, security-contact veröffentlicht (security.txt).

## 6. Solo-maintainer bus-factor (MEDIUM)
Maxim einziger Maintainer → Customer-FUD bei Enterprise-Sale.
**Mitigation:** Source-escrow mit Notar (Code+Schlüssel+Build-Setup) als Klausel im Enterprise-Contract; "Synapse Foundation"-Strawman-Plan für >10 Enterprise-Kunden; öffentlicher Contributor-Onboarding-Pfad.

## 7. mem0/Letta erweitern um encrypted-at-rest (MEDIUM)
Kommerzielles Killer-Feature wird kommoditisiert.
**Mitigation:** Nicht auf Encryption als Hauptmoat setzen — Performance + per-customer watermarking + on-prem license-server sind 3 weitere Moats. Roadmap auf Audit-Features (signed-query-log, tamper-evident WAL) verschieben.

## 8. Stripe / Resend / S3 vendor-shutdown (LOW-MEDIUM)
Payment / Email / Binary-distribution broken.
**Mitigation:** Stripe primär, Mollie als Backup-Plan dokumentiert; Resend mit SMTP-fallback; S3 mit Backblaze-B2 als Mirror, License-Server kennt beide presigner.

## 9. Hardware-fingerprint false-positive lockt Legitimkunden aus (MEDIUM)
CPU-Tausch, Disk-Replace, VM-Migration → Customer steht still.
**Mitigation:** Self-service rebind 1×/Jahr automatisch, danach via Support; 30d offline-grace puffert; recovery.txt aus Step 4 ermöglicht Notlauf; klare Doku im Onboarding.

## 10. DSGVO/SCC-Drama bei eigener Cloud-Tier (LOW-MEDIUM)
Free Dev-Cloud speichert customer-docs → DPA notwendig.
**Mitigation:** Dev-Tier explizit "non-confidential data only" in TOS, Hetzner Frankfurt als Hosting (EU-only), DPA-Template ready, Pro-Tier ist self-host = saubere Antwort auf jede DPA-Frage.
