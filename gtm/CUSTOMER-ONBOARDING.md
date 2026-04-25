# Customer Onboarding (Pro / Enterprise)

End-to-end vom Kauf bis zum laufenden Setup. Alles offline-fähig nach Step 3.

## Step 1 — License-Key per Email

Customer erhält nach Zahlung (Stripe webhook → license-server) eine Mail mit:

```
Key:   SYN-7K3M-Q9X2-LP4N-A8FZ
Tier:  Pro (1 seat)
Issued: 2026-04-25
```

Format: `SYN-XXXX-XXXX-XXXX-XXXX` (Keygen.sh-kompatibel, 4×4 base32).

## Step 2 — Per-customer Binary download

Mail enthält S3-presigned URL (24h TTL) zum customer-spezifisch gebauten, watermarked, signierten Binary:

```bash
curl -O "https://dl.synapse.dev/<presigned>/synapse"
curl -O "https://dl.synapse.dev/<presigned>/synapse.minisig"
curl -O "https://dl.synapse.dev/<presigned>/synapse.blake3"

minisign -Vm synapse -P RWQ<public-key>
b3sum -c synapse.blake3
chmod +x synapse && mv synapse /usr/local/bin/
```

Watermark ist customer-ID, eingebettet in 3 unabhängige Stellen (Header, .rodata, runtime-string). Ein Leak ist rückverfolgbar.

## Step 3 — Activate

```bash
synapse activate --key SYN-7K3M-Q9X2-LP4N-A8FZ --offline-grace 30d
```

Was passiert:
1. Hardware-fingerprint wird gehasht (CPU-ID + MAC + machine-id, BLAKE3, salted).
2. License-server-Call (TLS 1.3, pinned cert) → JWT (Ed25519, 24h TTL).
3. JWT cached in `~/.synapse/license.jwt` (chmod 0600).
4. TOFU lock: erste hw-fp wird vom Server fest gepinnt. Re-bind nur via Support-Ticket.

Offline-mode: `--offline-grace 30d` erlaubt 30 Tage ohne Server-Refresh.

## Step 4 — First run / encrypt brain.db

```bash
synapse init --encrypt --dry-run    # zeigt was passieren würde
synapse init --encrypt              # echte Migration
```

Was passiert:
- SQLCipher-key wird aus License-JWT + hw-fp deriviert (HKDF-SHA256).
- Existing `brain.db` wird in `brain.db.encrypted` migriert; Original 7-pass-shred nach Verify.
- Recovery-recipe wird in `~/.synapse/recovery.txt` geschrieben (für License-rebind-Fall).

## Step 5 — Steady state

- JWT refresh: alle 24h automatisch im Hintergrund (silent fail → offline-grace zählt).
- Telemetry: opt-in via `synapse config telemetry on`. Aggregierte counts only (queries/day, error-rates), keine Doc-Inhalte.
- Updates: `synapse self-update` zieht neue per-customer-watermarked binary über License-Server.

## Step 6 — Revoke flow

Admin (Support oder Customer-Account):
```bash
# auf License-Server
license-server revoke --key SYN-7K3M-... --reason "seat-reduction"
```

Effekt:
- JWT-blacklist (jti) wird published.
- Beim nächsten 24h-Refresh: 401 → Client geht in `degraded` (read-only).
- Nach 30d offline-grace: full lock, brain.db bleibt verschlüsselt aber unzugreifbar bis Re-activate.

Re-activate nach Klärung: gleicher Key + neue Aktivierung resetted JWT-cache.

## Diagnostics

```bash
synapse status           # license, hw-fp, JWT-TTL, encryption-state
synapse doctor           # full health-check, exit 0/1
synapse license export   # signed audit-report für Compliance
```
