# Synapse Pricing

Letzte Aktualisierung: 2026-04-25

## Tiers

| | **Dev** | **Pro** | **Enterprise** |
|---|---|---|---|
| Preis | Free | €49/mo oder €490/yr | €4,900/yr base + €0.5/seat/mo |
| Hosting | Cloud (synapse.dev) | Self-host | Self-host / on-prem |
| Doc-Cap | 10,000 | 1,000,000 | unlimited |
| Rate-Limit | 100 q/min | none | none |
| Binary | shared cloud | encrypted, watermarked, hw-fp bound | per-customer signed, custom watermark |
| License | TOS only | Ed25519 JWT, 30d offline grace | BSL/FSL source-available (2yr OSS sunset) |
| Support | Community Discord | Email 48h | Dedicated Slack, 4h SLA |
| SSO / Audit-Log | — | — | inkl. |
| On-prem license-server | — | — | inkl. (Docker image) |
| Watermark | "Synapse Dev" | per-customer ID | custom |

## Anchor

€49/mo Pro = ein Coffee-Budget eines Devs. Vergleichbar:
- Plausible Analytics: €9–79/mo
- Linear: $8–16/seat/mo
- Sentry Team: $26–80/mo

## vs. Alternatives

| | License | Self-host? | Encryption | Support |
|---|---|---|---|---|
| **Synapse Pro** | Commercial | yes (encrypted) | SQLCipher | Email 48h |
| **Synapse Enterprise** | BSL→Apache 2yr | yes | SQLCipher + custom WM | Slack 4h SLA |
| Hindsight | MIT (free OSS) | yes (plain) | none | community |
| mem0 | Apache + opaque SaaS | partial | TBD | seat-based, opaque |
| Zep | closed SaaS | no | vendor-managed | SaaS only |

## Notes

- Pro JWT-License ist hardware-fingerprint-gebunden (TOFU). Re-bind 1× pro Jahr auf Anfrage frei.
- Enterprise BSL Klausel: nicht als Hosted-Service weiterverkaufbar, sonst voll-frei. Konvertiert nach 2 Jahren zu Apache 2.0.
- Volume: >50 Enterprise-seats = custom quote. Education / OSS-maintainer: Pro free auf Antrag.
