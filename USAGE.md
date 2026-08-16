# Corros usage evidence for linguist

- Generated: 2026-08-16

## Headline numbers
- `.cro` files indexed (extension-only query, excludes forks): **351**
- Corros-specific files (`extension:cro NOT fork:true whilst`): **0** indexed by the API
- Unique non-fork repositories with Corros `.cro` files (author `cococopi` excluded): **0**
- Unique owners: **0**

## Search queries
- Corros-specific: https://github.com/search?type=code&q=extension%3Acro%20NOT%20fork%3Atrue%20whilst
- Extension-only:   https://github.com/search?type=code&q=extension%3Acro%20NOT%20fork%3Atrue

> **Note:** the Corros-specific query currently returns 0 — GitHub re-indexes
> repositories asynchronously, so counts right after a rename lag behind.
> Re-run this script later; the extension-only count is unaffected.

| owner | repos |
|---|---|

| repository | Corros `.cro` files | sample path |
|---|---|---|

## Notes
- The extension-only query also matches unrelated `.cro` users (critic2
  crystallography, AquaCrop crop data). Linguist's assessment filters
  dominant unrelated users and checks the remaining distribution.
- Corros' author-owned repositories are excluded above.
