#!/usr/bin/env python3.13
"""Namechk-style brand audit: domains + packages + social media in parallel."""
import urllib.request, urllib.error, json, socket, ssl, sys
from concurrent.futures import ThreadPoolExecutor, as_completed

NAMES = ["synapsedb", "syndb", "synx", "synapse", "brainpack", "recallx", "memx", "neurodb"]
TLDS = [".com", ".io", ".dev", ".app", ".ai", ".org", ".net", ".so", ".xyz", ".co"]

UA = {"User-Agent": "Mozilla/5.0 (namechk-bot)"}
TIMEOUT = 4

def http_status(url):
    try:
        req = urllib.request.Request(url, headers=UA, method="HEAD")
        ctx = ssl.create_default_context(); ctx.check_hostname=False; ctx.verify_mode=ssl.CERT_NONE
        with urllib.request.urlopen(req, timeout=TIMEOUT, context=ctx) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code
    except Exception:
        return None

def dns_taken(domain):
    try:
        socket.gethostbyname(domain)
        return True
    except socket.gaierror:
        return False
    except Exception:
        return None

def check_domain(name, tld):
    d = name + tld
    return d, dns_taken(d)

def check_pypi(name):
    s = http_status(f"https://pypi.org/pypi/{name}/json")
    return s == 200  # taken

def check_npm(name):
    s = http_status(f"https://registry.npmjs.org/{name}")
    return s == 200

def check_crates(name):
    try:
        req = urllib.request.Request(f"https://crates.io/api/v1/crates/{name}", headers=UA)
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            d = json.loads(r.read())
            return "crate" in d
    except Exception:
        return False

def check_gh_org(name):
    s = http_status(f"https://api.github.com/orgs/{name}")
    return s == 200

def check_gh_user(name):
    s = http_status(f"https://api.github.com/users/{name}")
    return s == 200

def check_x(name):
    s = http_status(f"https://x.com/{name}")
    return s == 200  # 404 = free

def check_reddit(name):
    s = http_status(f"https://www.reddit.com/r/{name}/about.json")
    return s == 200

def check_youtube(name):
    s = http_status(f"https://www.youtube.com/@{name}")
    return s == 200

def check_mastodon(name):
    s = http_status(f"https://mastodon.social/@{name}")
    return s == 200

def check_bluesky(name):
    s = http_status(f"https://bsky.app/profile/{name}.bsky.social")
    return s == 200

def check_dockerhub(name):
    s = http_status(f"https://hub.docker.com/v2/repositories/{name}/")
    return s == 200

# Parallel run all checks
results = {}  # name -> {category: status}

tasks = []
with ThreadPoolExecutor(max_workers=40) as ex:
    for name in NAMES:
        results[name] = {}
        # Domains
        for tld in TLDS:
            tasks.append((ex.submit(check_domain, name, tld), name, f"dom:{tld}"))
        # Packages
        tasks.append((ex.submit(check_pypi, name), name, "pypi"))
        tasks.append((ex.submit(check_npm, name), name, "npm"))
        tasks.append((ex.submit(check_crates, name), name, "crates"))
        tasks.append((ex.submit(check_dockerhub, name), name, "docker"))
        # Social
        tasks.append((ex.submit(check_gh_org, name), name, "gh-org"))
        tasks.append((ex.submit(check_gh_user, name), name, "gh-user"))
        tasks.append((ex.submit(check_x, name), name, "x"))
        tasks.append((ex.submit(check_reddit, name), name, "reddit"))
        tasks.append((ex.submit(check_youtube, name), name, "youtube"))
        tasks.append((ex.submit(check_mastodon, name), name, "mastodon"))
        tasks.append((ex.submit(check_bluesky, name), name, "bluesky"))

    for fut, name, key in tasks:
        try:
            v = fut.result(timeout=8)
            if isinstance(v, tuple):
                results[name][key] = v[1]  # taken bool
            else:
                results[name][key] = v
        except Exception:
            results[name][key] = None

# Output: markdown matrix
def fmt(v):
    if v is True: return "🔴"
    if v is False: return "🟢"
    return "❓"

OUT = "/tmp/brand_audit.md"
with open(OUT, "w") as f:
    f.write("# Brand Availability Audit 2026-05-06\n\n")
    f.write("🟢 = free, 🔴 = taken, ❓ = unknown\n\n")

    # Domains table
    f.write("## Domains\n\n")
    hdr = "| Name | " + " | ".join(t.lstrip(".") for t in TLDS) + " |\n"
    f.write(hdr); f.write("|" + "---|" * (len(TLDS)+1) + "\n")
    for n in NAMES:
        row = f"| **{n}** | "
        for tld in TLDS:
            row += fmt(results[n].get(f"dom:{tld}")) + " | "
        f.write(row + "\n")

    # Packages
    f.write("\n## Package Registries\n\n")
    f.write("| Name | PyPI | NPM | crates.io | Docker Hub |\n|---|---|---|---|---|\n")
    for n in NAMES:
        f.write(f"| **{n}** | {fmt(results[n].get('pypi'))} | {fmt(results[n].get('npm'))} | {fmt(results[n].get('crates'))} | {fmt(results[n].get('docker'))} |\n")

    # Social
    f.write("\n## Social Media\n\n")
    f.write("| Name | GH org | GH user | X/Twitter | Reddit | YouTube | Mastodon | Bluesky |\n|---|---|---|---|---|---|---|---|\n")
    for n in NAMES:
        f.write(f"| **{n}** | {fmt(results[n].get('gh-org'))} | {fmt(results[n].get('gh-user'))} | {fmt(results[n].get('x'))} | {fmt(results[n].get('reddit'))} | {fmt(results[n].get('youtube'))} | {fmt(results[n].get('mastodon'))} | {fmt(results[n].get('bluesky'))} |\n")

    # Summary score
    f.write("\n## Score (lower=worse, higher=more free slots)\n\n")
    scores = []
    for n in NAMES:
        free_count = sum(1 for v in results[n].values() if v is False)
        total = sum(1 for v in results[n].values() if v in (True, False))
        scores.append((n, free_count, total))
    scores.sort(key=lambda x: -x[1])
    f.write("| Name | Free / Total | Verdict |\n|---|---|---|\n")
    for n, free, total in scores:
        v = "🥇 BEST" if free >= total*0.7 else ("✅ ok" if free >= total*0.4 else "⚠️ crowded")
        f.write(f"| **{n}** | {free}/{total} | {v} |\n")

print(f"written: {OUT}")
print(f"checked {sum(len(r) for r in results.values())} endpoints")
