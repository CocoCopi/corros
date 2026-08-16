#!/usr/bin/env bash
# usage_proof.sh — collect the evidence github-linguist/linguist requires to
# add Corros as a language.
#
# Linguist only adds languages with widespread public usage (CONTRIBUTING.md):
#   - >= 2000 files with the extension indexed in the last year (excluding
#     forks) for source extensions like `.cro` that occur many times per repo
#   - spread across unique :user/:repo combinations, with the language
#     author's own repos filtered out
#
# This script queries the GitHub code-search API for `.cro` files, counts
# unique non-fork repos and owners (excluding the author), and writes
# USAGE.md — the evidence to paste into the linguist PR.
#
# Two queries are reported:
#   1. extension-only  — the raw `.cro` file count (includes unrelated users
#      like critic2 and AquaCrop; linguist filters dominant unrelated users).
#   2. keyword-filtered — `whilst`, a keyword unique to Corros, which
#      surfaces Corros code specifically.
#
# Note: GitHub re-indexes repos asynchronously; right after a rename the
# keyword count can read 0 for a while. Run again later.
#
# Usage:
#   GITHUB_TOKEN=<pat> tools/usage_proof.sh
#   GITHUB_TOKEN=<pat> AUTHOR=cococopi tools/usage_proof.sh
#
# Requires: curl, python3.
set -euo pipefail

TOKEN="${GITHUB_TOKEN:-}"
AUTHOR="${AUTHOR:-cococopi}"
EXT="${EXT:-cro}"
OUT="${OUT:-USAGE.md}"
# `whilst` appears in every Corros program and in almost no other code.
KEYWORDS="${KEYWORDS:-whilst}"

if [ -z "$TOKEN" ]; then
  echo "usage: GITHUB_TOKEN=<personal access token> tools/usage_proof.sh" >&2
  exit 1
fi

urlenc() { python3 -c 'import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1]))' "$1"; }

KW_QUERY="extension:${EXT} NOT fork:true ${KEYWORDS}"
EXT_QUERY="extension:${EXT} NOT fork:true"
KW_URL="https://github.com/search?type=code&q=$(urlenc "$KW_QUERY")"
EXT_URL="https://github.com/search?type=code&q=$(urlenc "$EXT_QUERY")"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Querying GitHub code search: $KW_QUERY" >&2

ext_total=0
kw_total=0
: > "$TMP/items.jsonl"

for page in 1 2 3 4 5 6 7 8 9 10; do
  resp="$TMP/page$page.json"
  curl -s "https://api.github.com/search/code?q=$(urlenc "$KW_QUERY")&per_page=100&page=$page" \
    -H "Authorization: Bearer $TOKEN" > "$resp" || { echo "error: search request failed" >&2; exit 1; }

  python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$resp" 2>/dev/null || {
    echo "error: unexpected API response" >&2
    head -c 500 "$resp" >&2; echo >&2
    exit 1
  }

  if [ "$page" = "1" ]; then
    kw_total=$(python3 -c "import json;print(json.load(open('$resp')).get('total_count',0))")
  fi
  n=$(python3 -c "import json;print(len(json.load(open('$resp')).get('items',[])))")
  python3 - "$resp" <<'PY' >> "$TMP/items.jsonl"
import json, sys
d = json.load(open(sys.argv[1]))
for it in d.get('items', []):
    r = it.get('repository', {})
    print(json.dumps({
        'repo': r.get('full_name', ''),
        'fork': r.get('fork', False),
        'path': it.get('path', ''),
    }))
PY
  echo "  page $page: $n results" >&2
  [ "$n" -lt 100 ] && break
  sleep 1
done

# Extension-only total (one call) for reference.
curl -s "https://api.github.com/search/code?q=$(urlenc "$EXT_QUERY")&per_page=1" \
  -H "Authorization: Bearer $TOKEN" > "$TMP/ext.json" || true
ext_total=$(python3 -c "import json;print(json.load(open('$TMP/ext.json')).get('total_count',0))" 2>/dev/null || echo 0)

KW_QUERY="$KW_QUERY" EXT_QUERY="$EXT_QUERY" AUTHOR="$AUTHOR" EXT="$EXT" \
KW_URL="$KW_URL" EXT_URL="$EXT_URL" KW_TOTAL="$kw_total" EXT_TOTAL="$ext_total" \
python3 - "$TMP/items.jsonl" "$OUT" <<'PY'
import json, sys, os, collections, datetime

author = os.environ['AUTHOR']
ext = os.environ['EXT']
kw_query = os.environ['KW_QUERY']
ext_query = os.environ['EXT_QUERY']
kw_url = os.environ['KW_URL']
ext_url = os.environ['EXT_URL']
kw_total = int(os.environ['KW_TOTAL'])
ext_total = int(os.environ['EXT_TOTAL'])

items = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
out = sys.argv[2]

files = [it for it in items if not it['fork']]
repos = collections.OrderedDict()
for it in files:
    repos.setdefault(it['repo'], []).append(it['path'])
non_author = {r: p for r, p in repos.items() if r.split('/', 1)[0] != author}
owners = collections.Counter(r.split('/', 1)[0] for r in non_author)

lines = []
lines.append("# Corros usage evidence for linguist\n")
lines.append(f"- Generated: {datetime.date.today().isoformat()}")
lines.append("")
lines.append("## Headline numbers")
lines.append(f"- `.{ext}` files indexed (extension-only query, excludes forks): **{ext_total}**")
lines.append(f"- Corros-specific files (`{kw_query}`): **{kw_total}** indexed by the API")
lines.append(f"- Unique non-fork repositories with Corros `.{ext}` files (author `{author}` excluded): **{len(non_author)}**")
lines.append(f"- Unique owners: **{len(owners)}**")
lines.append("")
lines.append("## Search queries")
lines.append(f"- Corros-specific: {kw_url}")
lines.append(f"- Extension-only:   {ext_url}")
lines.append("")
if kw_total == 0:
    lines.append("> **Note:** the Corros-specific query currently returns 0 — GitHub re-indexes")
    lines.append("> repositories asynchronously, so counts right after a rename lag behind.")
    lines.append("> Re-run this script later; the extension-only count is unaffected.")
    lines.append("")
lines.append("| owner | repos |")
lines.append("|---|---|")
for owner, n in owners.most_common():
    lines.append(f"| {owner} | {n} |")
lines.append("")
lines.append("| repository | Corros `.cro` files | sample path |")
lines.append("|---|---|---|")
for repo, paths in sorted(non_author.items(), key=lambda kv: -len(kv[1])):
    lines.append(f"| {repo} | {len(paths)} | {paths[0]} |")
lines.append("")
lines.append("## Notes")
lines.append(f"- The extension-only query also matches unrelated `.{ext}` users (critic2")
lines.append("  crystallography, AquaCrop crop data). Linguist's assessment filters")
lines.append("  dominant unrelated users and checks the remaining distribution.")
lines.append("- Corros' author-owned repositories are excluded above.")

open(out, 'w').write('\n'.join(lines) + '\n')
print(f"wrote {out}: ext_total={ext_total}, kw_total={kw_total}, "
      f"{len(non_author)} unique non-author repos, {len(owners)} owners")
PY
