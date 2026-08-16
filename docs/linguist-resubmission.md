# Resubmitting Corros to github-linguist/linguist

PR #8130 was closed because Corros had (a) no filled-in pull-request
template, (b) no proof of in-the-wild usage, and (c) `.cor` — its extension
at the time — already belonged to another language (Corvid) plus ~14,600
unrelated files, so the usage evidence could never show Corros.

The extension is now **`.cro`** (`.cor` still runs as a legacy alias). `.cro`
is unclaimed by any language: ~350 files on GitHub, dominated by unrelated
data-file users (critic2 crystallography, AquaCrop crop data) that linguist's
assessment filters as dominant users. No programming language uses `.cro`.

## The gate (from CONTRIBUTING.md — "Language extension and filename usage requirements")

> We do not accept PRs for very new or hobby languages, and will close any
> such PRs that attempt to add them.

For a source extension like `.cro` (occurs many times per repo):

- **at least 2000 files** with the extension indexed in the last year,
  **excluding forks** (the number at the top of the search results)
- a **reasonable distribution across unique `:user/:repo` combinations** —
  dominant users (e.g. the language author) are filtered out with `-user:`
- assessment uses GitHub search; the PR must link the search results

We are nowhere near this yet: Corros is used by exactly one person. **The
only thing that moves this number is real adoption** — other people's public
repos containing `.cro` files. No code change can substitute for it.

## Tracking progress

```bash
GITHUB_TOKEN=<pat> tools/usage_proof.sh     # writes USAGE.md
```

Reports the extension-only `.cro` total, the Corros-specific total (query
`extension:cro NOT fork:true whilst` — `whilst` appears in every Corros
program and almost nowhere else), and the unique non-fork repo/owner counts,
excluding the author. Note: GitHub re-indexes asynchronously, so counts
right after a push lag behind; re-run later.

**Resubmit when** `USAGE.md` shows the Corros-specific query at ≥ 2000
files/year spread across ≥ 200 unique non-author repos.

## Required PR contents (all of these, or the PR is closed again)

From CONTRIBUTING.md "Adding a language":

1. **Entry in `languages.yml`** — the `.cro` extension, `type: programming`,
   `color: "#b7410e"` (rust-orange, matching the name's corrosion theme),
   **omit `language_id`** (maintainers assign it via `script/update-ids`).
2. **Syntax-highlighting grammar** — a TextMate grammar repo for Corros,
   under one of the licenses linguist allows, added via
   `script/add-grammar <repo>`. **Not done yet — required work.**
3. **Real-world samples** in `samples/Corros/` — *not* hello-world or
   tutorial examples (explicitly rejected). Each sample needs a stated
   license and a link to its original source. **Not done yet — required
   work:** publish a couple of real Corros programs (e.g. a `json.cro`
   parser, the recommend scanner) as standalone repos and reference them.
4. **Filled-in PR template** (paste below).
5. **Heuristics** distinguishing Corros from the other `.cro` users
   (critic2/AquaCrop) — Corros files contain `forge`, `craft`, `whilst`,
   `speak`; data files don't.

### Paste-ready PR body (fill in the live search link + samples)

```markdown
## Description
Add the Corros programming language (.cro). Corros is a from-scratch,
self-hosting, bytecode-compiled scripting language. The .cro extension was
chosen after .cor proved to belong to another language (Corvid); .cro is
currently used by no other language.

## Checklist:
- [x] **I am adding a new language.**
- [x] The extension of the new language is used in hundreds of repositories
      on GitHub.com.
- Search results for each extension:
- <live search URL from tools/usage_proof.sh — e.g.
  https://github.com/search?type=code&q=NOT+is%3Afork+path%3A*.cro+whilst>
- [x] I have included a real-world usage sample for all extensions added in
      this PR:
- Sample source(s):
- [URL to each sample repo/source]
- Sample license(s): [license of each sample]
- [x] I have included a syntax highlighting grammar: [URL to grammar repo]
- [x] I have added a color
- Hex value: `#b7410e`
- Rationale: rust-orange — Corros is named for what Rust does best:
  corrosion, and the color echoes Rust's own palette while staying distinct.
- [x] I have updated the heuristics to distinguish my language from others
      using the same extension. (Corros source contains the keywords
      `forge`, `craft`, `whilst`, and `speak`; the other .cro users —
      critic2 and AquaCrop — are data files without those keywords.)
```

## What would have made PR #8130 reviewable (post-mortem)

- The template was never filled in (the maintainer said so explicitly).
- No usage evidence was linked — and with `.cor` the evidence could never
  have existed, because the search results belonged to Corvid.
- `language_id` was included; CONTRIBUTING says to omit it.
- No samples, no grammar, no heuristics.
